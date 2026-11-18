//! Signing in at the person's terminal, as the tests of the recovery key and of linking with it do:
//! the run's arguments, approving its code, and reading the recovery key it shows.

use std::time::{Duration, Instant};

use skald_github_stand_in::User;

use crate::browser::{APPROVE, Page};
use crate::cloud::{Cloud, WAIT};
use crate::machine::{FIRST_DEVICE, Machine, SERVER, SIGN_IN, YES};
use crate::terminal::Terminal;

/// The agent the tests run commands as.
pub const AGENT: &str = "claude-api";

/// `skald sign-in --server <cloud> <extra>`.
pub fn sign_in<'a>(cloud: &'a Cloud, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![SIGN_IN, SERVER, &cloud.url];
    args.extend_from_slice(extra);
    args
}

/// `skald --agent <AGENT> <args>`.
pub fn as_agent<'a>(args: &[&'a str]) -> Vec<&'a str> {
    [&["--agent", AGENT][..], args].concat()
}

/// Approves, as `user` at `page`, the code that device authorization gives `run` after the
/// `before` it gave earlier, once `run` shows it.
pub fn approve(cloud: &Cloud, page: &Page, user: &User, run: &mut Terminal, before: usize) {
    let deadline = Instant::now() + WAIT;
    while cloud.grants().len() <= before {
        assert!(!run.exited(), "skald exited: {}", run.shown());
        assert!(
            Instant::now() < deadline,
            "no device authorization: {}",
            run.shown()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let grant = cloud.last_grant();
    run.wait_to_show(&grant.user_code);
    cloud.decide(page, user, &grant, APPROVE);
}

/// Starts `skald sign-in --server <cloud> --first-device yes`, with `before` the arguments before
/// it, on `machine` at a terminal, and approves its code as `user`.
pub fn first_at_terminal(
    cloud: &Cloud,
    page: &Page,
    machine: &Machine,
    user: &User,
    before: &[&str],
) -> Terminal {
    let grants = cloud.grants().len();
    let args = [before, &sign_in(cloud, &[FIRST_DEVICE, YES])[..]].concat();
    let mut run = Terminal::start(machine, &args);
    approve(cloud, page, user, &mut run, grants);
    run
}

/// Every string in `text` in the shown form of a recovery key, 52 characters of the base32
/// alphabet of RFC 4648, §6, in groups of four joined by `-`, each once, in order.
pub fn keys_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let digit = |c: char| c.is_ascii_uppercase() || ('2'..='7').contains(&c);
    let mut found: Vec<String> = Vec::new();
    for start in 0..chars.len() {
        let Some(window) = chars.get(start..start + 64) else {
            break;
        };
        let shaped = window.iter().enumerate().all(
            |(at, c)| {
                if at % 5 == 4 { *c == '-' } else { digit(*c) }
            },
        );
        let bounded = |c: Option<&char>| c.is_none_or(|c| !(digit(*c) || *c == '-'));
        if shaped
            && bounded(start.checked_sub(1).and_then(|at| chars.get(at)))
            && bounded(chars.get(start + 64))
        {
            let key: String = window.iter().collect();
            if !found.contains(&key) {
                found.push(key);
            }
        }
    }
    found
}

/// Waits until `run` shows a recovery key; gives it in its shown form. Fails if it shows more than
/// one.
#[track_caller]
pub fn shown_key(run: &mut Terminal) -> String {
    let text = run.wait_for("a recovery key", |text| !keys_in(text).is_empty());
    let keys = keys_in(&text);
    assert_eq!(keys.len(), 1, "keys shown: {text}");
    keys[0].clone()
}
