//! The code first: a new machine linked by its code from a linked device of the account gets its
//! certificate and a device session of the account with no browser sign-in, gets into the
//! account's channels, and its agents and the linked device's exchange messages. The code never
//! leaves the machines.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use crate::agents::{ACME_DIR, Agents, CLAUDE, CODEX, run_as_in};
use crate::cloud::{Cloud, WAIT};
use crate::kit::{assert_members, group, received};
use crate::linking::{YES, link_with, typed};
use crate::machine::{DEVICES, Finished, Machine, NOT_LINKED};
use crate::prompt::assert_not_prompted;
use crate::terminal::Shown;
use crate::world::{ACME, ADD_RULE, AGENT, Person, World, dir_of, path, run};
use crate::{
    Fresh, assert_key_session, certified_named, issuer_of, secret_forms, until_no_run_open,
};

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// What the agents say to each other; neither is anywhere else.
const ASKED: &str = "link code sign-in test: codex on the laptop asks claude for the build's port";
const ANSWERED: &str = "link code sign-in test: claude says the build listens on 8080";

/// Ada's new machine, linked by its code, and what the runs showed.
pub struct Linked {
    pub fresh: Fresh,
    pub code: String,
    /// What the new machine's sign-in showed, and Ada's `skald link`.
    pub signed_in: Shown,
    pub link: Shown,
}

/// Ada's new machine `host` signs in at a terminal and shows a link code; nobody approves its
/// device authorization. Before her first machine takes the code, `while_waiting` runs. Her first
/// machine's `skald link` takes the code, typed in lowercase with spaces, and she confirms the new
/// device's name and kind: both runs exit with success. Gives the new machine and what they showed.
#[track_caller]
pub fn linked_by_code(
    cloud: &Cloud,
    ada: &Person,
    host: &'static str,
    while_waiting: impl FnOnce(&mut Fresh),
) -> Linked {
    let mut fresh = Fresh::start(cloud, host, &[]);
    let code = fresh.code(cloud);
    while_waiting(&mut fresh);
    let link = link_with(ada, &typed(&code), host, YES);
    assert!(link.success, "skald link failed: {}", link.text);
    let signed_in = fresh.finish();
    assert!(
        signed_in.success,
        "the new machine failed: {}",
        signed_in.text
    );
    Linked {
        fresh,
        code,
        signed_in,
        link,
    }
}

/// Ada's new machine signs in at a terminal: its device flow's page goes to the opener, it prints
/// the line with that page's link, and it shows a link code, all before anyone approves anything.
/// While it waits, her first machine's `skald devices` says nothing of a waiting device. Her first
/// machine's `skald link` takes the code: the new machine exits with success, and nobody approved
/// its device authorization. The server holds the new device's certificate, issued by her first
/// device, and the new machine holds a device session of her account, which has no sign-in method;
/// no run of the code is left open, and the new machine's `skald devices` shows it linked.
#[test]
fn a_new_machine_linked_by_its_code_gets_its_certificate_and_session_with_no_sign_in() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_193_101_001, "ada-links-by-code-only", "ada-workstation");

    let linked = linked_by_code(cloud, &ada, LAPTOP, |_| {
        let devices = run(&ada.machine, &[DEVICES]);
        assert!(devices.success, "{devices:?}");
        assert_not_prompted(&devices.shown(), LAPTOP);
    });

    let fresh = &linked.fresh;
    assert!(
        !fresh.signed_in(cloud),
        "the device flow gave a session: {}",
        linked.signed_in.text
    );
    let device = certified_named(cloud, &ada.token, LAPTOP);
    assert_eq!(issuer_of(cloud, &ada.token, device), Some(ada.device));
    let token = fresh.key_session(cloud);
    assert_key_session(cloud, &token, ada.account);
    assert!(
        !fresh.machine.files_holding(&token).is_empty(),
        "the new machine keeps no session"
    );
    until_no_run_open(cloud);
    let on_laptop = run(&fresh.machine, &[DEVICES]);
    assert!(on_laptop.success, "{on_laptop:?}");
    assert!(
        !on_laptop.shown().to_lowercase().contains(NOT_LINKED),
        "{on_laptop:?}"
    );
}

