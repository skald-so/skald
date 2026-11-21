//! The browser sign-in first: on an account without a master key, it makes the machine the account's
//! first device and ends its run of the code; on an account with one, the machine keeps waiting for
//! its code, which linked devices then say, and the code links it; with `--with-recovery-key`, the
//! sign-in and the recovery key link it.

use std::thread;
use std::time::{Duration, Instant};

use crate::at_terminal::keys_in;
use crate::cloud::{Cloud, WAIT};
use crate::linking::{YES, link_with, typed};
use crate::machine::DEVICES;
use crate::prompt::{UNVERIFIED, assert_not_prompted, assert_prompted};
use crate::recovering::{LOST, WITH_RECOVERY_KEY, play, sign_up};
use crate::terminal::Terminal;
use crate::world::{World, run};
use crate::{Fresh, certified_named, issuer_of, link_codes_in, until, until_no_run_open};

/// The host names of Ada's machines.
const WORKSTATION: &str = "ada-workstation";
const LAPTOP: &str = "ada-laptop";

/// The recovery keys `run` shows: strings in a recovery key's shown form that are no link code.
pub fn recovery_keys(cloud: &Cloud, run: &Terminal) -> Vec<String> {
    let shown = run.shown();
    let codes = link_codes_in(cloud, &shown);
    let mut keys = keys_in(&shown);
    keys.retain(|key| !codes.contains(key));
    keys
}

/// Waits until `fresh` shows a recovery key, for up to [`WAIT`]; gives the keys it shows.
#[track_caller]
pub fn shown_recovery_keys(cloud: &Cloud, fresh: &mut Fresh) -> Vec<String> {
    let deadline = Instant::now() + WAIT;
    loop {
        let keys = recovery_keys(cloud, fresh.run());
        if !keys.is_empty() {
            return keys;
        }
        assert!(!fresh.exited(), "skald exited: {}", fresh.shown());
        assert!(
            Instant::now() < deadline,
            "no recovery key: {}",
            fresh.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// Ada's machine signs in at a terminal on her new account and shows a link code; she approves its
/// device authorization in the browser. With nothing typed, it shows a recovery key, and Enter ends
/// it with success. The account has a master key and one device, certified by the master key, and
/// the server holds no open run of the code.
#[test]
fn on_a_new_account_the_browser_sign_in_makes_the_first_device_and_ends_the_run() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = cloud.user(4_193_201_001, "ada-signs-up-in-the-browser");
    let mut fresh = Fresh::start(cloud, WORKSTATION, &[]);

    fresh.approve_as(&world, &ada);
    let keys = shown_recovery_keys(cloud, &mut fresh);
    assert_eq!(keys.len(), 1, "{}", fresh.shown());
    fresh.run().enter();
    let shown = fresh.finish();

    assert!(shown.success, "{}", shown.text);
    let token = cloud.last_token();
    assert!(cloud.master(&token).is_some(), "no master key");
    let devices = cloud.devices(&token);
    assert_eq!(devices.len(), 1, "{devices:?}");
    let device = certified_named(cloud, &token, WORKSTATION);
    assert_eq!(
        issuer_of(cloud, &token, device),
        None,
        "not by the master key"
    );
    until_no_run_open(cloud);
}

/// Ada's new machine signs in at a terminal to her account, which has a master key, and shows a
/// link code. Her first machine's `skald devices` says nothing of it. She approves its device
/// authorization in the browser first: the new machine keeps running and showing its code, and her
/// first machine's `skald devices` then names it waiting, unverified, with `skald link`. Her first
/// machine's `skald link` takes the code: the new machine exits with success, and the server holds
/// its certificate, issued by her first device.
#[test]
fn on_an_account_with_a_master_key_the_browser_sign_in_keeps_the_machine_waiting_for_its_code() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_193_202_001, "ada-signs-in-then-links", WORKSTATION);
    let mut fresh = Fresh::start(cloud, LAPTOP, &[]);
    let before = run(&ada.machine, &[DEVICES]);
    assert!(before.success, "{before:?}");
    assert_not_prompted(&before.shown(), LAPTOP);

    fresh.approve_as(&world, &ada.user);
    until("the new device signed in", || fresh.signed_in(cloud));
    let mut prompted = String::new();
    until("a prompt on the first machine", || {
        let devices = run(&ada.machine, &[DEVICES]);
        assert!(devices.success, "{devices:?}");
        prompted = devices.shown();
        prompted.to_lowercase().contains(UNVERIFIED)
    });
    assert_prompted(&prompted, LAPTOP);
    assert!(
        !fresh.exited(),
        "the new machine stopped: {}",
        fresh.shown()
    );
    let code = fresh.code(cloud);

    let linked = link_with(&ada, &typed(&code), LAPTOP, YES);
    assert!(linked.success, "skald link failed: {}", linked.text);
    let shown = fresh.finish();
    assert!(shown.success, "the new machine failed: {}", shown.text);
    let device = certified_named(cloud, &ada.token, LAPTOP);
    assert_eq!(issuer_of(cloud, &ada.token, device), Some(ada.device));
    until_no_run_open(cloud);
}

/// Ada's account was made by her first machine at a terminal, which showed its recovery key. Her
/// new machine runs `skald sign-in --with-recovery-key`, naming her first device lost, at a
/// terminal: it shows the sign-in's line and a link code. She approves its device authorization and
/// types the key once asked: the run exits with success, the server holds the new device's
/// certificate, issued by the master key, and no open run of the code.
#[test]
fn with_the_recovery_key_the_browser_sign_in_and_the_key_link_the_machine() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = sign_up(
        &world,
        (4_193_203_001, "ada-recovers-by-browser"),
        WORKSTATION,
    );
    let first = ada.person.device.to_string();
    let mut fresh = Fresh::start(cloud, LAPTOP, &[WITH_RECOVERY_KEY, LOST, &first]);
    let grants = fresh.grants;

    play(
        &world,
        ada.user(),
        fresh.run(),
        (grants, Some(ada.key.as_str())),
        || false,
    );
    let shown = fresh.finish();

    assert!(shown.success, "{}", shown.text);
    let token = cloud.last_token();
    let device = certified_named(cloud, &token, LAPTOP);
    assert_eq!(
        issuer_of(cloud, &token, device),
        None,
        "not by the master key"
    );
    until_no_run_open(cloud);
}
