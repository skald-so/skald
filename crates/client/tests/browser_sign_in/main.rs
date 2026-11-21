//! `skald sign-in` opens the device flow's verification page with its code filled in, and a new
//! `client` device takes its place in the account with no question (`docs/protocol/membership.md`,
//! "Linking a device"): it creates the account's master key when the server reports none and
//! waits for linking otherwise. Against a `skald-server` with GitHub sign-in through the GitHub
//! stand-in, on PostgreSQL, behind the recording relay of the sign-in tests, with a test playing
//! the person on the verification page in headless Chrome; the person's terminal is a
//! pseudo-terminal, or a pseudo-console on Windows.
//!
//! The CLI, as these tests use it, beside what the tests of signing in, of the recovery key, of
//! linking with a link code and of linking on a local page use:
//! - `skald sign-in` writes a file readable only by its user that takes the browser to the
//!   verification page with the code filled in, `verification_uri_complete`, and runs the program
//!   `BROWSER` names, the tests' stand-in, with one argument: that file. No argument holds the code.
//!   The file is gone once the command ends. Until its person approves the code, it prints one line,
//!   which holds that link, and nothing else; also with no browser to open.
//! - It asks nothing: on an account without a master key, the device creates it and shows the
//!   recovery key; on one with a master key, it waits for linking and, at a terminal, shows a link
//!   code; of two devices creating it at once, the one the server refuses waits for linking. So
//!   does the local page of `--browser`, which asks nothing before the recovery key or the code.
//! - `--first-device` is an unknown option. `--with-recovery-key` on an account without a master
//!   key says it has none and creates none.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod local_page;
mod no_browser;
mod opening;
mod options;
mod unasked;

// The modules the tests of the channel history share (`history_common/mod.rs`), the terminal and
// signing in there of the tests of the recovery key, the helpers of the tests of linking with a
// link code, and the stand-in opener and pages of the tests of linking on a local page.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../link_pages/pages.rs"]
mod pages;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
// The helpers of linking with a link code name the grants so.
use grants as terminal_grants;

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use cloud::Cloud;
use machine::{Machine, RUN_LIMIT};
use pages::{BROWSER, Opener};
use relay::strings_named;

/// A new machine with the host name `host` whose `BROWSER` names the stand-in opener, which writes
/// down what it is given and opens nothing; gives both.
pub fn with_opener(host: &str) -> (Machine, Opener) {
    let machine = Machine::new().with_host_name(host);
    let opener = Opener::on(&machine);
    let program = opener.program().to_owned();
    (machine.with_env(BROWSER, program), opener)
}

/// Waits until `opener` has run at least `count` times, for up to [`RUN_LIMIT`]; meanwhile
/// `running` fails if the command that runs it ended, and gives what it printed. Gives the
/// arguments of each run.
#[track_caller]
pub fn openings(
    opener: &Opener,
    count: usize,
    mut running: impl FnMut() -> String,
) -> Vec<Vec<String>> {
    let deadline = Instant::now() + RUN_LIMIT;
    loop {
        let openings = opener.openings();
        if openings.len() >= count {
            return openings;
        }
        let shown = running();
        assert!(
            Instant::now() < deadline,
            "the opener ran {} times, not {count}: {shown}",
            openings.len()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// The file the opener was given in `args`, its only argument: a path, or a `file:` URL.
#[track_caller]
pub fn opened_file(args: &[String]) -> PathBuf {
    assert_eq!(args.len(), 1, "the opener's arguments: {args:?}");
    PathBuf::from(args[0].strip_prefix("file://").unwrap_or(&args[0]))
}

/// The verification page with the code filled in that device authorization gave last.
pub fn complete_uri(cloud: &Cloud) -> String {
    strings_named(&cloud.relay.answered(), "verification_uri_complete")
        .pop()
        .expect("device authorization gave a verification_uri_complete")
}

/// The lines of `text` that hold anything but spaces.
pub fn printed_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// Fails unless `text`, what a command printed before its person approved its code, is one line
/// that holds `link`.
#[track_caller]
pub fn assert_only_the_line(text: &str, link: &str) {
    let lines = printed_lines(text);
    assert_eq!(lines.len(), 1, "not one line: {text}");
    assert!(lines[0].contains(link), "the line holds no {link}: {text}");
}

/// The forms of a user code `code` an argument must not hold: as given, in lowercase, and without
/// its `-`.
pub fn code_forms(code: &str) -> Vec<String> {
    let plain: String = code.chars().filter(|c| *c != '-').collect();
    vec![
        code.to_owned(),
        code.to_lowercase(),
        plain.clone(),
        plain.to_lowercase(),
    ]
}

/// Whether `text`, what a command printed or a page showed, asks or names whether a device is the
/// account's first: a line with `first` that asks, or the option `--first-device`.
pub fn asks_whether_first(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.to_lowercase();
        (line.contains("first") && line.contains('?')) || line.contains("--first-device")
    })
}
