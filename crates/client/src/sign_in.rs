//! `skald sign-in`: the device flow (RFC 8628). It opens the server's verification page with the
//! code filled in, in the browser, and prints one line with its link for any other browser; its
//! person approves the code there. The session token goes to the secrets and the server and the
//! account to the local state. Signed in, the machine sets up its own device, [`crate::device`],
//! and shows a recovery key that waits: [`crate::recovery`].

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use skald_proto::AccountId;

use crate::connection::{self, Connection, printable};
use crate::device::{self, Keys, Setup};
use crate::key_packages;
use crate::page;
use crate::person::Person;
use crate::recovery;
use crate::recovery_link;
use crate::secrets::{self, Secrets};
use crate::ssh_signing;
use crate::state::{self, State};

/// The client ID of the CLI on the device flow's endpoints.
const CLIENT_ID: &str = "skald";
/// The secret that holds the session token.
pub const SESSION_TOKEN: &str = "session_token";
/// The server of the session token, kept with it so that no write between the secrets and the state
/// ever sends a token to another server.
pub const SESSION_SERVER: &str = "session_server";
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
pub struct Account {
    pub account: String,
    pub username: String,
}

/// Why a sign-in failed.
#[derive(Debug)]
pub enum Error {
    Server(connection::Error),
    State(state::Error),
    Secrets(secrets::Error),
    Device(device::Error),
    Io(String),
    /// The person denied the code.
    Denied,
    /// The server refused its person's sign-up: sign-up is closed there.
    SignUpClosed,
    /// The code expired before the person approved it.
    Expired,
    /// An answer the device flow does not define.
    Unexpected(String),
    /// Linking with the recovery key stopped, for this reason.
    Recovery(String),
    /// The person who approved the code signed in to another account than the one the device
    /// belongs to, the account with `username`; `unended` is why the server did not end that
    /// session, if it did not.
    OtherAccount {
        username: String,
        unended: Option<connection::Error>,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Server(err) => err.fmt(f),
            Error::State(err) => err.fmt(f),
            Error::Secrets(err) => err.fmt(f),
            Error::Device(err) => err.fmt(f),
            Error::Io(err) => f.write_str(err),
            Error::Denied => {
                f.write_str("the code was denied on the verification page; nothing is signed in")
            }
            Error::SignUpClosed => f.write_str(
                "sign-up is closed on the server: the account you signed in with on the \
                 verification page has no Skald account and may not create one; ask the server's \
                 operator to add it, then run skald sign-in again",
            ),
            Error::Expired => f.write_str(
                "the code expired before it was approved; run skald sign-in again for a new one",
            ),
            Error::Unexpected(answer) => write!(f, "unexpected answer from the server: {answer}"),
            Error::Recovery(why) => f.write_str(why),
            Error::OtherAccount { username, unended } => {
                write!(
                    f,
                    "the code was approved by account {}, which is not the one this device \
                     belongs to; this machine keeps no session of it and its device stays as it \
                     was. Run skald sign-in again and approve the code signed in to the device's \
                     own account",
                    printable(username)
                )?;
                match unended {
                    Some(err) => write!(f, ". The server did not end that session: {err}"),
                    None => Ok(()),
                }
            }
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

impl From<device::Error> for Error {
    fn from(err: device::Error) -> Self {
        Error::Device(err)
    }
}

impl From<secrets::Error> for Error {
    fn from(err: secrets::Error) -> Self {
        Error::Secrets(err)
    }
}

/// What `skald sign-in` is given beyond the server.
pub struct Options<'p> {
    /// Where the secrets go.
    pub storage: secrets::Storage,
    /// The device's `name` the user gives.
    pub name: Option<String>,
    /// The machine's host name, which names the device unless the user gives a name.
    pub host_name: String,
    /// Where its person answers: at the terminal, if the run is the person's there, or on its
    /// local page. Only there may it show a recovery key or link codes.
    pub person: Option<Person<'p>>,
    /// With the recovery key, the devices its person names: then, waiting for linking, the device
    /// links with the account's recovery key.
    pub recovery: Option<recovery_link::Named>,
    /// How the device signs its key if it issues its own certificate.
    pub ssh: ssh_signing::Options,
}

/// Signs the machine in to the server of `connection`, keeping the state in `dir` and the secrets
/// as `options` says, then sets up its device. A session the machine has on that server is checked
/// with it instead; a session the server refuses is dropped and a new one is signed in.
pub fn run(connection: &mut Connection, dir: &Path, options: &Options<'_>) -> Result<(), Error> {
    let unusable = |err| {
        Error::Io(format!(
            "cannot use the state directory {}: {err}",
            dir.display()
        ))
    };
    state::create_dir(dir).map_err(unusable)?;
    let (state, secrets) = open(dir, options.storage)?;
    let mut stored = secrets.load()?;

    // The keys of a device the state knows, and the name the user gives, are checked before
    // anything changes.
    let known = state.device()?;
    let keys = device::known_keys(&stored, known.as_ref())?;
    let name = match &known {
        Some(device)
            if options
                .name
                .as_ref()
                .is_some_and(|name| *name != device.name) =>
        {
            return Err(device::Error::Renamed(device.name.clone()).into());
        }
        Some(device) => device.name.clone(),
        None => device::name(options.name.as_deref(), &options.host_name)?,
    };

    let belongs = known.as_ref().and_then(|device| device.account.as_deref());
    let (token, account) = session(connection, dir, (&state, &secrets), &mut stored, belongs)?;

    // Keys the secrets hold without a device in the state were generated by a run that stopped
    // before it recorded the device: they are its keys.
    let keys = match keys {
        Some(keys) => keys,
        None => {
            let keys = Keys::generate();
            keys.store(&mut stored);
            secrets.save(&stored)?;
            keys
        }
    };
    let device = match known {
        Some(device) => {
            state.set_device_account(&account.to_string())?;
            device
        }
        None => {
            state.add_device(keys.id(), &name, &account.to_string())?;
            state::Device {
                id: keys.id(),
                name,
                account: Some(account.to_string()),
                registered: false,
                link: state::Link::New,
            }
        }
    };
    Setup {
        connection,
        state: &state,
        secrets: &secrets,
        stored: &mut stored,
        token: &token,
        account,
        device,
        keys: &keys,
        person: options.person,
        recovery: options.recovery.as_ref(),
        ssh: &options.ssh,
    }
    .run()?;
    // A device that holds its certificate keeps its KeyPackages on the server.
    let certified = state.device()?.map(|device| device.link);
    if matches!(certified, Some(state::Link::Certified { .. }))
        && let Err(err) = key_packages::keep(connection, &token, (&state, &secrets), &keys, false)
    {
        key_packages::warn(&err);
    }
    recovery::show(&state, &secrets, connection.server(), options.person)
}

/// The machine's session on the server of `connection`, with `stored`, the secrets: the one it
/// has if the server takes it, otherwise a new one by the device flow, whose page's file goes to
/// `dir`. Gives its token and account, which is `belongs`, the account of the machine's device, if
/// it has one.
fn session(
    connection: &mut Connection,
    dir: &Path,
    (state, secrets): (&State, &Secrets),
    stored: &mut secrets::Map,
    belongs: Option<&str>,
) -> Result<(String, AccountId), Error> {
    let server = connection.server().to_owned();
    let session = state.session()?;
    let token = token_for(stored, &server).cloned();
    if let (Some(session), Some(token)) = (session, token)
        && session.server == server
    {
        match connection.get_as("/session", &token) {
            Ok(body) => {
                return take(connection, state, secrets, stored, belongs, token, &body);
            }
            Err(connection::Error::Refused { status: 401, .. }) => {
                println!("The server no longer accepts this machine's session; signing in again.");
                forget(state, secrets, stored)?;
            }
            Err(err) => return Err(err.into()),
        }
    }

    let token = device_flow(connection, dir)?;
    stored.insert(SESSION_TOKEN.to_owned(), token.clone());
    stored.insert(SESSION_SERVER.to_owned(), server.clone());
    secrets.save(stored)?;
    state.start_session(&server)?;
    let body = connection.get_as("/session", &token)?;
    take(connection, state, secrets, stored, belongs, token, &body)
}

/// Takes the session of `token`, whose account `body`, `GET /session`'s answer, gives: records
/// its account and says it is signed in, if it is `belongs`, the account of the machine's device,
/// or the device has none. Otherwise ends the session on the server and forgets it.
fn take(
    connection: &mut Connection,
    state: &State,
    secrets: &Secrets,
    stored: &mut secrets::Map,
    belongs: Option<&str>,
    token: String,
    body: &[u8],
) -> Result<(String, AccountId), Error> {
    let account: Account = serde_json::from_slice(body)
        .map_err(|_| Error::Unexpected("a session without its account".to_owned()))?;
    let id = account_id(&account)?;
    if let Some(belongs) = belongs {
        let belongs: AccountId = belongs
            .parse()
            .map_err(|_| state::Error::Malformed(format!("the device's account {belongs:?}")))?;
        if belongs != id {
            let unended = connection.delete_as("/session", &token).err();
            forget(state, secrets, stored)?;
            return Err(Error::OtherAccount {
                username: account.username,
                unended,
            });
        }
    }
    state.set_account(&account.account, &account.username)?;
    println!(
        "Signed in to {} as {}.",
        connection.server(),
        printable(&account.username)
    );
    Ok((token, id))
}

/// The local state and the secrets, in `storage`, of the machine whose state directory `dir`
/// exists.
pub fn open(dir: &Path, storage: secrets::Storage) -> Result<(State, Secrets), Error> {
    // The key storage's entry is named by the directory's absolute path.
    let dir = &dir.canonicalize().map_err(|err| {
        Error::Io(format!(
            "cannot use the state directory {}: {err}",
            dir.display()
        ))
    })?;
    let state = State::open(dir)?;
    Ok((state, Secrets::new(dir, storage)))
}

/// The session token `stored`, the secrets, keep for `server`: a token goes only to the server it
/// came from.
pub fn token_for<'s>(stored: &'s secrets::Map, server: &str) -> Option<&'s String> {
    let kept_for = stored.get(SESSION_SERVER).map(String::as_str);
    stored
        .get(SESSION_TOKEN)
        .filter(|_| kept_for == Some(server))
}

