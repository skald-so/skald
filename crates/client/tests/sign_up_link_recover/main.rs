//! Signing up, linking and recovering end to end (`docs/dev/architecture.md`, "Device linking";
//! `docs/protocol/membership.md`, "Account", "Linking a device" and "Link code"). On a new account,
//! the first machine signs up with one browser sign-in and shows a recovery key of 12 characters; a
//! second machine links with only a link code typed in the first's `skald link`, with no browser
//! sign-in, and agents on both exchange messages; a third recovers with the key pasted after a blank
//! line, with O for each 0 and I for each 1, and shows the channel's history. No output asks whether
//! a device is the account's first, and each text of these flows is one short line.
//!
//! Runs the CLI in a Unix pseudo-terminal or Windows ConPTY. Headless Chrome follows the file
//! passed to the stand-in browser opener and signs in through the local GitHub stand-in.

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

use skald_github_stand_in::User;
use skald_proto::{ChannelId, DeviceId};

use agents::{CLAUDE, CODEX, run_as_in};
use at_terminal::{keys_in, sign_in};
use browser::{APPROVE, Browser, CODE_FIELD, SIGN_IN_GITHUB};
use cloud::{Cloud, WAIT};
use fixtures::fronted;
use kit::{assert_members, group, latest, received};
use linking::{LINK, OWN_DEVICE, YES, codes_in, until};
use machine::{DEVICES, Machine, NOT_LINKED};
use pages::{BROWSER, Opener, file_url, named_file};
use reading::history;
use recovering::{KEEP, LOST, recovery_args};
use terminal::Terminal;
use world::{ACME, ADD_RULE, AGENT, CREATE_SPACE, World, dir_of, handle_of, path, run, space_id};

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

/// About 9.5% of generated keys contain both digits. The retry limit keeps the chance of missing
/// both below one in a million.
const ACCOUNTS: u64 = 160;
const LOGIN: &str = "ada-signs-up-links-by-code-recovers";

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

