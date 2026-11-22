//! Signing up, linking and recovering end to end (`docs/dev/architecture.md`, "Device linking";
//! `docs/protocol/membership.md`, "Account", "Linking a device" and "Link code"). On a new account,
//! the first machine signs up with one browser sign-in and shows a recovery key of 12 characters; a
//! second machine links with only a link code typed in the first's `skald link`, with no browser
//! sign-in, and agents on both exchange messages; a third recovers with the key pasted after a blank
//! line, with O for each 0 and I for each 1, and shows the channel's history. No output asks whether
//! a device is the account's first, and each text of these flows is one short line.
//!
//! Against a `skald-server` with GitHub sign-in through the stand-in, behind the recording relay of
//! the sign-in tests, whose public address is that of the front of the tests of sending. Each machine runs `skald` at a terminal, a pseudo-terminal or a pseudo-console
//! on Windows; a test plays the person on the verification page in headless Chrome, and `BROWSER`
//! names the stand-in opener of the tests of linking on a local page, so no test opens a real
//! browser. The CLI as the tests of signing in by a link code or the browser
//! (`link_code_sign_in/main.rs`), of the recovery key (`recovery_key_entry/main.rs`) and of
//! handing messages to agents use it.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The modules the tests of the channel history share (`history_common/mod.rs`); the agents of the
// tests of handing messages over; the recovery key's terminal, signing in there and linking with
// it as its tests do; the helpers of the tests of linking with a link code, of reading the history,
// and the stand-in opener of the tests of linking on a local page.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../link_pages/pages.rs"]
mod pages;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../recovery_linking/recovering.rs"]
mod recovering;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
// The helpers of linking with a link code name the grants so.
use grants as terminal_grants;

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{ChannelId, DeviceId};

use agents::{CLAUDE, CODEX, run_as_in};
use at_terminal::{approve, keys_in, shown_key, sign_in};
use cloud::{Cloud, WAIT};
use fixtures::fronted;
use kit::{assert_members, create_space, group, received};
use linking::{YES, codes_in, link_with, until};
use machine::{DEVICES, Machine, NOT_LINKED};
use pages::{BROWSER, Opener};
use reading::history;
use recovering::{KEEP, LOST, play, start_recovery};
use terminal::Terminal;
use world::{ACME, ADD_RULE, AGENT, Person, World, dir_of, path, run};

/// The host names of Ada's three machines.
const WORKSTATION: &str = "ada-workstation";
const LAPTOP: &str = "ada-laptop";
const RECOVERED: &str = "ada-recovered";

/// The folder, on Ada's first two machines, bound to `acme`, where her agents run.
const WORK: &str = "work/acme";

/// What Ada's agents say to each other; neither is anywhere else.
const ASKED: &str = "end to end test: codex on the laptop asks claude which branch to ship";
const ANSWERED: &str = "end to end test: claude says ship the release branch";

/// The longest a text of these flows may be, each link and each ID in it counted as one character:
/// the limit of the tests of signing in by a link code or the browser.
const SHORT: usize = 100;

/// A terminal's bracketed-paste markers, around what is pasted.
const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

/// New accounts tried at most until one's recovery key holds a 0 or a 1: a key holds neither with
/// a chance of (30/32)^12, about 0.46, so all of them miss about 4 times in a million.
const ACCOUNTS: u8 = 16;

/// The GitHub logins of Ada's accounts start so; with a letter for each account tried after a `-`,
/// each is 39 characters, the longest GitHub allows, so a text that names it is short for every
/// account.
const LOGIN: &str = "ada-signs-up-links-by-code-recovers-a";

/// A new machine with the host name `host`, its `BROWSER` naming a stand-in opener of its own;
/// gives both.
fn with_opener(host: &str) -> (Machine, Opener) {
    let bare = Machine::new().with_host_name(host);
    let opener = Opener::on(&bare);
    (bare.with_env(BROWSER, opener.program()), opener)
}

