//! Receiving: `skald send` and `skald recv` decrypt each message in its channel's group in mailbox
//! order, past Commits, and drop what a proxy puts into catch-up that fails a check, or repeats. A
//! message waits for a Commit before it that waits for a later run, and for the device's Welcome.

use skald_proto::{ChannelId, DeviceId, MessageId, Role, SpaceId};

use crate::fixtures::{cert_of, send};
use crate::kit::{Change, create_space, issue_channel, latest, received, revise, warnings_naming};
use crate::machine::Finished;
use crate::played::take_key_package;
use crate::talk::{
    BOB_HOST, CAROL_HOST, Line, Talk, assert_shows_once, encrypted, one_message_sent,
    revisions_fail_once, sealed, stored, without_key_packages,
};
use crate::world::{Person, now, warnings};

/// The texts the tests send; none is anywhere else.
const BOB_SAYS: [&str; 5] = [
    "invoice batch one is out",
    "invoice batch two is out",
    "invoice batch three is out",
    "invoice batch four is out",
    "invoice batch five is out",
];
const ADA_SAYS: &str = "thanks, closing the quarter now";
const OTHER_GROUP: &str = "this came from another group";
const NOT_HIS: &str = "signed by bob as somebody else";
const WRONG_ID: &str = "the a2a id does not match";
const OUTSIDER: &str = "a word from outside the group";
const AFTER_COMMIT: &str = "said in the epoch after the commit";
const BEFORE_WELCOME: &str = "said before the welcome came";
const NOT_ADMITTED: &str = "said in a channel ada is not in";

/// Bob's device sends five messages to the channel with an empty Commit of its own between each
/// two, four Commits in all. Ada's `skald send` goes through them first and shows none of them; her
/// message, sent in the group's current epoch, is one Bob's device reads. Her `skald recv` then
/// shows each of Bob's messages once, with the channel, Bob's address and his device's name.
#[test]
fn send_goes_past_commits_with_messages_between_them_and_recv_then_shows_each_once() {
    let mut talk = Talk::new(
        &[4_136_062_001, 4_136_062_002],
        &["ada-catches-up-mls", "bob-commits-often"],
        false,
    );
    let at = talk.at();
    let cloud = &talk.world.cloud;
    let from_bob = talk.address(&talk.played[0].person);
    let bob = &mut talk.played[0];
    for (counter, text) in (1..).zip(BOB_SAYS) {
        if counter > 1 {
            let empty = (bob.mls)
                .commit_empty(at.1, b"", now())
                .expect("an empty Commit");
            bob.commits(cloud, at, &empty);
        }
        let envelope = encrypted(bob, at, at.1, &Line::new(&from_bob, counter, text));
        stored(cloud, bob, &envelope);
    }
    let from = talk.front.sent().len();

    let said = talk.send(ADA_SAYS);

    assert!(said.success, "{said:?}");
    let shown = said.shown();
    for text in BOB_SAYS {
        assert!(!shown.contains(text), "skald send shows {text:?}: {said:?}");
    }
    let msg_id = one_message_sent(&talk.front, talk.channel, from);
    talk.bob_reads_from_ada(&[(msg_id, 1, ADA_SAYS)]);

    let later = received(&talk.ada.machine).shown();

    for text in BOB_SAYS {
        assert_shows_once(&later, text, (talk.channel, &from_bob, BOB_HOST));
    }
}

/// How many warnings of `ran` name `device`, or, without one, how many warnings it has.
fn warned(ran: &Finished, device: Option<DeviceId>) -> usize {
    match device {
        Some(device) => warnings_naming(ran, device).len(),
        None => warnings(ran).len(),
    }
}

