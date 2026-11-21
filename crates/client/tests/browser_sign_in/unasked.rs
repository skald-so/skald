//! At a terminal, a new device takes its place in the account with no question: the account's first
//! device creates the master key and shows the recovery key, any other waits for linking and shows
//! a link code, and of two at once the one the server refuses waits.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::DeviceId;

use crate::asks_whether_first;
use crate::at_terminal::{keys_in, shown_key, sign_in};
use crate::browser::APPROVE;
use crate::cloud::{GIVE_MASTER, WAIT, certified, ids};
use crate::grants::shown_grant;
use crate::linking::codes_in;
use crate::terminal::Terminal;
use crate::with_opener;
use crate::world::World;

/// Ada's machine on a new account signs in at a terminal. Once she approves its code, with nothing
/// typed, it creates the account's master key, holds a certificate issued with it and shows the
/// recovery key; it asks nothing on the way. Enter ends the run.
#[test]
fn a_new_accounts_first_device_creates_the_master_key_and_shows_its_recovery_key_unasked() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = cloud.user(4_312_640_021, "ada-unasked-standin");
    let (machine, _opener) = with_opener("ada-workstation");

    let mut run = Terminal::start(&machine, &sign_in(cloud, &[]));
    let grant = shown_grant(cloud, &mut run, 0);
    cloud.decide(&world.browser.page(), &ada, &grant, APPROVE);
    shown_key(&mut run);
    run.enter();
    let shown = run.finish();

    assert!(shown.success, "{}", shown.text);
    assert!(
        !asks_whether_first(&shown.text),
        "a question: {}",
        shown.text
    );
    let token = cloud.last_token();
    assert!(cloud.master(&token).is_some(), "no master key");
    let devices = cloud.devices(&token);
    assert_eq!(devices.len(), 1, "{devices:?}");
    assert_eq!(certified(&devices), ids(&devices), "{devices:?}");
}

/// Ada's second machine signs in at a terminal to her account, which has a master key. Once she
/// approves its code, with nothing typed, it shows a link code and waits; it asks nothing, sends no
/// master key, and the account keeps its master key and its one certified device.
#[test]
fn a_device_of_an_account_with_a_master_key_waits_for_linking_unasked() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_312_640_022, "ada-waits-unasked", "ada-desktop");
    let (machine, _opener) = with_opener("ada-laptop");
    let before = cloud.grants().len();
    let mark = cloud.relay.mark();

    let mut run = Terminal::start(&machine, &sign_in(cloud, &[]));
    let grant = shown_grant(cloud, &mut run, before);
    cloud.decide(&world.browser.page(), &ada.user, &grant, APPROVE);
    let shown = run.wait_for("a link code", |shown| !codes_in(shown).is_empty());

    assert!(!asks_whether_first(&shown), "a question: {shown}");
    assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "a master key sent");
    assert_eq!(cloud.master(&ada.token), Some(ada.master));
    let devices = cloud.devices(&ada.token);
    assert_eq!(
        devices.len(),
        2,
        "the new device is not registered: {devices:?}"
    );
    assert_eq!(certified(&devices), vec![ada.device], "{devices:?}");
    run.kill();
}

/// What a run at a terminal shows once it is past signing in: the recovery key of the account's
/// first device, or a link code.
#[derive(Debug, PartialEq)]
enum Outcome {
    Key,
    Code,
}

/// What `run` shows past signing in, if anything yet.
fn outcome(run: &Terminal) -> Option<Outcome> {
    let shown = run.shown();
    if !keys_in(&shown).is_empty() {
        Some(Outcome::Key)
    } else if !codes_in(&shown).is_empty() {
        Some(Outcome::Code)
    } else {
        None
    }
}

/// Two new machines of a new account sign in at terminals at once: the relay holds the first
/// master key one sends until the other's comes. With nothing typed, the account gets one master
/// key and one certified device; one machine shows the recovery key, the other, which the server
/// refused, shows a link code and waits. Neither asks anything.
#[test]
fn of_two_new_devices_at_once_one_creates_the_master_key_and_the_other_waits_for_linking() {
    let world = World::start();
    let cloud = &world.cloud;
    let page = world.browser.page();
    let ada = cloud.user(4_312_640_023, "ada-two-at-once");
    let (one, _one_opener) = with_opener("ada-one");
    let (two, _two_opener) = with_opener("ada-two");
    cloud.relay.hold(GIVE_MASTER, 2);

    let mut run_one = Terminal::start(&one, &sign_in(cloud, &[]));
    let grant_one = shown_grant(cloud, &mut run_one, 0);
    let mut run_two = Terminal::start(&two, &sign_in(cloud, &[]));
    let grant_two = shown_grant(cloud, &mut run_two, 1);
    cloud.decide(&page, &ada, &grant_one, APPROVE);
    cloud.decide(&page, &ada, &grant_two, APPROVE);
    let deadline = Instant::now() + WAIT;
    let outcomes = loop {
        if let (Some(first), Some(second)) = (outcome(&run_one), outcome(&run_two)) {
            break [first, second];
        }
        for run in [&mut run_one, &mut run_two] {
            assert!(!run.exited(), "skald exited: {}", run.shown());
        }
        assert!(
            Instant::now() < deadline,
            "no recovery key or link code: {} / {}",
            run_one.shown(),
            run_two.shown()
        );
        thread::sleep(Duration::from_millis(50));
    };

    assert_eq!(cloud.relay.held(), 2, "both did not send a master key");
    for run in [&run_one, &run_two] {
        assert!(
            !asks_whether_first(&run.shown()),
            "a question: {}",
            run.shown()
        );
    }
    let keys = outcomes
        .iter()
        .filter(|seen| **seen == Outcome::Key)
        .count();
    assert_eq!(
        keys, 1,
        "not one recovery key and one link code: {outcomes:?}"
    );
    let token = cloud.last_token();
    assert!(cloud.master(&token).is_some(), "no master key");
    let devices = cloud.devices(&token);
    assert_eq!(devices.len(), 2, "{devices:?}");
    let certified: Vec<DeviceId> = certified(&devices);
    assert_eq!(certified.len(), 1, "{devices:?}");
    let (winner, loser) = if outcomes[0] == Outcome::Key {
        (run_one, run_two)
    } else {
        (run_two, run_one)
    };
    winner.enter();
    let shown = winner.finish();
    assert!(shown.success, "{}", shown.text);
    loser.kill();
}
