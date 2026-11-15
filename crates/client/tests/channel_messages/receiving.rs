//! Receiving: `skald send` and `skald recv` decrypt each message in its channel's group in mailbox
//! order, past Commits, and drop what a proxy puts into catch-up that fails a check, or repeats.

use skald_proto::{DeviceId, MessageId, Role};

use crate::fixtures::cert_of;
use crate::kit::{Change, latest, received, revise, warnings_naming};
use crate::machine::Finished;
use crate::talk::{
    BOB_HOST, CAROL_HOST, Line, Talk, assert_shows_once, encrypted, one_message_sent, sealed,
    stored, without_key_packages,
};
use crate::world::{now, warnings};

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
