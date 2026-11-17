//! Entries that fail the checks of "History entry" show nothing: the history command drops each
//! with a warning naming the sending device and the check.

use skald_crypto::SigningKey;
use skald_proto::{Cbor as _, ChannelId, MessageId};

use crate::chain::key_of;
use crate::joins::{intruder, intruder_joins};
use crate::kit::received;
use crate::reading::{Duo, assert_history, body_of, sealed_as_is, uploads_entry, warned_of};
use crate::talk::Line;
use crate::world::HOST;

/// The texts of the entries the checks drop; none is anywhere else.
const OTHER_CHANNEL: &str = "this body names another channel";
const OTHER_ACCOUNT: &str = "this is from an agent of another account";
const OTHER_MESSAGE_ID: &str = "this a2a message has another messageId";
const OTHER_SIGNER: &str = "this is signed by another device";
const INTRUDER: &str = "the server's own device writes this";

/// The agent of Ada's account that Bob's device claims to send as.
const CLAUDE: &str = "claude-k2d7rw";

/// Bob says a line in epoch 1. Bob's device, a compromised one, then writes four entries of epoch
/// 1 under its key and uploads them, each of a message of its own: one whose body names the space's
/// default channel, one with `from` an agent of Ada's account, one whose A2A `messageId` is not
/// its `msg_id`, and one in the name of Bob's device signed by Ada's. Ada's `skald history` shows
/// Bob's line alone, and a warning naming Bob's device for each of the four, saying the check:
/// `channel`, `from`, `messageId` and `signature`.
#[test]
fn entries_that_fail_a_check_are_dropped_with_a_warning_naming_the_device_and_the_check() {
    let mut duo = Duo::new(
        &[4_166_008_001, 4_166_008_002],
        &["ada-checks-entries", "bob-device-compromised"],
    );
    let (good, ..) = duo.bob_says(&duo.bob_s(1, "bob's honest line"));
    let Duo { talk, at, from_bob } = &mut duo;
    let agent_of_ada = format!("{}/{}/{CLAUDE}@{HOST}", talk.space.name, talk.ada.handle);
    let mut other_message_id = Line::new(from_bob, 4, OTHER_MESSAGE_ID);
    other_message_id.message_id = Some(MessageId::generate());
    let bob = &talk.played[0];
    let bad: [(Line, ChannelId, &SigningKey); 4] = [
        (
            Line::new(from_bob, 2, OTHER_CHANNEL),
            talk.space.default,
            &bob.person.key,
        ),
        (
            Line::new(&agent_of_ada, 3, OTHER_ACCOUNT),
            at.1,
            &bob.person.key,
        ),
        (other_message_id, at.1, &bob.person.key),
        (Line::new(from_bob, 5, OTHER_SIGNER), at.1, &talk.ada.key),
    ];
    for (line, named, signer) in &bad {
        let msg_id = MessageId::generate();
        let body = body_of(bob.device(), *named, (msg_id, line));
        let entry = sealed_as_is(key_of(bob, at.1, 1), (at.1, 1), &body, signer);
        uploads_entry(
            &talk.world.cloud,
            &bob.person,
            *at,
            (msg_id, &entry.to_cbor()),
        );
    }

    let shown = duo.ada_s_history();

    let dropped = [OTHER_CHANNEL, OTHER_ACCOUNT, OTHER_MESSAGE_ID, OTHER_SIGNER];
    assert_history(&shown, duo.at.1, &[good], &dropped);
    let warned = warned_of(&shown, duo.talk.played[0].device());
    assert!(
        warned.len() >= dropped.len(),
        "not a warning per entry: {shown:?}"
    );
    for check in ["channel", "from", "messageid", "signature"] {
        assert!(
            warned.iter().any(|line| line.contains(check)),
            "no warning says {check:?}: {warned:?}"
        );
    }
}

/// Bob says a line in epoch 1. A device the server lets in on Bob's account, which the members do
/// not admit, joins the channel's group by itself: epoch 2. It writes an entry under that epoch's
/// key, signed by itself, and uploads it. Ada's run removes the device; her `skald history` shows
/// Bob's line and not the device's, and warns naming the device.
#[test]
fn entry_a_device_the_server_joined_writes_in_its_epoch_is_dropped_with_a_warning() {
    let mut duo = Duo::new(
        &[4_166_009_001, 4_166_009_002],
        &["ada-spots-server-entry", "bob-account-borrowed"],
    );
    let (good, ..) = duo.bob_says(&duo.bob_s(1, "bob before the intruder"));
    let line = duo.bob_s(2, INTRUDER);
    let Duo { talk, at, .. } = &mut duo;
    let (cloud, bob) = (&talk.world.cloud, &mut talk.played[0]);
    let intruder = intruder(cloud, &bob.person, "bob-server-device");
    intruder_joins(cloud, &intruder, *at);
    bob.take_in(cloud);
    let msg_id = MessageId::generate();
    let body = body_of(intruder.id(), at.1, (msg_id, &line));
    let entry = key_of(bob, at.1, 2)
        .seal_entry(2, &body, &intruder.person.key)
        .expect("an entry");
    uploads_entry(cloud, &intruder.person, *at, (msg_id, &entry.to_cbor()));
    received(&talk.ada.machine);

    let shown = duo.ada_s_history();

    assert_history(&shown, duo.at.1, &[good], &[INTRUDER]);
    assert!(
        !warned_of(&shown, intruder.id()).is_empty(),
        "no warning naming the intruder: {shown:?}"
    );
}
