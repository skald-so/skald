//! Signing in at the person's terminal, as the tests of the recovery key and of linking with it do:
//! the run's arguments, approving its code, reading the recovery key it shows, and the lines that
//! say one waits.

use skald_github_stand_in::User;

use crate::browser::{APPROVE, Page};
use crate::cloud::Cloud;
use crate::machine::{Machine, SERVER, SIGN_IN};
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
    let grant = crate::grants::shown_grant(cloud, run, before);
    cloud.decide(page, user, &grant, APPROVE);
}

/// Starts `skald sign-in --server <cloud>`, with `before` the arguments before
/// it, on `machine` at a terminal, and approves its code as `user`.
pub fn first_at_terminal(
    cloud: &Cloud,
    page: &Page,
    machine: &Machine,
    user: &User,
    before: &[&str],
) -> Terminal {
    let grants = cloud.grants().len();
    let args = [before, &sign_in(cloud, &[])[..]].concat();
    let mut run = Terminal::start(machine, &args);
    approve(cloud, page, user, &mut run, grants);
    run
}

/// Every string in `text` in the shown form of a recovery key, 3 groups of 4 characters of
/// Crockford's base32 joined by `-`, each once, in order; not a link code of the same form, whose
/// first 6 characters are a pairing a server gave ([`crate::relay::given_pairings`]).
pub fn keys_in(text: &str) -> Vec<String> {
    let pairings = crate::relay::given_pairings();
    let mut keys = shaped_keys_in(text);
    keys.retain(|key| {
        let plain: String = key.chars().filter(|c| *c != '-').collect();
        !pairings
            .iter()
            .any(|pairing| plain.starts_with(pairing.as_str()))
    });
    keys
}

/// Every string in `text` in the shown form of a recovery key, each once, in order.
fn shaped_keys_in(text: &str) -> Vec<String> {
    const LENGTH: usize = 14;
    let chars: Vec<char> = text.chars().collect();
    let digit = |c: char| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c);
    let mut found: Vec<String> = Vec::new();
    for start in 0..chars.len() {
        let Some(window) = chars.get(start..start + LENGTH) else {
            break;
        };
        let shaped = window.iter().enumerate().all(
            |(at, c)| {
                if at % 5 == 4 { *c == '-' } else { digit(*c) }
            },
        );
        let bounded =
            |c: Option<&char>| c.is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == '-'));
        if shaped
            && bounded(start.checked_sub(1).and_then(|at| chars.get(at)))
            && bounded(chars.get(start + LENGTH))
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

/// The lines of `text` that say a recovery key waits: those that hold `recovery key`, in any
/// case, and the command `skald sign-in --server <cloud>`.
pub fn waiting_lines<'t>(cloud: &Cloud, text: &'t str) -> Vec<&'t str> {
    let command = format!("skald sign-in --server {}", cloud.url);
    text.lines()
        .filter(|line| line.contains(&command) && line.to_lowercase().contains("recovery key"))
        .collect()
}
