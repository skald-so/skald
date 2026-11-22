//! Signing in by a link code or the browser (`docs/protocol/membership.md`, "Linking a device",
//! "Link code" and "Device key session"; `docs/dev/architecture.md`, "Device linking"). `skald
//! sign-in` on a machine without a device offers both ways on one screen, asking nothing: the
//! browser sign-in of the device flow, for the account's first device or for recovery with
//! `--with-recovery-key`, and a link code for `skald link` on a linked device. Whichever finishes
//! first wins. Linked by the code, the machine gets its certificate and a device session of its
//! account from its own key, with no browser sign-in; signed in with the browser on an account
//! without a master key, it is the account's first device and its run ends; on an account with
//! one, it keeps waiting for the code, and linked devices say it waits. Each text of signing in,
//! linking and recovery is one short line.
//!
//! As the tests of linking with a link code (`link_codes/main.rs`): Ada's first machine is linked,
//! her new machine signs in at a terminal, a pseudo-terminal or a pseudo-console on Windows, and
//! her first machine takes its code with `skald link`. Against a `skald-server` with GitHub sign-in
//! through the stand-in, behind the recording relay of the sign-in tests; a test plays the person
//! on the verification page and on local pages in headless Chrome. `BROWSER` names the stand-in
//! opener of the tests of linking on a local page, so no test opens a real browser.
//!
//! The CLI, as these tests use it, beside what the tests of linking with a link code, of linking on
//! a local page, of the recovery key and of the prompt while a device waits use:
//! - `skald sign-in --server <URL>` on a machine without a device, at a terminal, asks for a device
//!   authorization, has the opener open its verification page and prints a line holding the page's
//!   link, `verification_uri_complete`; at once it also opens a run without a session and shows its
//!   code: the run's pairing and its code as one code of 12 characters of the base32 alphabet of
//!   RFC 4648, the pairing first, in three groups of four joined by `-`, such as `PQRS-TUK7-QM2X`.
//! - `skald link` on a linked device takes that code, typed in any case, with spaces or without
//!   `-`, and links the new device as it links one that shows a code of 6 characters.
//! - Linked by the code, the new device stores its certificate and gets a device session of its
//!   account by its key (`POST /devices/{device}/session`, which answers `{"token": <text>}`), keeps
//!   that session's token among its secrets, and exits with success; its device flow ends unused.
//! - Approved first in the browser, on an account without a master key, the device is the account's
//!   first: it shows the recovery key and, once Enter is pressed, exits with success; its run of the
//!   code is over. On an account with a master key, it keeps showing the code and waiting; once
//!   linked by the code, it exits with success.
//! - With `--with-recovery-key`, the same screen; approved in the browser, it asks for the recovery
//!   key and links with it as before, and its run of the code is over.
//! - With `--browser`, the command's local page shows the browser sign-in's link and the code; the
//!   command's output holds no code, also as an agent.
//! - A run killed after the grant, before its device holds a session, stores the certificate and
//!   gets the session at its next run of `skald sign-in`, with no sign-in. A certificate the server
//!   refuses as over the devices limit ends the run failing, on a line that says so, holding `as
//!   many devices` or both `device` and `limit`; a later run of `skald sign-in` links the device
//!   once the account has a place for it.
//! - A text is one short line: each line these flows print, at a terminal, on a local page or in an
//!   MCP tool's result, holds at most [`SHORT`] characters, each link and each ID in it counted as
//!   one.
//!
//! The server's API, as these tests use it besides that of the tests of linking with a link code:
//! `POST /links` answers a new device's hello with `{"pairing": <text>, "token": <text>}`, and `GET
//! /links/messages` with that token as bearer answers `401` once the run is over.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod browser_first;
mod code_first;
mod interrupted;
mod on_page;
mod short_texts;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices and the helpers of the tests of reading the
// history; the helpers of the tests of linking with a link code, the stand-in opener and pages of
// the tests of linking on a local page, the prompt of the tests of a device waiting for linking,
// the agents of the tests of handing messages over, the agent CLI of the tests of the MCP server,
// the recovery key's terminal and signing in there, and linking with it as its tests do.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../mcp_inbox/client.rs"]
mod mcp_client;
#[path = "../link_pages/pages.rs"]
mod pages;
#[path = "../link_prompt/prompt.rs"]
mod prompt;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../recovery_linking/recovering.rs"]
mod recovering;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
// The helpers of linking with a link code name the grants so.
use grants as terminal_grants;

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::{AccountId, DeviceId};