/// How many characters `line` takes, each link and each ID, 8-4-4-4-12 hex digits, as one.
fn length(line: &str) -> usize {
    let words: Vec<&str> = line.split(' ').collect();
    let spaces = words.len().saturating_sub(1);
    let counted: usize = (words.iter())
        .map(|word| {
            let bare = word.trim_start_matches(|c: char| !c.is_alphanumeric());
            if bare.contains("://") || bare.starts_with("file:") {
                1
            } else {
                word.chars().count() - 35 * ids_in(word)
            }
        })
        .sum();
    spaces + counted
}

/// How many IDs, 8-4-4-4-12 hex digits, `word` holds.
fn ids_in(word: &str) -> usize {
    const DASHES: [usize; 4] = [8, 13, 18, 23];
    let chars: Vec<char> = word.chars().collect();
    let mut found = 0;
    let mut at = 0;
    while at + 36 <= chars.len() {
        let id = (0..36).all(|i| {
            let c = chars[at + i];
            if DASHES.contains(&i) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
        if id {
            found += 1;
            at += 36;
        } else {
            at += 1;
        }
    }
    found
}

/// Fails unless each line of `text`, what `what` printed, is one short line, at most [`SHORT`]
/// characters as [`length`] counts them, and none asks whether the device is the account's first:
/// no line both names `first` and asks, and none names `--first-device`.
#[track_caller]
fn assert_short_and_unasked(what: &str, text: &str) {
    for line in text.lines().map(str::trim) {
        assert!(
            length(line) <= SHORT,
            "{what}: a line longer than {SHORT} characters: {line:?}\nin: {text}"
        );
        let lower = line.to_lowercase();
        assert!(
            !(lower.contains("first") && lower.contains('?')) && !lower.contains("--first-device"),
            "{what}: asks whether the device is the first: {line:?}\nin: {text}"
        );
    }
}

/// `key`, a recovery key in its shown form, with O for each 0 and I for each 1.
fn swapped(key: &str) -> String {
    key.replace('0', "O").replace('1', "I")
}

/// `key` without its `-`.
fn plain(key: &str) -> String {
    key.chars().filter(|c| *c != '-').collect()
}

/// `key`, a recovery key in its shown form, as pasted here: after a blank line, inside a
/// terminal's bracketed-paste markers, with O for each 0 and I for each 1.
fn pasted(key: &str) -> String {
    format!("\r{PASTE_START}{}{PASTE_END}", swapped(key))
}

/// Ada's first machine on her new account, the key it showed and what its sign-in showed.
struct SignedUp {
    ada: Person,
    key: String,
    shown: String,
    _opener: Opener,
}

/// GitHub user `id` with `login` signs up on a new machine at a terminal: `skald sign-in` has the
/// opener open its page, the person approves its device authorization once in the browser, and
/// presses Enter once it shows a recovery key. Fails unless the run succeeds after one device
/// authorization and one session of the device flow, and shows one key of 12 characters, in 3
/// groups of 4, and no other.
#[track_caller]
fn signs_up(world: &World, id: u64, login: &str) -> SignedUp {
    let cloud = &world.cloud;
    let user = cloud.user(id, login);
    let (machine, opener) = with_opener(WORKSTATION);
    let (grants, tokens) = (cloud.grants().len(), cloud.tokens().len());

    let mut run = Terminal::start(&machine, &sign_in(cloud, &[]));
    until("the opener ran", || !opener.openings().is_empty());
    approve(cloud, &world.browser.page(), &user, &mut run, grants);
    let key = shown_key(&mut run);
    run.enter();
    let shown = run.finish();

    assert!(shown.success, "signing up failed: {}", shown.text);
    assert_eq!(cloud.grants().len(), grants + 1, "not one browser sign-in");
    assert_eq!(cloud.tokens().len(), tokens + 1, "not one browser sign-in");
    assert_eq!(keys_in(&shown.text), [key.as_str()], "{}", shown.text);
    assert_eq!(plain(&key).chars().count(), 12, "not a key of 12: {key}");
    SignedUp {
        ada: world.person(user, machine, WORKSTATION),
        key,
        shown: shown.text,
        _opener: opener,
    }
}

/// Signs up new accounts, as [`signs_up`] does, until one's recovery key holds a 0 or a 1, so the
/// pasted key reads O as 0 and I as 1; at most [`ACCOUNTS`]. Each sign-up's output is short and asks
/// nothing.
#[track_caller]
fn signs_up_with_a_0_or_a_1(world: &World) -> SignedUp {
    for attempt in 0..ACCOUNTS {
        let login = format!("{LOGIN}-{}", char::from(b'a' + attempt));
        let signed_up = signs_up(world, 4_610_500_001 + u64::from(attempt), &login);
        assert_short_and_unasked("skald sign-in on a new account", &signed_up.shown);
        if signed_up.key.contains(['0', '1']) {
            return signed_up;
        }
    }
    panic!("no recovery key of {ACCOUNTS} accounts holds a 0 or a 1");
}

/// The one device of the account of the session `token` whose certificate names `name`, and the
/// issuer of that certificate: `None` for the master key.
#[track_caller]
fn certified(cloud: &Cloud, token: &str, name: &str) -> (DeviceId, Option<DeviceId>) {
    let devices = cloud.devices(token);
    let named: Vec<(DeviceId, Option<DeviceId>)> = (devices.iter())
        .filter_map(|device| {
            let body = device.cert.as_ref()?.body();
            (body.name.as_str() == name).then_some((device.id, body.issuer))
        })
        .collect();
    assert_eq!(named.len(), 1, "certified devices named {name}: {named:?}");
    named[0]
}

/// `skald --agent <agent> <args>` on `machine` in `folder`; fails unless it succeeds.
#[track_caller]
fn agent_runs(machine: &Machine, folder: &Path, agent: &str, args: &[&str]) {
    let all: Vec<&str> = [AGENT, agent].iter().chain(args).copied().collect();
    let finished = machine.start_in(folder, &all).finish();
    assert!(
        finished.success,
        "{all:?} in {}: {finished:?}",
        folder.display()
    );
}

/// Waits, for up to [`WAIT`], until `skald --agent <agent> recv` on `machine` in `folder` shows
/// `text`.
#[track_caller]
fn agent_gets(machine: &Machine, folder: &Path, agent: &str, text: &str) {
    let deadline = Instant::now() + WAIT;
    let mut shown = Vec::new();
    loop {
        let finished = run_as_in(machine, folder, agent);
        if finished.shown().contains(text) {
            return;
        }
        shown.push(finished.shown());
        assert!(
            Instant::now() < deadline,
            "{agent} never got {text:?}: {shown:#?}"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// Waits, for up to [`WAIT`], until `skald history <channel>` on `machine` succeeds and shows each
/// of `texts`.
#[track_caller]
fn history_shows(machine: &Machine, channel: ChannelId, texts: &[&str]) {
    let deadline = Instant::now() + WAIT;
    loop {
        let shown = history(machine, channel, None);
        if shown.success && texts.iter().all(|text| shown.stdout.contains(text)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "skald history shows not all of {texts:?}: {shown:?}"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// `machine`'s folder [`WORK`], bound to `acme`.
#[track_caller]
fn bound_work(machine: &Machine) -> PathBuf {
    let folder = dir_of(machine, WORK);
    let bound = run(machine, &[ADD_RULE, path(&folder), ACME]);
    assert!(bound.success, "not bound: {bound:?}");
    folder
}

/// On a new account, Ada's first machine signs up at a terminal with one browser sign-in and shows
/// a recovery key of 12 characters, and creates the space `acme`. Her second machine's `skald
/// sign-in` shows a link code, which she types in her first machine's `skald link`, confirming the
/// new device: both runs succeed, the server holds the second device's certificate issued by the
/// first, and the device flow gave the second machine no session. Her agents, codex on the second
/// machine and claude on the first, run with `--agent` in folders bound to `acme`, exchange a
/// message each way. Her third machine runs `skald sign-in --with-recovery-key`, naming the first
/// device lost and keeping the second; she approves its device authorization and pastes the key
/// after a blank line, as a bracketed paste, with O for each 0 and I for each 1: it succeeds
/// without echoing the key, holds a certificate issued by the master key, and its `skald history`
/// of `acme`'s default channel shows both messages. No output of signing up, linking or recovering
/// asks whether a device is the first, and each of their lines is short.
#[test]
fn an_account_signs_up_in_the_browser_links_by_a_code_and_recovers_by_a_pasted_key() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;

    let SignedUp { ada, key, .. } = signs_up_with_a_0_or_a_1(&world);
    let space = create_space(cloud, &ada, ACME);
    let at_work = bound_work(&ada.machine);

    let (laptop, _laptop_opener) = with_opener(LAPTOP);
    let tokens = cloud.tokens().len();
    let mut signing_in = Terminal::start(&laptop, &sign_in(cloud, &[]));
    let shown = signing_in.wait_for("a link code", |shown| !codes_in(shown).is_empty());
    let code = codes_in(&shown).pop().expect("a link code");
    let linked = link_with(&ada, &code, LAPTOP, YES);
    assert!(
        linked.success,
        "skald link failed: {}\nthe laptop showed: {}",
        linked.text,
        signing_in.shown()
    );
    let signed_in = signing_in.finish();
    assert!(signed_in.success, "the laptop failed: {}", signed_in.text);
    assert_short_and_unasked("skald sign-in linked by its code", &signed_in.text);
    assert_short_and_unasked("skald link", &linked.text);
    assert_eq!(
        cloud.tokens().len(),
        tokens,
        "the laptop signed in in the browser"
    );
    let (laptop_device, issuer) = certified(cloud, &ada.token, LAPTOP);
    assert_eq!(
        issuer,
        Some(ada.device),
        "the laptop's certificate's issuer"
    );
    let devices = run(&laptop, &[DEVICES]);
    assert!(devices.success, "{devices:?}");
    assert!(
        !devices.shown().to_lowercase().contains(NOT_LINKED),
        "{devices:?}"
    );

    for machine in [&laptop, &ada.machine, &laptop] {
        received(machine);
    }
    let members = group(cloud, &ada.token, space.id, space.default).members;
    assert_members(&members, &[ada.device, laptop_device]);
    let on_laptop = bound_work(&laptop);
    run_as_in(&ada.machine, &at_work, CLAUDE);
    run_as_in(&laptop, &on_laptop, CODEX);
    let to_claude = format!("{}/{CLAUDE}", ada.handle);
    agent_runs(
        &laptop,
        &on_laptop,
        CODEX,
        &["send", "--to", &to_claude, ASKED],
    );
    agent_gets(&ada.machine, &at_work, CLAUDE, ASKED);
    let to_codex = format!("{}/{CODEX}", ada.handle);
    agent_runs(
        &ada.machine,
        &at_work,
        CLAUDE,
        &["send", "--to", &to_codex, ANSWERED],
    );
    agent_gets(&laptop, &on_laptop, CODEX, ANSWERED);

    let (third, _third_opener) = with_opener(RECOVERED);
    let named = [(LOST, ada.device), (KEEP, laptop_device)];
    let (mut recovering, grants) = start_recovery(&world, &third, &[], &named);
    let paste = pasted(&key);
    play(
        &world,
        &ada.user,
        &mut recovering,
        (grants, Some(&paste)),
        || false,
    );
    let recovered = recovering.finish();
    assert!(recovered.success, "recovering failed: {}", recovered.text);
    assert_short_and_unasked("skald sign-in --with-recovery-key", &recovered.text);
    let shown = recovered.text.to_uppercase();
    for form in [
        key.clone(),
        plain(&key),
        swapped(&key),
        plain(&swapped(&key)),
    ] {
        assert!(
            !shown.contains(&form),
            "the key is echoed: {}",
            recovered.text
        );
    }
    let (_, issuer) = certified(cloud, &cloud.last_token(), RECOVERED);
    assert_eq!(issuer, None, "the recovered device's certificate's issuer");
    history_shows(&third, space.default, &[ASKED, ANSWERED]);
}
