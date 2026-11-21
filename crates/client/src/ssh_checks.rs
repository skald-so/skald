//! Checking certificates' SSH signatures against GitHub (`docs/protocol/membership.md`, "Device
//! certificate"): the client takes the login and the public SSH keys, authentication and signing
//! keys, of the GitHub user a certificate names from GitHub's API, unauthenticated, as it reaches
//! its server (`docs/dev/architecture.md`, "Network"). It checks each certificate once and keeps
//! the result by the API's address, so a run with another address, as an agent could make, changes
//! nothing runs with GitHub's own show. One it cannot check waits for a later run; GitHub's
//! failures fail no command.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use skald_crypto::{SshCheck, check_ssh_signature};
use skald_proto::{Cert, DeviceId, DeviceKind};

use crate::connection;
use crate::devices::text;
use crate::state::State;

/// The largest answer of GitHub's API the client reads.
const LIMIT: u64 = 1 << 20;

/// The keys the client takes of each of a user's lists: GitHub's largest page.
const PER_PAGE: usize = 100;

/// The longest GitHub login.
const LOGIN_MAX: usize = 39;

/// What a device's certificate showed of its SSH signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    /// It passed: the login GitHub gave its GitHub user.
    Passed(String),
    /// A key change.
    Failed,
    /// It waits for a later run.
    NotChecked,
}

/// What the certificates of a run showed, by device, of those that carry an SSH signature.
pub type Results = HashMap<DeviceId, Seen>;

/// What `results` shows of `cert` at the end of its device's line: nothing for a certificate
/// without an SSH signature.
pub fn shown(results: &Results, cert: &Cert) -> String {
    match results.get(&cert.body().device) {
        Some(Seen::Passed(login)) => format!("  GitHub {login}"),
        Some(Seen::Failed) => "  GitHub key change".to_owned(),
        Some(Seen::NotChecked) => "  GitHub not checked".to_owned(),
        None => String::new(),
    }
}

/// Checks the SSH signature of each of `certs`, those of `client` devices that carry one, against
/// GitHub's API at `api`, each request failing after `timeout`, unless `state` holds its result;
/// keeps each result. Gives the results, and the warning of each key change found now.
pub fn check(
    state: &State,
    api: &str,
    timeout: Duration,
    certs: &[&Cert],
) -> Result<(Results, Vec<String>), String> {
    let api = api.trim_end_matches('/');
    let mut github = Github::new(api, timeout);
    let mut results = Results::new();
    let mut warnings = Vec::new();
    for cert in certs {
        let body = cert.body();
        let (Some(_), Some(user), DeviceKind::Client) = (&body.ssh, body.github, body.kind) else {
            continue;
        };
        let hash: [u8; 32] = Sha256::digest(cert.body_bytes()).into();
        if let Some(kept) = state.ssh_check(api, &hash).map_err(text)? {
            results.insert(body.device, kept.map_or(Seen::Failed, Seen::Passed));
            continue;
        }
        let seen = match github.keys(user) {
            Some(Listed::User { login, keys }) => {
                let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
                match check_ssh_signature(body, &keys) {
                    SshCheck::Passed { .. } => Seen::Passed(login.clone()),
                    SshCheck::Failed => Seen::Failed,
                    _ => Seen::NotChecked,
                }
            }
            Some(Listed::NoUser) => Seen::Failed,
            None => Seen::NotChecked,
        };
        let login = match &seen {
            Seen::Passed(login) => Some(login.as_str()),
            Seen::Failed => None,
            Seen::NotChecked => {
                results.insert(body.device, seen);
                continue;
            }
        };
        state.keep_ssh_check(api, &hash, login).map_err(text)?;
        if seen == Seen::Failed {
            warnings.push(format!(
                "the SSH signature of device {} verifies with no SSH key GitHub lists for the \
                 GitHub user {user} its certificate names: its key may have been swapped",
                body.device
            ));
        }
        results.insert(body.device, seen);
    }
    Ok((results, warnings))
}

/// What GitHub's API lists for a user ID.
enum Listed {
    /// The user's login and SSH keys, as OpenSSH public key lines.
    User { login: String, keys: Vec<String> },
    /// No such user: no key of it verifies anything.
    NoUser,
}

/// GitHub's API, as a run reaches it: once GitHub cannot be reached, or answers otherwise than with
/// a list, the run asks it nothing more.
struct Github<'a> {
    api: &'a str,
    agent: Option<ureq::Agent>,
    /// The users the run listed.
    listed: HashMap<u64, Listed>,
}

impl<'a> Github<'a> {
    fn new(api: &'a str, timeout: Duration) -> Self {
        Github {
            api,
            agent: connection::agent_to(api, timeout).ok(),
            listed: HashMap::new(),
        }
    }