/// Forgets the machine's session: deletes its token from `stored`, the secrets, and the session
/// from `state`. The device stays.
pub fn forget(state: &State, secrets: &Secrets, stored: &mut secrets::Map) -> Result<(), Error> {
    stored.remove(SESSION_TOKEN);
    stored.remove(SESSION_SERVER);
    secrets.save(stored)?;
    state.end_session()?;
    Ok(())
}

/// The ID of the session's `account`.
fn account_id(account: &Account) -> Result<AccountId, Error> {
    account
        .account
        .parse()
        .map_err(|_| Error::Unexpected("a session of an account without a valid ID".to_owned()))
}

/// Asks for a code, opens its verification page in the browser through a file in `dir`, prints
/// one line with its link and polls until its person approves it; gives the session token. The
/// file is gone once it returns.
fn device_flow(connection: &mut Connection, dir: &Path) -> Result<String, Error> {
    let body = connection.post_form("/auth/device", &[("client_id", CLIENT_ID)])?;
    let grant: Authorization = serde_json::from_slice(&body)
        .map_err(|_| Error::Unexpected("a device authorization without its codes".to_owned()))?;
    let deadline = deadline(grant.expires_in)?;
    let link = grant
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&grant.verification_uri);
    // Kept until the device flow ends, so the browser can read it.
    let opened = page::open_url(dir, link);
    println!("{}", fallback(&grant, opened.is_some()));

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