use cloud::{Cloud, Grant, WAIT};
use machine::{Machine, RUN_LIMIT, SERVER, SIGN_IN};
use pages::{BROWSER, Opener};
use relay::strings_named;
use terminal::{Shown, Terminal};
use world::Person;

/// The longest a line of these flows may be, each link and each ID in it counted as one character.
pub const SHORT: usize = 100;

/// The base32 alphabet of RFC 4648, §6, of link codes and pairings.
const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// The request that gets a device session by the device's key, as its line starts once the
/// device's ID is known; every request of a device without a session to `/devices/` starts so.
pub const DEVICE_REQUESTS: &str = "POST /devices/";

/// Each string in `text` in the shown form of a code of 12 characters, three groups of four
/// characters of the alphabet joined by `-`, in order; only a whole token.
pub fn twelve_in(text: &str) -> Vec<String> {
    const LENGTH: usize = 14;
    let chars: Vec<char> = text.chars().collect();
    let edge = |c: Option<&char>| c.is_none_or(|c| !(c.is_alphanumeric() || *c == '-'));
    (0..chars.len().saturating_sub(LENGTH - 1))
        .filter(|&at| {
            let window = &chars[at..at + LENGTH];
            window.iter().enumerate().all(|(i, c)| {
                if i % 5 == 4 {
                    *c == '-'
                } else {
                    ALPHABET.contains(*c)
                }
            }) && edge(at.checked_sub(1).and_then(|before| chars.get(before)))
                && edge(chars.get(at + LENGTH))
        })
        .map(|at| chars[at..at + LENGTH].iter().collect())
        .collect()
}

/// The characters of `code` without its `-`.
pub fn plain(code: &str) -> String {
    code.chars().filter(|c| *c != '-').collect()
}

/// The pairings the server gave runs without a session so far, in order.
pub fn pairings(cloud: &Cloud) -> Vec<String> {
    run_answers(cloud)
        .into_iter()
        .map(|(pairing, _)| pairing)
        .collect()
}

/// The answers the server gave to `POST /links` so far, in order: each run's pairing and token.
pub fn run_answers(cloud: &Cloud) -> Vec<(String, String)> {
    let answered = cloud.relay.answered();
    let mut found = Vec::new();
    for (at, _) in answered.match_indices("\"pairing\"") {
        let Some(start) = answered[..at].rfind('{') else {
            continue;
        };
        let Some(end) = answered[at..].find('}') else {
            continue;
        };
        let Ok(object) = serde_json::from_str::<Value>(&answered[start..=at + end]) else {
            continue;
        };
        if let (Some(pairing), Some(token)) = (object["pairing"].as_str(), object["token"].as_str())
        {
            found.push((pairing.to_owned(), token.to_owned()));
        }
    }
    found
}

/// The link codes `text` shows: codes of 12 characters whose first 6 are a pairing the server gave.
pub fn link_codes_in(cloud: &Cloud, text: &str) -> Vec<String> {
    let given = pairings(cloud);
    twelve_in(text)
        .into_iter()
        .filter(|code| given.iter().any(|pairing| plain(code).starts_with(pairing)))
        .collect()
}

/// The device sessions the server gave by a device's key from `since` on: the `token` of each
/// answer that is not a run's.
pub fn key_sessions(cloud: &Cloud, since: Instant) -> Vec<String> {
    let runs: Vec<String> = run_answers(cloud)
        .into_iter()
        .map(|(_, token)| token)
        .collect();
    let mut tokens = strings_named(&cloud.relay.answered_since(since), "token");
    tokens.retain(|token| !runs.contains(token));
    tokens
}

