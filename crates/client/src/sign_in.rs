//! `skald sign-in`: the device flow (RFC 8628) on a machine without a browser. Its person approves
//! the code on the server's verification page from any browser; the session token goes to the
//! secrets and the server and the account to the local state.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::connection::{self, Connection, printable};
use crate::secrets::{self, Secrets};
use crate::state::{self, State};

/// The client ID of the CLI on the device flow's endpoints.
const CLIENT_ID: &str = "skald";
/// The secret that holds the session token.
const SESSION_TOKEN: &str = "session_token";
/// The server of the session token, kept with it so that no write between the secrets and the state
/// ever sends a token to another server.
const SESSION_SERVER: &str = "session_server";
/// How much longer polls wait after `slow_down` (RFC 8628, §3.5).
const SLOW_DOWN: Duration = Duration::from_secs(5);

/// Device authorization's answer (RFC 8628, §3.2).
#[derive(Deserialize)]
struct Authorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
}

/// The session's account, from `GET /session`.
#[derive(Deserialize)]
struct Account {
    account: String,
    github_login: String,
}

/// Why a sign-in failed.
#[derive(Debug)]
pub enum Error {
    Server(connection::Error),
    State(state::Error),
    Secrets(secrets::Error),
    Io(String),
    /// The person denied the code.
    Denied,
    /// The code expired before the person approved it.
    Expired,
    /// An answer the device flow does not define.
    Unexpected(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Server(err) => err.fmt(f),
            Error::State(err) => err.fmt(f),
            Error::Secrets(err) => err.fmt(f),
            Error::Io(err) => f.write_str(err),
            Error::Denied => {
                f.write_str("the code was denied on the verification page; nothing is signed in")
            }
            Error::Expired => f.write_str(
                "the code expired before it was approved; run skald sign-in again for a new one",
            ),
            Error::Unexpected(answer) => write!(f, "unexpected answer from the server: {answer}"),
        }
    }
}

impl From<connection::Error> for Error {
    fn from(err: connection::Error) -> Self {
        Error::Server(err)
    }
}

impl From<state::Error> for Error {
    fn from(err: state::Error) -> Self {
        Error::State(err)
    }
}

impl From<secrets::Error> for Error {
    fn from(err: secrets::Error) -> Self {
        Error::Secrets(err)
    }
}

/// Signs the machine in to the server of `connection`, keeping the state in `dir` and the secrets
/// in `storage`. A session the machine has on that server is checked with it instead; a session the
/// server refuses is dropped and a new one is signed in.
pub fn run(
    connection: &mut Connection,
    dir: &Path,
    storage: secrets::Storage,
) -> Result<(), Error> {
    let server = connection.server().to_owned();
    let unusable = |err| {
        Error::Io(format!(
            "cannot use the state directory {}: {err}",
            dir.display()
        ))
    };
    state::create_dir(dir).map_err(unusable)?;
    // The key storage's entry is named by the directory's absolute path.
    let dir = &dir.canonicalize().map_err(unusable)?;
    let state = State::open(dir)?;
    let secrets = Secrets::new(dir, storage);
    let mut stored = secrets.load()?;

    let session = state.session()?;
    let token = stored.get(SESSION_TOKEN).cloned();
    if let (Some(session), Some(token)) = (session, token)
        && session.server == server
        && stored.get(SESSION_SERVER) == Some(&server)
    {
        match connection.get_as("/session", &token) {
            Ok(body) => {
                let account = record_account(&state, &body)?;
                println!(
                    "Signed in to {server} as {}.",
                    printable(&account.github_login)
                );
                return Ok(());
            }
            Err(connection::Error::Refused { status: 401, .. }) => {
                println!("The server no longer accepts this machine's session; signing in again.");
                stored.remove(SESSION_TOKEN);
                stored.remove(SESSION_SERVER);
                secrets.save(&stored)?;
                state.end_session()?;
            }
            Err(err) => return Err(err.into()),
        }
    }

    let token = device_flow(connection)?;
    stored.insert(SESSION_TOKEN.to_owned(), token.clone());
    stored.insert(SESSION_SERVER.to_owned(), server.clone());
    secrets.save(&stored)?;
    state.start_session(&server)?;
    let body = connection.get_as("/session", &token)?;
    let account = record_account(&state, &body)?;
    println!(
        "Signed in to {server} as {}.",
        printable(&account.github_login)
    );
    Ok(())
}

/// Reads the session's account from `body`, `GET /session`'s answer, and records it.
fn record_account(state: &State, body: &[u8]) -> Result<Account, Error> {
    let account: Account = serde_json::from_slice(body)
        .map_err(|_| Error::Unexpected("a session without its account".to_owned()))?;
    state.set_account(&account.account, &account.github_login)?;
    Ok(account)
}