    /// What GitHub lists for the user `user`; `None` if it gave nothing this run.
    fn keys(&mut self, user: u64) -> Option<&Listed> {
        if !self.listed.contains_key(&user) {
            let agent = self.agent.as_ref()?;
            match list(agent, self.api, user) {
                Some(listed) => {
                    self.listed.insert(user, listed);
                }
                None => {
                    self.agent = None;
                    return None;
                }
            }
        }
        self.listed.get(&user)
    }
}

/// A GitHub user as its API gives it.
#[derive(Deserialize)]
struct User {
    login: String,
}

/// An SSH key as GitHub's API lists it.
#[derive(Deserialize)]
struct Key {
    key: String,
}

/// What GitHub's API at `api` lists for the user `user`: its login, from `GET /user/{id}`, and its
/// SSH keys, from `GET /users/{login}/keys` and `GET /users/{login}/ssh_signing_keys`. `None` when
/// it cannot be reached or gives anything else, its rate limit or a malformed login included.
fn list(agent: &ureq::Agent, api: &str, user: u64) -> Option<Listed> {
    let found: User = match get(agent, &format!("{api}/user/{user}"))? {
        Answer::Found(body) => serde_json::from_slice(&body).ok()?,
        Answer::NotFound => return Some(Listed::NoUser),
    };
    let login = found.login;
    let valid = (1..=LOGIN_MAX).contains(&login.len())
        && login
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !valid {
        return None;
    }
    let mut keys = Vec::new();
    for list in ["keys", "ssh_signing_keys"] {
        let url = format!("{api}/users/{login}/{list}?per_page={PER_PAGE}");
        let Answer::Found(body) = get(agent, &url)? else {
            return None;
        };
        let listed: Vec<Key> = serde_json::from_slice(&body).ok()?;
        keys.extend(listed.into_iter().map(|key| key.key));
    }
    Some(Listed::User { login, keys })
}

/// An answer of GitHub's API that gives something.
enum Answer {
    /// `200`, with its body.
    Found(Vec<u8>),
    /// `404`.
    NotFound,
}

