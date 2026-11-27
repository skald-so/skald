//! The account's other devices, once it is deleted: each command and the helper say once that the
//! server reports the account deleted and which state directory `skald delete-account` erases, and
//! send nothing.

use std::io;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use skald_client::{Client, DeviceContext, KeyStorage, MessageError};

use crate::deletion::{
    DELETE_ACCOUNT, USERNAME, assert_says_deleted_once, deleted_at_terminal, deleted_lines,
    hand_over, shared_with, until,
};
use crate::edits::SPACE_CHANGE;
use crate::fixtures::{fronted, send};
use crate::kit::{create_space, latest, received};
use crate::linking::{NewDevice, link};
use crate::machine::{Machine, Run};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::terminal::Terminal;
use crate::world::{ACME, run};

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

#[test]
fn sdk_receive_keeps_pending_messages_after_the_server_closes_a_deleted_account() {
    let (world, front) = fronted();
    let ada = world.sign_up(6_120_006_003, "ada-sdk-deletion", ADA_HOST);
    let channel = create_space(&world.cloud, &ada, ACME).default;
    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    link(&world.cloud, &ada, &mut new);
    let laptop = new.into_person(&world.cloud, &ada);
    received(&ada.machine);
    received(&laptop.machine);

    let client = |machine: &Machine| {
        Client::new(DeviceContext::one_shot(
            machine.state_dir(),
            KeyStorage::File,
            Duration::from_secs(30),
        ))
    };
    let mut sender = client(&ada.machine);
    let mut receiver = client(&laptop.machine);
    let first = sender.send(channel, "Keep this report pending.").unwrap();
    let declined = receiver.receive(|message| {
        assert_eq!(message.id, first);
        Err(io::Error::other("retry the handover"))
    });
    assert!(
        matches!(declined, Err(MessageError::Handover(_))),
        "{declined:?}"
    );

    let second = sender
        .send(channel, "This report witnesses catch-up.")
        .unwrap();
    front.watch_delivery(laptop.device, second);
    front.hold_end(true);
    let receiving = thread::spawn(move || {
        let mut calls = 0;
        let result = receiver.receive(|_| {
            calls += 1;
            Ok(())
        });
        (result, calls)
    });
    until(
        "the SDK acknowledging an entry while catch-up is held",
        || front.delivery().1 || receiving.is_finished(),
        String::new,
    );
    assert!(front.delivery().1 && !receiving.is_finished());

    let deleting = Terminal::start(&ada.machine, &[DELETE_ACCOUNT]);
    until(
        "the deletion confirmation",
        || deleting.shown().contains(USERNAME),
        String::new,
    );
    // Permit the deletion and its WebSocket close. Block later HTTP probes so only the close
    // can teach the SDK about the deletion.
    let relay = &world.cloud.relay;
    for method in ["GET ", "POST ", "PUT "] {
        relay.block(method);
    }
    let since = relay.mark();
    deleting.type_line(ada.handle.as_str());
    let deleted = deleting.finish();
    assert!(deleted.success, "{}", deleted.text);

    let (result, calls) = receiving.join().expect("the SDK receive finished");
    assert!(relay.answered_since(since).contains("account-deleted"));
    assert_eq!(calls, 0, "the SDK handed over a message after deletion");
    assert!(
        matches!(result, Err(MessageError::AccountDeleted(_))),
        "{result:?}"
    );
    assert!(laptop.machine.state_dir().join("account-deleted").is_file());

    let reopened = client(&laptop.machine).receive(|_| panic!("handed over after reopening"));
    assert!(
        matches!(reopened, Err(MessageError::AccountDeleted(_))),
        "{reopened:?}"
    );
}