/// Bob's device and Carol's account are in the channel, Carol's device, without KeyPackages, not in
/// its group. Bob's device sends a message, which a proxy repeats under another `seq` in Ada's
/// catch-up: Ada's `skald recv` shows it once. Then, one `skald recv` each, the proxy puts into
/// Ada's catch-up: an envelope of the channel encrypted in the default channel's group; Bob's
/// content with `from` Carol's address; Bob's content whose `messageId` is not its `msg_id`; and
/// `hpke` content from Carol's device. Each run drops it, shows none of its text, and warns more
/// than a run with nothing put in: naming the sending device when the payload authenticates it.
#[test]
fn envelopes_a_proxy_puts_into_catch_up_are_dropped_with_a_warning_and_a_repeat_shows_once() {
    let mut talk = Talk::new(
        &[4_136_062_101, 4_136_062_102],
        &["ada-drops-mls", "bob-twisted-mls"],
        false,
    );
    let carol = talk
        .world
        .sign_up(4_136_062_103, "carol-outside-group", CAROL_HOST);
    let cloud = &talk.world.cloud;
    without_key_packages(cloud, carol.device);
    let previous = latest(cloud, &talk.ada.token, talk.space.id);
    let change = Change {
        members: vec![(&carol, Role::Member)],
        join: vec![(talk.channel, &carol)],
        ..Change::default()
    };
    revise(cloud, &previous, &talk.ada, &change);
    received(&talk.ada.machine);
    let (at, ada) = (talk.at(), talk.ada.device);
    let (from_bob, from_carol) = (talk.address(&talk.played[0].person), talk.address(&carol));
    let bob = &mut talk.played[0];
    let message = encrypted(bob, at, at.1, &Line::new(&from_bob, 1, BOB_SAYS[0]));
    stored(cloud, bob, &message);
    talk.front.inject(ada, vec![message]);

    let ran = received(&talk.ada.machine);

    assert_eq!(talk.front.injected(), 1, "not repeated: {ran:?}");
    assert_shows_once(&ran.shown(), BOB_SAYS[0], (at.1, &from_bob, BOB_HOST));
    // A run with nothing put in: the warnings every run shows.
    let quiet = received(&talk.ada.machine);
    let bob = &mut talk.played[0];
    let other_group = encrypted(
        bob,
        at,
        talk.space.default,
        &Line::new(&from_bob, 2, OTHER_GROUP),
    );
    let not_his = encrypted(bob, at, at.1, &Line::new(&from_carol, 3, NOT_HIS));
    let mut wrong = Line::new(&from_bob, 4, WRONG_ID);
    wrong.message_id = Some(MessageId::generate());
    let wrong_id = encrypted(bob, at, at.1, &wrong);
    let ada_key = cert_of(cloud, &talk.ada).body().hpke;
    let outsider = sealed(
        &carol,
        at,
        (ada, &ada_key),
        &Line::new(&from_carol, 1, OUTSIDER),
    );
    let bob = bob.device();
    let drops = [
        (other_group, OTHER_GROUP, None),
        (not_his, NOT_HIS, Some(bob)),
        (wrong_id, WRONG_ID, Some(bob)),
        (outsider, OUTSIDER, Some(carol.device)),
    ];
    for (count, (envelope, text, sender)) in (2..).zip(drops) {
        talk.front.inject(ada, vec![envelope]);

        let ran = received(&talk.ada.machine);

        assert_eq!(talk.front.injected(), count, "{text:?} not put in: {ran:?}");
        assert!(!ran.shown().contains(text), "{text:?} shown: {ran:?}");
        assert!(
            warned(&ran, sender) > warned(&quiet, sender),
            "no warning of {text:?}, naming {sender:?}: {ran:?}"
        );
    }
}

/// Bob's device commits an empty Commit, then sends a message in the new epoch. Ada's `skald send`
/// to her other space goes through her mailbox while the first request for `acme`'s revisions
/// fails: the Commit waits for a later run, and so does the message after it. Her `skald recv` then
/// shows the message once.
#[test]
fn message_after_a_commit_that_waits_for_a_later_run_waits_with_it() {
    let mut talk = Talk::new(
        &[4_136_062_201, 4_136_062_202],
        &["ada-sync-fails-once", "bob-commits-then-says"],
        false,
    );
    let cloud = &talk.world.cloud;
    let other = create_space(cloud, &talk.ada, "beta");
    let at = talk.at();
    let from_bob = talk.address(&talk.played[0].person);
    let bob = &mut talk.played[0];
    let empty = (bob.mls)
        .commit_empty(at.1, b"", now())
        .expect("an empty Commit");
    bob.commits(cloud, at, &empty);
    let message = encrypted(bob, at, at.1, &Line::new(&from_bob, 1, AFTER_COMMIT));
    stored(cloud, bob, &message);
    revisions_fail_once(cloud, at.0);

    // No other device is in the group of the other space's default channel: nothing is sent.
    let said = send(&talk.ada.machine, other.default, ADA_SAYS);

    assert!(!said.shown().contains(AFTER_COMMIT), "{said:?}");
    let shown = received(&talk.ada.machine).shown();
    assert_shows_once(&shown, AFTER_COMMIT, (at.1, &from_bob, BOB_HOST));
}