/// Asks for a code, shows it and polls until its person approves it; gives the session token.
fn device_flow(connection: &mut Connection) -> Result<String, Error> {
    let body = connection.post_form("/auth/device", &[("client_id", CLIENT_ID)])?;
    let grant: Authorization = serde_json::from_slice(&body)
        .map_err(|_| Error::Unexpected("a device authorization without its codes".to_owned()))?;
    let deadline = Instant::now() + Duration::from_secs(grant.expires_in);
    println!(
        "To sign this machine in, open {} in a browser on any device and enter the code {}",
        printable(&grant.verification_uri),
        printable(&grant.user_code)
    );
    if let Some(complete) = &grant.verification_uri_complete {
        println!(
            "or open {}, which has the code filled in.",
            printable(complete)
        );
    }
    println!(
        "The code expires in {}. Waiting for its approval...",
        lifetime(grant.expires_in)
    );

    // RFC 8628, §3.5: 5 seconds when the server gives no interval.
    let mut interval = Duration::from_secs(grant.interval.unwrap_or(5).max(1));
    loop {
        thread::sleep(interval);
        if Instant::now() >= deadline {
            return Err(Error::Expired);
        }
        let answer = connection.post_form(
            "/auth/token",
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &grant.device_code),
                ("client_id", CLIENT_ID),
            ],
        );
        match poll(answer, interval)? {
            Poll::Token(token) => return Ok(token),
            Poll::Wait(next) => interval = next,
        }
    }
}

/// What a poll of the token endpoint tells the client to do.
#[derive(Debug, PartialEq)]
enum Poll {
    /// The session token.
    Token(String),
    /// Poll again after this long.
    Wait(Duration),
}

/// Acts on the answer to a poll sent `interval` after the previous one (RFC 8628, §3.5).
fn poll(answer: Result<Vec<u8>, connection::Error>, interval: Duration) -> Result<Poll, Error> {
    match answer {
        Ok(body) => {
            let token: Token = serde_json::from_slice(&body)
                .map_err(|_| Error::Unexpected("a token answer without its token".to_owned()))?;
            Ok(Poll::Token(token.access_token))
        }
        Err(connection::Error::OAuth { error, description }) => match error.as_str() {
            "authorization_pending" => Ok(Poll::Wait(interval)),
            "slow_down" => Ok(Poll::Wait(interval + SLOW_DOWN)),
            "access_denied" => Err(Error::Denied),
            "expired_token" => Err(Error::Expired),
            _ => Err(connection::Error::OAuth { error, description }.into()),
        },
        Err(err) => Err(err.into()),
    }
}

/// `seconds` for people: in minutes from two minutes on.
fn lifetime(seconds: u64) -> String {
    match seconds {
        1 => "1 second".to_owned(),
        0..120 => format!("{seconds} seconds"),
        _ => format!("{} minutes", seconds / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    fn oauth(error: &str) -> Result<Vec<u8>, connection::Error> {
        Err(connection::Error::OAuth {
            error: error.to_owned(),
            description: String::new(),
        })
    }

    #[test]
    fn pending_waits_the_interval_and_slow_down_five_seconds_more() {
        assert_eq!(
            poll(oauth("authorization_pending"), SECOND).unwrap(),
            Poll::Wait(SECOND)
        );
        assert_eq!(
            poll(oauth("slow_down"), SECOND).unwrap(),
            Poll::Wait(Duration::from_secs(6))
        );
    }

    #[test]
    fn approval_gives_the_token() {
        let body = br#"{"access_token":"abc","token_type":"Bearer"}"#.to_vec();
        assert_eq!(
            poll(Ok(body), SECOND).unwrap(),
            Poll::Token("abc".to_owned())
        );
        let error = poll(Ok(b"{}".to_vec()), SECOND).expect_err("no token");
        assert!(matches!(error, Error::Unexpected(_)), "{error:?}");
    }

    #[test]
    fn denied_expired_and_other_errors_stop() {
        assert!(matches!(
            poll(oauth("access_denied"), SECOND),
            Err(Error::Denied)
        ));
        assert!(matches!(
            poll(oauth("expired_token"), SECOND),
            Err(Error::Expired)
        ));
        let error = poll(oauth("invalid_grant"), SECOND).expect_err("stops");
        assert!(
            matches!(&error, Error::Server(connection::Error::OAuth { error, .. }) if error == "invalid_grant"),
            "{error:?}"
        );
        let error = poll(Err(connection::Error::Timeout(SECOND)), SECOND).expect_err("stops");
        assert!(
            matches!(error, Error::Server(connection::Error::Timeout(_))),
            "{error:?}"
        );
    }

    #[test]
    fn lifetime_reads_in_seconds_then_minutes() {
        assert_eq!(lifetime(1), "1 second");
        assert_eq!(lifetime(3), "3 seconds");
        assert_eq!(lifetime(119), "119 seconds");
        assert_eq!(lifetime(120), "2 minutes");
        assert_eq!(lifetime(900), "15 minutes");
    }
}
