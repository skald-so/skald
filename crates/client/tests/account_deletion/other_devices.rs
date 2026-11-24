//! The account's other devices, once it is deleted: each command and the helper say once that the
//! server reports the account deleted and which state directory `skald delete-account` erases, and
//! send nothing.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crate::deletion::{
    assert_says_deleted_once, deleted_at_terminal, deleted_lines, hand_over, shared_with, until,
};
use crate::edits::SPACE_CHANGE;
use crate::fixtures::{fronted, send};
use crate::kit::latest;
use crate::linking::{NewDevice, link};
use crate::machine::{Machine, Run};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::run;

/// The host name of Ada's second machine.
const LAPTOP: &str = "ada-laptop";
/// How long the test watches the helper after it said the account is deleted, for it to say so
/// again.
const WATCH: Duration = Duration::from_secs(5);

/// The socket of the helper of `machine`, there while one listens.
fn socket(machine: &Machine) -> PathBuf {
    machine.state_dir().join("helper").join("sock")
}

/// `skald helper` on `machine`, once it listens; killed once dropped.
fn helper(machine: &Machine) -> Run {
    let started = machine.start(&["helper"]);
    let socket = socket(machine);
    until("a helper listening", || socket.exists(), String::new);
    started
}

/// Ada's first machine created `acme`, with Bob a member, and handed her admin rights there to
/// Bob; her second machine is linked with a link code and runs `skald helper`. Once her first
/// machine's `skald delete-account` has deleted her account, the second machine's helper, and
/// then its `skald recv` and its `skald send` to `acme`'s default channel, each say once that the
/// server reports the account deleted and name the machine's state directory; nothing sends an
/// envelope, a history upload or a revision.
#[test]
fn the_accounts_other_devices_say_it_is_deleted_and_send_nothing() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_006_001, "ada-leaves-a-laptop", ADA_HOST);
    let bob = world.sign_up(6_120_006_002, "bob-keeps-acme", BOB_HOST);
    let space = shared_with(cloud, &ada, &bob);
    hand_over(&ada, &bob);
    let channel = latest(cloud, &bob.token, space).body().channels[0].id;
    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    link(cloud, &ada, &mut new);
    let laptop = new.into_person(cloud, &ada);
    let helped = helper(&laptop.machine);
    let (since, sent, uploads) = (
        cloud.relay.mark(),
        front.sent().len(),
        front.uploads().len(),
    );

    deleted_at_terminal(&world, &ada);

    until(
        "the helper saying the account is deleted",
        || !deleted_lines(&helped.shown()).is_empty(),
        || helped.shown(),
    );
    thread::sleep(WATCH);
    assert_says_deleted_once(&helped.shown(), &laptop.machine, "the helper");
    let received = run(&laptop.machine, &["recv"]);
    assert_says_deleted_once(&received.shown(), &laptop.machine, "skald recv");
    let sending = send(&laptop.machine, channel, "after the deletion");
    assert_says_deleted_once(&sending.shown(), &laptop.machine, "skald send");

    assert_eq!(front.sent().len(), sent, "an envelope was sent");
    assert_eq!(front.uploads().len(), uploads, "a history upload was sent");
    assert_eq!(
        cloud.count_sent(since, SPACE_CHANGE),
        0,
        "a revision was sent"
    );
}