/// Ada's new machine is linked by its code: no request the machines sent through the relay and no
/// row of the server's database holds the code, whole or its own 6 characters after the pairing,
/// in any case, with or without `-` and spaces.
#[test]
fn no_request_and_no_row_holds_the_code() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_193_102_001, "ada-keeps-her-code", "ada-workstation");

    let linked = linked_by_code(cloud, &ada, LAPTOP, |_| {});

    let sent = cloud.relay.sent_bytes();
    let stored = cloud.rows().join("\n").into_bytes();
    for form in secret_forms(&linked.code) {
        assert!(!holds(&sent, &form), "a request holds the code as {form:?}");
        assert!(
            !holds(&stored, &form),
            "the server holds the code as {form:?}"
        );
    }
}

/// Whether `bytes` hold `form`.
fn holds(bytes: &[u8], form: &str) -> bool {
    crate::linking::holds(bytes, form)
}

/// `skald --agent <agent> <args>` on `machine` in `folder`; fails unless it succeeds.
#[track_caller]
fn as_agent_in(machine: &Machine, folder: &Path, agent: &str, args: &[&str]) -> Finished {
    let all = [&[AGENT, agent][..], args].concat();
    let finished = machine.start_in(folder, &all).finish();
    assert!(finished.success, "{all:?}: {finished:?}");
    finished
}

/// Runs `skald --agent <agent> recv` on `machine` in `folder` until it shows `text`, for up to
/// [`WAIT`]; gives what it showed then.
#[track_caller]
fn received_by(machine: &Machine, folder: &Path, agent: &str, text: &str) -> String {
    let deadline = Instant::now() + WAIT;
    let mut seen = String::new();
    loop {
        let shown = run_as_in(machine, folder, agent).shown();
        seen.push_str(&shown);
        if shown.contains(text) {
            return shown;
        }
        assert!(
            Instant::now() < deadline,
            "{agent} never got {text:?}: {seen}"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// In a talk of Ada and Bob in `acme`, Ada's new machine is linked by its code with no browser
/// sign-in. Once it and her first machine have run, both of `acme`'s channels have Ada's two
/// devices and Bob's. `~/work/acme` is bound to `acme` on both machines; codex on the new machine
/// sends claude on the first one a message, which claude's `skald recv` shows, and claude's answer
/// to codex shows in codex's `skald recv`.
#[test]
fn agents_on_a_machine_linked_by_its_code_and_on_the_linked_one_exchange_messages() {
    let agents = Agents::new(
        [4_193_103_001, 4_193_103_002],
        [
            "ada-agents-on-a-code-linked-laptop",
            "bob-beside-the-laptop",
        ],
    );
    let talk = &agents.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);

    let linked = linked_by_code(cloud, ada, LAPTOP, |_| {});
    assert!(
        !linked.fresh.signed_in(cloud),
        "the device flow gave a session"
    );
    let token = linked.fresh.key_session(cloud);
    let laptop = linked.fresh.into_person(cloud, ada, token);
    for machine in [&laptop.machine, &ada.machine, &laptop.machine] {
        received(machine);
    }
    let at = talk.at();
    let devices = [ada.device, talk.played[0].device(), laptop.device];
    for channel in [talk.space.default, at.1] {
        assert_members(&group(cloud, &ada.token, at.0, channel).members, &devices);
    }

    let folder = dir_of(&laptop.machine, ACME_DIR);
    let bound = run(&laptop.machine, &[ADD_RULE, path(&folder), ACME]);
    assert!(bound.success, "not bound: {bound:?}");
    run_as_in(&ada.machine, &agents.folder, CLAUDE);
    run_as_in(&laptop.machine, &folder, CODEX);

    let to_claude = format!("{}/{CLAUDE}", ada.handle);
    as_agent_in(
        &laptop.machine,
        &folder,
        CODEX,
        &["send", "--to", &to_claude, ASKED],
    );
    received_by(&ada.machine, &agents.folder, CLAUDE, ASKED);
    let to_codex = format!("{}/{CODEX}", ada.handle);
    as_agent_in(
        &ada.machine,
        &agents.folder,
        CLAUDE,
        &["send", "--to", &to_codex, ANSWERED],
    );
    received_by(&laptop.machine, &folder, CODEX, ANSWERED);
}