/// When codes that live `expires_in` seconds from now expire.
fn deadline(expires_in: u64) -> Result<Instant, Error> {
    Instant::now()
        .checked_add(Duration::from_secs(expires_in))
        .ok_or_else(|| Error::Unexpected("a code lifetime too long".to_owned()))
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
            "slow_down" => interval
                .checked_add(SLOW_DOWN)
                .map(Poll::Wait)
                .ok_or_else(|| Error::Unexpected("a polling interval too long".to_owned())),
            "access_denied" => Err(Error::Denied),
            "sign_up_closed" => Err(Error::SignUpClosed),
            "expired_token" => Err(Error::Expired),
            _ => Err(connection::Error::OAuth { error, description }.into()),
        },
        Err(err) => Err(err.into()),
    }
}

/// The one line a sign-in prints with `grant`'s verification page, for a browser that did not
/// open; `opened` says whether one did.
fn fallback(grant: &Authorization, opened: bool) -> String {
    match &grant.verification_uri_complete {
        Some(complete) if opened => format!(
            "Sign in in the browser that opened, or at {}",
            printable(complete)
        ),
        Some(complete) => format!("Sign in at {}", printable(complete)),
        None => format!(
            "Sign in at {} with the code {}",
            printable(&grant.verification_uri),
            printable(&grant.user_code)
        ),
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
    fn overflowing_lifetime_or_interval_is_an_unexpected_answer() {
        let error = deadline(u64::MAX).expect_err("too long");
        assert!(matches!(error, Error::Unexpected(_)), "{error:?}");
        assert!(deadline(60).is_ok_and(|at| at > Instant::now()));
        let error = poll(oauth("slow_down"), Duration::MAX).expect_err("too long");
        assert!(matches!(error, Error::Unexpected(_)), "{error:?}");
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
        assert!(matches!(
            poll(oauth("sign_up_closed"), SECOND),
            Err(Error::SignUpClosed)
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
    fn other_account_names_who_approved_and_whether_the_session_ended() {
        let error = |unended| {
            Error::OtherAccount {
                username: "bob\u{1b}".to_owned(),
                unended,
            }
            .to_string()
        };
        let ended = error(None);
        assert!(ended.contains("account bob\\u{1b}, which"), "{ended}");
        assert!(!ended.contains("GitHub"), "{ended}");
        assert!(ended.contains("skald sign-in"), "{ended}");
        assert!(!ended.contains("did not end"), "{ended}");
        let unended = error(Some(connection::Error::Timeout(SECOND)));
        assert!(unended.starts_with(&ended), "{unended}");
        assert!(
            unended.ends_with(&format!(
                ". The server did not end that session: {}",
                connection::Error::Timeout(SECOND)
            )),
            "{unended}"
        );
    }

    #[test]
    fn fallback_is_one_line_with_the_link() {
        let grant = |complete: Option<&str>| Authorization {
            device_code: "dc".to_owned(),
            user_code: "BCDF-GHJK".to_owned(),
            verification_uri: "https://a.test/device".to_owned(),
            verification_uri_complete: complete.map(str::to_owned),
            expires_in: 900,
            interval: None,
        };
        let complete = "https://a.test/device?user_code=BCDF-GHJK";
        let opened = fallback(&grant(Some(complete)), true);
        assert!(opened.ends_with(&format!(" at {complete}")), "{opened}");
        assert!(opened.contains("browser that opened"), "{opened}");
        assert_eq!(
            fallback(&grant(Some(complete)), false),
            format!("Sign in at {complete}")
        );
        assert_eq!(
            fallback(&grant(None), false),
            "Sign in at https://a.test/device with the code BCDF-GHJK"
        );
        let escaped = fallback(&grant(Some("https://a.test/\u{1b}[2J")), false);
        assert!(!escaped.contains('\u{1b}'), "{escaped}");
        for line in [opened, escaped] {
            assert!(!line.contains('\n'), "{line}");
        }
    }
}