/// Whether the server holds an open run of the run token `token`.
pub fn run_open(cloud: &Cloud, token: &str) -> bool {
    let (status, body) = cloud.get_as("/links/messages", token);
    match status {
        200 => true,
        401 => false,
        other => panic!("GET /links/messages: {other} {body}"),
    }
}

/// Waits for up to [`WAIT`] until the server holds no open run of any token it gave.
#[track_caller]
pub fn until_no_run_open(cloud: &Cloud) {
    let deadline = Instant::now() + WAIT;
    loop {
        let open: Vec<String> = (run_answers(cloud).into_iter())
            .filter(|(_, token)| run_open(cloud, token))
            .map(|(pairing, _)| pairing)
            .collect();
        if open.is_empty() {
            return;
        }
        assert!(Instant::now() < deadline, "runs still open: {open:?}");
        thread::sleep(Duration::from_millis(100));
    }
}

pub use crate::linking::until;

/// Fails unless `token` is a session of `account` without a sign-in method, which the server
/// takes for the account's devices.
#[track_caller]
pub fn assert_key_session(cloud: &Cloud, token: &str, account: AccountId) {
    let (status, body) = cloud.session_of(token);
    assert_eq!(status, 200, "GET /session: {body}");
    assert_eq!(
        body["account"].as_str(),
        Some(account.to_string().as_str()),
        "a session of the account: {body}"
    );
    assert!(
        body.get("sign_in_method").is_none(),
        "a sign-in method: {body}"
    );
    let (status, body) = cloud.get_as("/devices", token);
    assert_eq!(status, 200, "GET /devices with the session: {body}");
}

/// The issuer of the certificate the server holds of `device`, for `first`'s session: `None` for
/// the master key. Fails if it holds none.
#[track_caller]
pub fn issuer_of(cloud: &Cloud, token: &str, device: DeviceId) -> Option<DeviceId> {
    let cert = (cloud.cert(token, device))
        .unwrap_or_else(|| panic!("no certificate of {device}: {:?}", cloud.devices(token)));
    cert.body().issuer
}

/// The one certified device of `first`'s account named `name`.
#[track_caller]
pub fn certified_named(cloud: &Cloud, token: &str, name: &str) -> DeviceId {
    let named: Vec<DeviceId> = (cloud.devices(token).into_iter())
        .filter(|device| {
            device
                .cert
                .as_ref()
                .is_some_and(|cert| cert.body().name.as_str() == name)
        })
        .map(|device| device.id)
        .collect();
    assert_eq!(
        named.len(),
        1,
        "not one certified device {name:?}: {named:?}"
    );
    named[0]
}

/// `line` with each link and each ID in it as one character.
fn measured(line: &str) -> usize {
    let mut count = 0;
    for word in line.split_inclusive(char::is_whitespace) {
        let bare = word.trim_end();
        let link = ["http://", "https://", "file:"].iter().any(|scheme| {
            bare.trim_start_matches(['(', '"', '\'', '<'])
                .starts_with(scheme)
        });
        if link {
            count += 1 + (word.len() - bare.len());
        } else {
            count += without_ids(word).chars().count();
        }
    }
    count
}

/// `word` with each ID in it, 8-4-4-4-12 hex digits, as one character.
fn without_ids(word: &str) -> String {
    let chars: Vec<char> = word.chars().collect();
    let shape = [8, 4, 4, 4, 12];
    let mut out = String::new();
    let mut at = 0;
    'outer: while at < chars.len() {
        let mut end = at;
        for (i, len) in shape.iter().enumerate() {
            let group = chars.get(end..end + len);
            if !group.is_some_and(|group| group.iter().all(char::is_ascii_hexdigit)) {
                out.push(chars[at]);
                at += 1;
                continue 'outer;
            }
            end += len;
            if i < shape.len() - 1 {
                if chars.get(end) != Some(&'-') {
                    out.push(chars[at]);
                    at += 1;
                    continue 'outer;
                }
                end += 1;
            }
        }
        out.push('#');
        at = end;
    }
    out
}