/// The answer to `GET <url>` of GitHub's API; `None` if none came or it is another.
fn get(agent: &ureq::Agent, url: &str) -> Option<Answer> {
    let mut response = agent
        .get(url)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .header("user-agent", concat!("skald/", env!("CARGO_PKG_VERSION")))
        .call()
        .ok()?;
    match response.status().as_u16() {
        200 => (response.body_mut().with_config().limit(LIMIT))
            .read_to_vec()
            .ok()
            .map(Answer::Found),
        404 => Some(Answer::NotFound),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use skald_crypto::{SigningKey, issue};
    use skald_github_stand_in::{App, Fault, GitHub};
    use skald_proto::{AccountId, CertBody, Unknown};

    use super::*;
    use crate::test_dir::TestDir;

    const TIMEOUT: Duration = Duration::from_secs(5);

    /// A certificate of a device of `kind`, carrying an SSH signature, not one that verifies, by
    /// GitHub's user `github` if given.
    fn cert(kind: DeviceKind, github: Option<u64>) -> Cert {
        let key = SigningKey::generate();
        let signing = key.public_key();
        let body = CertBody {
            account: AccountId::generate(),
            device: DeviceId::from_signing_key(&signing),
            kind,
            signing,
            hpke: [7; 32],
            issuer: None,
            issued: 1,
            ssh: github.map(|_| vec![1, 2, 3]),
            github,
            name: "laptop".to_owned(),
            unknown: Unknown::default(),
        };
        issue(&body, &SigningKey::generate(), &key).expect("a certificate")
    }

    fn hash(cert: &Cert) -> [u8; 32] {
        Sha256::digest(cert.body_bytes()).into()
    }

    /// A GitHub stand-in that gives the user `5` with a login GitHub never gives.
    fn github() -> GitHub {
        let github = GitHub::start(App {
            client_id: "Ov23li0standin0unit".to_owned(),
            client_secret: "standin-secret-unit".to_owned(),
            callback: None,
        });
        github.put_user(skald_github_stand_in::User::new(5, "ada keys"));
        github
    }

    #[test]
    fn a_line_shows_the_login_a_key_change_or_not_checked_and_nothing_without_a_signature() {
        let certs = [
            cert(DeviceKind::Client, Some(1)),
            cert(DeviceKind::Client, Some(1)),
            cert(DeviceKind::Client, Some(1)),
            cert(DeviceKind::Client, None),
        ];
        let id = |index: usize| certs[index].body().device;
        let results = Results::from([
            (id(0), Seen::Passed("ada".to_owned())),
            (id(1), Seen::Failed),
            (id(2), Seen::NotChecked),
        ]);
        let lines: Vec<String> = certs.iter().map(|cert| shown(&results, cert)).collect();
        assert_eq!(
            lines,
            [
                "  GitHub ada",
                "  GitHub key change",
                "  GitHub not checked",
                ""
            ]
        );
    }

    /// GitHub at an address nothing listens on, or one the client does not connect to: nothing is
    /// checked, nothing is kept and nothing fails; certificates without a signature, or of other
    /// kinds, ask nothing.
    #[test]
    fn unreachable_github_leaves_signed_certificates_not_checked_and_keeps_nothing() {
        let dir = TestDir::new("ssh-checks-unreachable");
        let state = State::open(&dir).unwrap();
        let signed = cert(DeviceKind::Client, Some(5));
        let others = [
            cert(DeviceKind::Client, None),
            cert(DeviceKind::Browser, Some(5)),
        ];
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        for api in [closed.as_str(), "http://github.example", "not a url"] {
            let certs = [&signed, &others[0], &others[1]];
            let (results, warnings) = check(&state, api, TIMEOUT, &certs).unwrap();
            assert_eq!(
                results,
                Results::from([(signed.body().device, Seen::NotChecked)]),
                "{api}"
            );
            assert!(warnings.is_empty(), "{api}: {warnings:?}");
            assert_eq!(state.ssh_check(api, &hash(&signed)).unwrap(), None, "{api}");
        }
    }

    /// A user GitHub does not have lists no key: a key change, warned of once. A login GitHub never
    /// gives leaves the certificate not checked.
    #[test]
    fn a_missing_user_is_a_key_change_once_and_a_malformed_login_is_not_checked() {
        let dir = TestDir::new("ssh-checks-missing");
        let state = State::open(&dir).unwrap();
        let github = github();
        let (missing, malformed) = (
            cert(DeviceKind::Client, Some(6)),
            cert(DeviceKind::Client, Some(5)),
        );
        let expected = Results::from([
            (missing.body().device, Seen::Failed),
            (malformed.body().device, Seen::NotChecked),
        ]);
        for warned in [1, 0] {
            let certs = [&missing, &malformed];
            let (results, warnings) = check(&state, github.url(), TIMEOUT, &certs).unwrap();
            assert_eq!(results, expected);
            assert_eq!(warnings.len(), warned, "{warnings:?}");
            assert!(
                (warnings.iter()).all(|w| w.contains(&missing.body().device.to_string())),
                "{warnings:?}"
            );
        }
        assert_eq!(
            state.ssh_check(github.url(), &hash(&malformed)).unwrap(),
            None
        );
    }

    /// Once GitHub answers with an error, the run asks it nothing more.
    #[test]
    fn a_run_asks_github_nothing_after_an_error() {
        let dir = TestDir::new("ssh-checks-error");
        let state = State::open(&dir).unwrap();
        let github = github();
        github.set_faults(&[Fault::ApiStatus { status: 500 }]);
        let certs = [
            cert(DeviceKind::Client, Some(6)),
            cert(DeviceKind::Client, Some(7)),
        ];
        let (results, _) = check(&state, github.url(), TIMEOUT, &[&certs[0], &certs[1]]).unwrap();
        assert!(
            results.values().all(|seen| *seen == Seen::NotChecked),
            "{results:?}"
        );
        assert_eq!(github.requests().len(), 1, "{:?}", github.requests());
    }

    /// The results kept for one address show only for it, with or without a trailing `/`, and the
    /// first kept stays.
    #[test]
    fn kept_results_are_apart_by_the_api_s_address() {
        let dir = TestDir::new("ssh-checks-kept");
        let state = State::open(&dir).unwrap();
        let signed = cert(DeviceKind::Client, Some(5));
        let (github, other) = ("https://api.github.com", "http://127.0.0.1:9");
        state
            .keep_ssh_check(github, &hash(&signed), Some("ada"))
            .unwrap();
        state.keep_ssh_check(github, &hash(&signed), None).unwrap();
        state.keep_ssh_check(other, &hash(&signed), None).unwrap();

        let device = signed.body().device;
        for (api, seen) in [
            (github, Seen::Passed("ada".to_owned())),
            (other, Seen::Failed),
            ("http://127.0.0.1:9/", Seen::Failed),
        ] {
            let (results, warnings) = check(&state, api, TIMEOUT, &[&signed]).unwrap();
            assert_eq!(results, Results::from([(device, seen)]), "{api}");
            assert!(warnings.is_empty(), "{api}: {warnings:?}");
        }
    }
}
