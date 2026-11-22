//! A device of the account that runs no helper when the account is deleted learns it from the
//! server once its session is refused: each command that would reach the server then says so and
//! sends nothing, and its `skald delete-account` deletes its local state.

use crate::deletion::{
    AtGrant, DELETE_ACCOUNT, Held, assert_keeps_nothing, assert_says_deleted_once,
    delete_at_terminal, deleted_at_terminal, deleted_lines, hand_over, shared_with,
};
use crate::edits::SPACE_CHANGE;
use crate::fixtures::{fronted, send};
use crate::kit::latest;
use crate::linking::{NewDevice, link};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::terminal::Terminal;
use crate::world::run;

/// The host name of Ada's second machine.
const LAPTOP: &str = "ada-laptop-offline";

/// Ada's first machine created `acme`, with Bob a member, and handed her admin rights there to
/// Bob; her second machine is linked with a link code and runs no helper. Once her first machine's
/// `skald delete-account` has deleted her account, the second machine's `skald recv`, its `skald
/// send` to `acme`'s default channel and its `skald link` at a terminal each say once that the
/// server reports the account deleted and name the machine's state directory; nothing sends an
/// envelope, a history upload or a revision. Its `skald delete-account` at a terminal then asks
/// nothing, signs in to nothing, says the account is deleted, and its state directory keeps nothing
/// of the account.
#[test]
fn a_device_off_during_the_deletion_says_it_is_deleted_and_sends_nothing() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_008_001, "ada-leaves-a-laptop-off", ADA_HOST);
    let bob = world.sign_up(6_120_008_002, "bob-keeps-acme-on", BOB_HOST);
    let space = shared_with(cloud, &ada, &bob);
    hand_over(&ada, &bob);
    let channel = latest(cloud, &bob.token, space).body().channels[0].id;
    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    link(cloud, &ada, &mut new);
    let laptop = new.into_person(cloud, &ada);
    let held = Held::of(cloud, &laptop);
    let (since, sent, uploads) = (
        cloud.relay.mark(),
        front.sent().len(),
        front.uploads().len(),
    );

    deleted_at_terminal(&world, &ada);

    let received = run(&laptop.machine, &["recv"]);
    assert_says_deleted_once(&received.shown(), &laptop.machine, "skald recv");
    let sending = send(&laptop.machine, channel, "after the deletion");
    assert_says_deleted_once(&sending.shown(), &laptop.machine, "skald send");
    let linking = Terminal::start(&laptop.machine, &["link"]).finish();
    assert!(!linking.success, "{}", linking.text);
    assert_says_deleted_once(&linking.text, &laptop.machine, "skald link");
    assert_eq!(front.sent().len(), sent, "an envelope was sent");
    assert_eq!(front.uploads().len(), uploads, "a history upload was sent");
    assert_eq!(
        cloud.count_sent(since, SPACE_CHANGE),
        0,
        "a revision was sent"
    );

    let deletion = delete_at_terminal(
        &world,
        &laptop.machine,
        laptop.handle.as_str(),
        AtGrant::Refuse,
    );
    let text = &deletion.shown.text;
    assert!(deletion.shown.success, "{text}");
    assert_eq!(deletion.asked, 0, "skald {DELETE_ACCOUNT} asked: {text}");
    assert_eq!(deleted_lines(text).len(), 1, "{text}");
    assert_keeps_nothing(cloud, &laptop, &held);
}