/// Fails unless each line of `text`, what `what` printed or showed, that holds anything but spaces
/// is one short line: at most [`SHORT`] characters, each link and each ID counted as one.
#[track_caller]
pub fn assert_short_lines(what: &str, text: &str) {
    let long: Vec<&str> = (text.lines())
        .filter(|line| measured(line.trim()) > SHORT)
        .collect();
    assert!(
        long.is_empty(),
        "{what}: lines longer than {SHORT} characters: {long:#?}\nin: {text}"
    );
}

/// A new machine signing in at a terminal, with no device yet, whose `BROWSER` names the stand-in
/// opener; it shows the browser sign-in's line and a link code.
pub struct Fresh {
    /// Dropped first, so its run ends before the machine's directories go.
    run: Option<Terminal>,
    pub opener: Opener,
    pub machine: Machine,
    pub host: &'static str,
    /// When it started, from which the relay's requests and answers are counted.
    pub started: Instant,
    /// The grants device authorization gave before it, and the session tokens the device flow
    /// gave before it.
    pub grants: usize,
    pub tokens: usize,
}

impl Fresh {
    /// A new machine with the host name `host` runs `skald sign-in --server <cloud> <extra>` at a
    /// terminal; gives it once the opener has run, it shows the line with its verification page's
    /// link and it shows a link code, before anyone approves anything.
    #[track_caller]
    pub fn start(cloud: &Cloud, host: &'static str, extra: &[&str]) -> Self {
        Self::start_on(cloud, Machine::new().with_host_name(host), host, extra)
    }

    /// As [`Fresh::start`], on `machine`, whose host name is `host`.
    #[track_caller]
    pub fn start_on(cloud: &Cloud, machine: Machine, host: &'static str, extra: &[&str]) -> Self {
        let opener = Opener::on(&machine);
        let machine = machine.with_env(BROWSER, opener.program());
        let grants = cloud.grants().len();
        let tokens = cloud.tokens().len();
        let started = cloud.relay.mark();
        let mut args = vec![SIGN_IN, SERVER, &cloud.url];
        args.extend_from_slice(extra);
        let run = Terminal::start(&machine, &args);
        let mut fresh = Fresh {
            run: Some(run),
            opener,
            machine,
            host,
            started,
            grants,
            tokens,
        };
        let grant = fresh.grant(cloud);
        let link = complete_uri(cloud, fresh.grants);
        fresh
            .run()
            .wait_for("the sign-in's line and a link code", |shown| {
                shown.contains(&link) && !link_codes_in(cloud, shown).is_empty()
            });
        fresh.wait_for_opening(cloud, &grant);
        fresh
    }