/// Checks the length of each line and detects the old first-device question and option.
#[track_caller]
fn assert_short_and_unasked(what: &str, text: &str) {
    let words: Vec<_> = text.split_whitespace().collect();
    assert!(
        !words.join(" ").to_lowercase().contains("first device?"),
        "{what}: asks whether the device is the first"
    );
    for (index, line) in text.lines().map(str::trim).enumerate() {
        assert!(
            length(line) <= SHORT,
            "{what}: line {} is longer than {SHORT} characters",
            index + 1
        );
        let lower = line.to_lowercase();
        assert!(
            !(lower.contains("first") && lower.contains('?')) && !lower.contains("--first-device"),
            "{what}: asks whether the device is the first"
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

#[track_caller]
fn opened_file(opener: &Opener) -> PathBuf {
    until("the browser opener ran", || !opener.openings().is_empty());
    let openings = opener.openings();
    assert_eq!(openings.len(), 1, "browser opener runs");
    assert_eq!(openings[0].len(), 1, "browser opener arguments");
    let file = named_file(&openings[0][0]);
    assert!(file.is_file(), "the browser opener did not get a file");
    file
}

/// Follows the file given to the browser opener and approves the prefilled code.
#[track_caller]
fn approve_opened(world: &World, user: &User, opener: &Opener, before: usize) {
    let file = opened_file(opener);
    let cloud = &world.cloud;
    let grant = cloud
        .grants()
        .get(before)
        .cloned()
        .expect("device authorization");
    let browser = Browser::launch();
    let page = browser.page();
    assert_eq!(
        page.goto(&cloud.github.sign_in_url(user.id)).status,
        Some(200),
        "GitHub stand-in session"
    );
    let opened = page.goto_settled(&file_url(&file));
    assert!(
        opened.error.is_none(),
        "could not open the verification page"
    );
    assert_eq!(page.count(SIGN_IN_GITHUB), 1, "GitHub sign-in action");
    let signed_in = page.click_settled(&format!("{SIGN_IN_GITHUB} >> nth=0"));
    assert!(signed_in.error.is_none(), "browser sign-in failed");
    assert!(
        page.values(CODE_FIELD) == [grant.user_code],
        "the verification page did not prefill this sign-in's code"
    );
    assert_eq!(page.click(APPROVE).status, Some(200), "device approval");
}

#[track_caller]
fn fallback_line(cloud: &Cloud, output: &str) {
    let grant = cloud.last_grant();
    assert_one_line(
        "browser fallback",
        output,
        &["Sign in", &grant.verification_uri, &grant.user_code],
    );
}

/// Every required part of a prompt or status appears together on exactly one line.
#[track_caller]
fn assert_one_line(what: &str, text: &str, parts: &[&str]) {
    assert_eq!(
        text.lines()
            .filter(|line| parts.iter().all(|part| line.contains(part)))
            .count(),
        1,
        "{what} does not appear on exactly one line"
    );
}

struct SignedUp {
    machine: Machine,
    user: User,
    token: String,
    device: DeviceId,
    key: String,
}

/// Signs up a new account via the opened page and captures its key before acknowledging it.
#[track_caller]
fn signs_up(world: &World, id: u64, login: &str) -> SignedUp {
    let cloud = &world.cloud;
    let user = cloud.user(id, login);
    let (machine, opener) = with_opener(WORKSTATION);
    let (grants, tokens) = (cloud.grants().len(), cloud.tokens().len());

    let mut run = Terminal::start(&machine, &sign_in(cloud, &[])).with_private_output();
    approve_opened(world, &user, &opener, grants);
    run.wait_to_show("Press Enter once it is written down");
    let shown = run.shown();
    let mut keys = keys_in(&shown);
    assert_eq!(
        keys.len(),
        1,
        "the first device did not show one recovery key"
    );
    let key = keys.pop().expect("a recovery key");
    assert_eq!(plain(&key).chars().count(), 12, "recovery key length");
    run.enter();
    let shown = run.finish();

    assert!(shown.success, "sign-up failed");
    assert!(
        keys_in(&shown.text) == [key.as_str()],
        "extra recovery key shown"
    );
    assert_eq!(
        cloud.grants().len(),
        grants + 1,
        "device authorizations for sign-up"
    );
    assert_eq!(
        cloud.tokens().len(),
        tokens + 1,
        "device sessions from browser sign-in"
    );
    let token = cloud.last_token();
    assert_eq!(cloud.login_of(&token), login);
    let (device, issuer) = certified(cloud, &token, WORKSTATION);
    assert_eq!(issuer, None, "the first device's certificate issuer");
    fallback_line(cloud, &shown.text);
    assert_short_and_unasked("skald sign-in on a new account", &shown.text);
    assert_one_line(
        "recovery key heading",
        &shown.text,
        &["recovery key", "shown only once"],
    );
    assert_one_line(
        "recovery key keep instruction",
        &shown.text,
        &[
            "Keep it apart from your devices: it recovers the account and its history if all are lost.",
        ],
    );
    assert_one_line(
        "recovery key confirmation",
        &shown.text,
        &["Press Enter", "written down"],
    );
    SignedUp {
        machine,
        user,
        token,
        device,
        key,
    }
}

/// Selects a key that tests both digit aliases without controlling the CLI's key generator.
#[track_caller]
fn signs_up_with_both_digits(world: &World) -> SignedUp {
    for attempt in 0..ACCOUNTS {
        let login = format!("{LOGIN}-{attempt:03}");
        assert_eq!(login.len(), 39, "GitHub's maximum login length");
        let signed_up = signs_up(world, 4_610_500_001 + attempt, &login);
        if signed_up.key.contains('0') && signed_up.key.contains('1') {
            return signed_up;
        }
    }
    panic!("no recovery key contained both 0 and 1 after {ACCOUNTS} sign-ups");
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

    let first = signs_up_with_both_digits(&world);
    let created = run(&first.machine, &[CREATE_SPACE, ACME]);
    assert!(created.success, "{created:?}");
    let space = space_id(cloud, &first.token, ACME);
    let roster = latest(cloud, &first.token, space);
    let channel = roster.body().channels.first().expect("default channel").id;
    let at_work = bound_work(&first.machine);

    let (laptop, opener) = with_opener(LAPTOP);
    let tokens = cloud.tokens().len();
    let mut signing_in = Terminal::start(&laptop, &sign_in(cloud, &[])).with_private_output();
    let shown = signing_in.wait_for("a link code", |shown| !codes_in(shown).is_empty());
    let code = codes_in(&shown).pop().expect("a link code");
    opened_file(&opener);
    let mut linking = Terminal::start(&first.machine, &[LINK]).with_private_output();
    linking.wait_to_show("Link code:");
    linking.type_line(&code);
    let confirmation = linking.wait_to_show("[yes/no]");
    assert_one_line(
        "new device identity",
        &confirmation,
        &["Link", LAPTOP, "client device"],
    );
    linking.type_line(YES);
    let linked = linking.finish();
    assert!(linked.success, "skald link failed");
    let signed_in = signing_in.finish();
    assert!(signed_in.success, "linking by code failed");
    fallback_line(cloud, &signed_in.text);
    assert_one_line(
        "link-code instruction",
        &signed_in.text,
        &["run skald link", "type this code there"],
    );
    assert_one_line(
        "link-code reminder",
        &linked.text,
        &["Enter only", OWN_DEVICE, "joins this account"],
    );
    assert_one_line(
        "link confirmation",
        &linked.text,
        &["Link it to this account?", "[yes/no]"],
    );
    assert_short_and_unasked("skald sign-in linked by its code", &signed_in.text);
    assert_short_and_unasked("skald link", &linked.text);
    assert_eq!(
        cloud.tokens().len(),
        tokens,
        "the laptop signed in in the browser"
    );
    let (laptop_device, issuer) = certified(cloud, &first.token, LAPTOP);
    assert_eq!(
        issuer,
        Some(first.device),
        "the laptop's certificate's issuer"
    );
    let devices = run(&laptop, &[DEVICES]);
    assert!(devices.success, "{devices:?}");
    assert!(
        !devices.shown().to_lowercase().contains(NOT_LINKED),
        "{devices:?}"
    );

    for machine in [&laptop, &first.machine, &laptop] {
        received(machine);
    }
    let members = group(cloud, &first.token, space, channel).members;
    assert_members(&members, &[first.device, laptop_device]);
    let on_laptop = bound_work(&laptop);
    run_as_in(&first.machine, &at_work, CLAUDE);
    run_as_in(&laptop, &on_laptop, CODEX);
    let handle = handle_of(cloud, &first.token);
    let to_claude = format!("{handle}/{CLAUDE}");
    agent_runs(
        &laptop,
        &on_laptop,
        CODEX,
        &["send", "--to", &to_claude, ASKED],
    );
    agent_gets(&first.machine, &at_work, CLAUDE, ASKED);
    let to_codex = format!("{handle}/{CODEX}");
    agent_runs(
        &first.machine,
        &at_work,
        CLAUDE,
        &["send", "--to", &to_codex, ANSWERED],
    );
    agent_gets(&laptop, &on_laptop, CODEX, ANSWERED);

    let (third, opener) = with_opener(RECOVERED);
    let named = [(LOST, first.device), (KEEP, laptop_device)];
    let (grants, tokens) = (cloud.grants().len(), cloud.tokens().len());
    let args = recovery_args(cloud, &[], &named);
    let mut recovering = Terminal::start(&third, &fixtures::strs(&args)).with_private_output();
    approve_opened(&world, &first.user, &opener, grants);
    let prompt = recovering.wait_to_show("then press Enter:");
    assert_one_line(
        "recovery input",
        &prompt,
        &["Type or paste", "recovery key", "then press Enter:"],
    );
    let key = &first.key;
    recovering.type_line(&pasted(key));
    let recovered = recovering.finish();
    assert!(recovered.success, "recovery failed");
    assert_eq!(
        cloud.grants().len(),
        grants + 1,
        "device authorizations for recovery"
    );
    assert_eq!(
        cloud.tokens().len(),
        tokens + 1,
        "device sessions for recovery"
    );
    fallback_line(cloud, &recovered.text);
    assert_short_and_unasked("skald sign-in --with-recovery-key", &recovered.text);
    assert_one_line(
        "recovery success",
        &recovered.text,
        &["Linked with the recovery key, which this device keeps nowhere."],
    );
    let shown = recovered.text.to_uppercase();
    for form in [key.clone(), plain(key), swapped(key), plain(&swapped(key))] {
        assert!(!shown.contains(&form), "the recovery key was echoed");
    }
    let (_, issuer) = certified(cloud, &cloud.last_token(), RECOVERED);
    assert_eq!(issuer, None, "the recovered device's certificate's issuer");
    history_shows(&third, channel, &[ASKED, ANSWERED]);
}