/// Bob's device forms the group of a new channel of the three of them, adding Ada's device and
/// Carol's, and its Commit is stored; it sends the Welcome to Carol's device alone, as if killed
/// before it sent Ada's. Carol's device sends a message in the group. Ada's `skald recv` shows none
/// of it; once Bob's device sends Ada's device the Welcome, her next `skald recv` shows the
/// message once.
#[test]
fn message_that_comes_before_the_welcome_waits_for_it() {
    let mut talk = Talk::new(
        &[4_136_062_301, 4_136_062_302, 4_136_062_303],
        &["ada-welcomed-late", "bob-adds-late", "carol-says-early"],
        false,
    );
    let cloud = &talk.world.cloud;
    let ada = talk.ada.device;
    let (bob, carol) = (&talk.played[0].person, &talk.played[1].person);
    let at = new_channel(&talk, &[&talk.ada, bob, carol]);
    let packages = [ada, carol.device].map(|device| take_key_package(cloud, &bob.token, device));
    let certs = [cert_of(cloud, &talk.ada), cert_of(cloud, carol)];
    let (carol_device, from_carol) = (carol.device, talk.address(carol));
    let adder = &mut talk.played[0];
    adder.mls.create_group(at.1, now()).expect("a group");
    let commit = (adder.mls)
        .add_devices(at.1, &packages, &certs, now())
        .expect("the Commit that adds them");
    let welcome = adder.commits(cloud, at, &commit).expect("their Welcome");
    adder.uploads_group_info(cloud, at);
    adder.stores(cloud, at, &welcome, &[carol_device]);
    let channel = at.1;
    let carol = &mut talk.played[1];
    carol.take_in(cloud);
    assert_eq!(carol.joined_to(channel).len(), 1, "Carol's device joins");
    let message = encrypted(
        carol,
        at,
        channel,
        &Line::new(&from_carol, 1, BEFORE_WELCOME),
    );
    stored(cloud, carol, &message);

    let early = received(&talk.ada.machine);

    assert!(!early.shown().contains(BEFORE_WELCOME), "{early:?}");
    talk.played[0].stores(cloud, at, &welcome, &[ada]);
    let shown = received(&talk.ada.machine).shown();
    assert_shows_once(&shown, BEFORE_WELCOME, (channel, &from_carol, CAROL_HOST));
}

/// A proxy puts into Ada's catch-up Bob's message in a channel of Bob's alone, whose group holds
/// his device alone: Ada's `skald recv`, with no group of the channel, which does not admit her
/// device, drops it with a warning and shows none of it.
#[test]
fn message_of_a_channel_that_does_not_admit_the_device_is_dropped_without_a_group() {
    let mut talk = Talk::new(
        &[4_136_062_401, 4_136_062_402],
        &["ada-left-out", "bob-alone-in-channel"],
        false,
    );
    let cloud = &talk.world.cloud;
    let ada = talk.ada.device;
    let at = new_channel(&talk, &[&talk.played[0].person]);
    let channel = at.1;
    let quiet = received(&talk.ada.machine);
    let from_bob = talk.address(&talk.played[0].person);
    let bob = &mut talk.played[0];
    bob.creates_alone(cloud, at);
    let message = encrypted(bob, at, channel, &Line::new(&from_bob, 1, NOT_ADMITTED));
    talk.front.inject(ada, vec![message]);

    let ran = received(&talk.ada.machine);

    assert_eq!(talk.front.injected(), 1, "not put in: {ran:?}");
    assert!(!ran.shown().contains(NOT_ADMITTED), "{ran:?}");
    assert!(
        warned(&ran, None) > warned(&quiet, None),
        "no warning: {ran:?}"
    );
}

/// A new channel of `people` in the talk's space, which a fixture revision Ada's device signs adds;
/// gives the space's ID and the channel's.
fn new_channel(talk: &Talk, people: &[&Person]) -> (SpaceId, ChannelId) {
    let (cloud, space, ada) = (talk.cloud(), talk.space.id, &talk.ada);
    let channel = issue_channel(cloud, &ada.token, space);
    let change = Change {
        channels: vec![(channel, people.to_vec())],
        ..Change::default()
    };
    revise(cloud, &latest(cloud, &ada.token, space), ada, &change);
    (space, channel)
}
