//! The baseline under `mls`: a device added to a channel's group after messages were sent there
//! counts from the first message it gets from each sending device.

use skald_proto::Role;

use crate::fixtures::{cert_of, fronted, person_address};
use crate::gaps::{assert_none_reported, assert_reported_once, marked, msg_id_of};
use crate::kit::{Change, create_space, issue_channel, latest, received, revise};
use crate::played::{Played, take_key_package};
use crate::talk::{ADA_HOST, BOB_HOST, Line, assert_shows_once, encrypted, stored};
use crate::world::{ACME, now};

/// The texts the tests send; none is anywhere else.
const BEFORE: [&str; 3] = [
    "draft one of the pitch deck",
    "draft two of the pitch deck",
    "draft three of the pitch deck",
];
const AFTER: [&str; 3] = [
    "welcome aboard, final deck attached",
    "this one gets lost on the way",
    "rehearsal is on thursday",
];

/// A revision Ada's device signs makes Bob an admin of `acme` and adds a channel with Bob alone,
/// whose group Bob's device creates; it sends three messages there. A revision Bob's device signs
/// adds Ada to the channel, and Bob's device adds Ada's device to the group and sends it the
/// Welcome, then sends a fourth message. Ada's `skald recv` shows the fourth, unmarked, none of the
/// three before, and reports nothing missing. Bob's device then sends a fifth, which never reaches
/// the server, and a sixth: Ada's next run shows the sixth marked and reports messages missing
/// once, naming the channel and Bob's device.
#[test]
fn device_added_to_the_channel_after_messages_reports_none_of_them_missing() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_137_073_001, "ada-added-later", ADA_HOST);
    let mut bob = Played::new(
        cloud,
        world.sign_up(4_137_073_002, "bob-talks-first", BOB_HOST),
    );
    let space = create_space(cloud, &ada, ACME);
    let channel = issue_channel(cloud, &ada.token, space.id);
    let previous = latest(cloud, &ada.token, space.id);
    let change = Change {
        members: vec![(&bob.person, Role::Admin)],
        channels: vec![(channel, vec![&bob.person])],
        ..Change::default()
    };
    revise(cloud, &previous, &ada, &change);
    received(&ada.machine);
    bob.take_in(cloud);
    let at = (space.id, channel);
    bob.creates_alone(cloud, at);
    bob.uploads_group_info(cloud, at);
    let from_bob = person_address(&space.name, &bob.person);
    let says = |bob: &mut Played, counter: u64, text: &str| {
        encrypted(bob, at, channel, &Line::new(&from_bob, counter, text))
    };
    for (counter, text) in (1..).zip(BEFORE) {
        let envelope = says(&mut bob, counter, text);
        stored(cloud, &bob, &envelope);
    }
    let previous = latest(cloud, &ada.token, space.id);
    let change = Change {
        join: vec![(channel, &ada)],
        ..Change::default()
    };
    revise(cloud, &previous, &bob.person, &change);
    let package = take_key_package(cloud, &bob.person.token, ada.device);
    let commit = (bob.mls)
        .add_devices(channel, &[package], &[cert_of(cloud, &ada)], now())
        .expect("a Commit adding Ada's device");
    let welcome = bob.commits(cloud, at, &commit).expect("a Welcome");
    bob.uploads_group_info(cloud, at);
    bob.stores(cloud, at, &welcome, &[ada.device]);
    let fourth = says(&mut bob, 4, AFTER[0]);
    stored(cloud, &bob, &fourth);

    let shown = received(&ada.machine).shown();

    assert_shows_once(&shown, AFTER[0], (channel, &from_bob, BOB_HOST));
    for text in BEFORE {
        assert!(!shown.contains(text), "{text:?} shown: {shown}");
    }
    assert!(
        !marked(&shown, msg_id_of(&fourth)),
        "the first message after joining marked: {shown}"
    );
    assert_none_reported(&shown);
    says(&mut bob, 5, AFTER[1]);
    let sixth = says(&mut bob, 6, AFTER[2]);
    stored(cloud, &bob, &sixth);

    let next = received(&ada.machine).shown();

    assert_shows_once(&next, AFTER[2], (channel, &from_bob, BOB_HOST));
    assert!(!next.contains(AFTER[1]), "the dropped message: {next}");
    assert!(
        marked(&next, msg_id_of(&sixth)),
        "the sixth message not marked: {next}"
    );
    assert_reported_once(&next, channel, bob.device());
}