    /// Waits until the opener has run with a file that takes the browser to this sign-in's
    /// verification page, for up to [`RUN_LIMIT`].
    #[track_caller]
    fn wait_for_opening(&mut self, cloud: &Cloud, grant: &Grant) {
        let link = complete_uri(cloud, self.grants);
        let deadline = Instant::now() + RUN_LIMIT;
        loop {
            let opened = (self.opener.openings().iter())
                .filter_map(|args| args.first())
                .any(|arg| {
                    fs::read_to_string(named_file(arg))
                        .is_ok_and(|text| text.replace("&amp;", "&").contains(&link))
                });
            if opened {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the opener got no page of {}: {:?}; {}",
                grant.user_code,
                self.opener.openings(),
                self.shown()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// The grant device authorization gave it, once it has; fails if it ended first.
    #[track_caller]
    pub fn grant(&mut self, cloud: &Cloud) -> Grant {
        let before = self.grants;
        let run = self.run.as_mut().expect("the new machine's run");
        cloud.next_grant(before, || {
            assert!(!run.exited(), "skald exited: {}", run.shown());
            run.shown()
        });
        cloud.grants()[before].clone()
    }

    /// Its run at the terminal.
    pub fn run(&mut self) -> &mut Terminal {
        self.run.as_mut().expect("the new machine's run")
    }

    /// What it showed so far.
    pub fn shown(&self) -> String {
        self.run.as_ref().map(Terminal::shown).unwrap_or_default()
    }

    /// The link code it shows last.
    #[track_caller]
    pub fn code(&self, cloud: &Cloud) -> String {
        link_codes_in(cloud, &self.shown())
            .pop()
            .unwrap_or_else(|| panic!("no link code shown: {}", self.shown()))
    }

    /// Whether its run has exited.
    pub fn exited(&mut self) -> bool {
        self.run().exited()
    }

    /// Waits for its run to exit; gives what it showed.
    pub fn finish(&mut self) -> Shown {
        self.run.take().expect("the new machine's run").finish()
    }

    /// Kills its run at once; gives what it showed.
    pub fn kill(&mut self) -> Shown {
        self.run.take().expect("the new machine's run").kill()
    }

    /// Whether the device flow gave a session token since it started.
    pub fn signed_in(&self, cloud: &Cloud) -> bool {
        cloud.tokens().len() > self.tokens
    }

    /// The device session the server gave it by its key last; fails if it gave none.
    #[track_caller]
    pub fn key_session(&self, cloud: &Cloud) -> String {
        key_sessions(cloud, self.started)
            .pop()
            .expect("a device session by the device's key")
    }

    /// Approves its device authorization as `first`'s person.
    pub fn approve_as(&mut self, world: &world::World, user: &skald_github_stand_in::User) {
        let cloud = &world.cloud;
        let grant = self.grant(cloud);
        cloud.decide(&world.browser.page(), user, &grant, browser::APPROVE);
    }

    /// The person of `first`'s account on this machine, once it is linked and holds the session
    /// `token`, as a test of that account uses it.
    pub fn into_person(mut self, cloud: &Cloud, first: &Person, token: String) -> Person {
        drop(self.run.take());
        let device = certified_named(cloud, &first.token, self.host);
        let cert = (cloud.cert(&first.token, device)).expect("its certificate on the server");
        let holders = self.machine.files_holding(&token);
        Person {
            user: first.user.clone(),
            machine: self.machine,
            token,
            account: first.account,
            master: first.master,
            handle: first.handle.clone(),
            device,
            host: self.host,
            key: keys::signing_key(&holders, &cert.body().signing),
        }
    }
}

/// The path of the file an argument of the opener names, as a path or a `file:` URL.
pub fn named_file(arg: &str) -> std::path::PathBuf {
    let Some(rest) = arg.strip_prefix("file://") else {
        return arg.into();
    };
    let path = rest.replace("%20", " ");
    // `file:///C:/...` on Windows.
    if cfg!(windows) {
        path.trim_start_matches('/').into()
    } else {
        path.into()
    }
}

/// The verification page with the code filled in of the grant device authorization gave after the
/// `before` it gave earlier.
#[track_caller]
pub fn complete_uri(cloud: &Cloud, before: usize) -> String {
    strings_named(&cloud.relay.answered(), "verification_uri_complete")
        .into_iter()
        .nth(before)
        .expect("device authorization gave a verification_uri_complete")
}

/// Fails unless `text`, what a command printed, holds no form of `code` and no code of 12
/// characters.
#[track_caller]
pub fn assert_no_code(what: &str, text: &str, code: &str) {
    for form in linking::forms(code) {
        assert!(
            !text.contains(&form),
            "{what} holds the code as {form:?}: {text}"
        );
    }
    assert_eq!(
        twelve_in(text),
        Vec::<String>::new(),
        "{what} holds a code: {text}"
    );
}

/// The forms of the code `code` that must never leave a machine: its whole forms, and its own 6
/// characters after the pairing, in upper and lower case.
pub fn secret_forms(code: &str) -> Vec<String> {
    let own = plain(code)[6..].to_owned();
    let mut forms = linking::forms(code);
    forms.extend([own.to_uppercase(), own.to_lowercase()]);
    forms
}
