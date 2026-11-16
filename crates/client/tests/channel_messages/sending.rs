//! Sending: one `mls` envelope per message that only the group's other devices read, sent again
//! with the same bytes after a lost answer, and nothing while the group has a member that is not
//! admitted or no other device.

use skald_crypto::MlsFraming;
use skald_proto::{ChannelId, Role, Scheme, SpaceId};

use crate::fixtures::{database, fronted, hex, kill_once, send, send_args, sent_after, strs};
use crate::front::{Answers, Sent};
use crate::keys;
use crate::kit::{Change, assert_members, create_space, latest, received, revise, with_channel};
use crate::played::without_key_packages;
use crate::talk::{
    ADA_HOST, BOB_HOST, NO_DEVICE, Talk, assert_read, commit_sent, delivered, messages_sent,
    one_message_sent, received_until,
};
use crate::world::{ACME, post_as};

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "the ledger migration starts at dawn";
const SECOND: &str = "ledger migration done, verify the totals";

/// Fails unless no envelope of `sent` is under `hpke`.
#[track_caller]
fn assert_no_hpke(sent: &[Sent]) {
    for one in sent {
        let header = &one.envelope.envelope().header;
        assert_eq!(header.scheme, Scheme::Mls, "an hpke envelope: {header:?}");
    }
}

/// Ada's device sends to a channel with Bob's and Carol's devices in its group: one `mls` envelope
/// without `recipients`, with `report` true, a PrivateMessage of the channel's group holding an
/// application message, and no `hpke` one. The server holds it once, and nothing in the database
/// holds the text. Bob's and Carol's devices each read it once: from Ada's device, with the person
/// address, `counter` 1 and the text in an A2A message whose `messageId` is the `msg_id`. Ada's
/// `skald recv` then shows `delivered` once for each of their devices.
#[test]
fn message_is_one_mls_envelope_that_each_other_device_of_the_group_reads_once() {
    let mut talk = Talk::new(
        &[4_136_061_001, 4_136_061_002, 4_136_061_003],
        &["ada-says-in-group", "bob-reads-group", "carol-reads-group"],
        false,
    );
    let from = talk.front.sent().len();

    let said = talk.send(FIRST);

    assert!(said.success, "{said:?}");
    assert_no_hpke(&sent_after(&talk.front, from));
    let sent = messages_sent(&talk.front, talk.channel, from);
    assert_eq!(sent.len(), 1, "not one message envelope: {sent:?}");
    let envelope = sent[0].envelope.envelope();
    let header = &envelope.header;
    assert_eq!(
        (header.space, header.channel, header.recipients.as_ref()),
        (talk.space.id, talk.channel, None),
        "{header:?}"
    );
    assert!(sent[0].report, "sent with report false: {header:?}");
    match MlsFraming::read(&envelope.payload) {
        Some(MlsFraming::Message { group, public, .. }) => {
            assert_eq!(group, talk.channel.as_bytes(), "not the channel's group");
            assert!(!public, "not a PrivateMessage");
        }
        other => panic!("no MLS message: {other:?}"),
    }
    let cloud = &talk.world.cloud;
    let bytes = hex(sent[0].envelope.bytes());
    assert_eq!(
        database(cloud).matches(&bytes).count(),
        1,
        "the server does not hold the envelope once"
    );
    assert!(
        !keys::holds(cloud.rows().concat().as_bytes(), FIRST.as_bytes()),
        "the database holds the text"
    );
    let msg_id = header.msg_id;
    let from_ada = talk.address(&talk.ada);
    let mut devices = Vec::new();
    for played in &mut talk.played {
        played.take_in(cloud);
        assert_read(
            played,
            talk.channel,
            (talk.ada.device, &from_ada),
            &[(msg_id, 1, FIRST)],
        );
        devices.push(played.device());
    }
    let shown = received_until(&talk.ada.machine, |shown| {
        devices
            .iter()
            .all(|device| delivered(shown, msg_id, *device) > 0)
    });
    for device in devices {
        assert_eq!(
            delivered(&shown, msg_id, device),
            1,
            "delivered to {device} not shown once: {shown}"
        );
    }
}

/// How a run loses the answer to Ada's message.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lost {
    /// The front loses each `stored`, and the run ends by itself.
    Stored,
    /// The front loses each `stored`, and the test kills the run once one is lost.
    Killed,
}

/// Ada's `skald send` of [`FIRST`] loses its `stored` as `lost` says. Her next `skald send`, with
/// the answers forwarded, sends it with the same bytes, then [`SECOND`]: the front saw two
/// distinct message envelopes, and Bob's device reads each once, with the counters 1 and 2.
fn sent_again_with_the_same_bytes(lost: Lost, ids: &[u64], logins: &[&str]) {
    let mut talk = Talk::new(ids, logins, true);
    let from = talk.front.sent().len();
    talk.front.answer(Answers::Lose);
    match lost {
        Lost::Stored => drop(talk.send(FIRST)),
        Lost::Killed => {
            let args = send_args(talk.channel, FIRST);
            let started = talk.ada.machine.start(&strs(&args));
            let front = &talk.front;
            kill_once(started, "a lost stored", || front.lost() > 0);
        }
    }
    assert!(talk.front.lost() > 0, "no stored was lost");
    talk.front.answer(Answers::Forward);

    let second = talk.send(SECOND);

    assert!(second.success, "{second:?}");
    let sent = messages_sent(&talk.front, talk.channel, from);
    assert_eq!(
        sent.len(),
        2,
        "not the left message with the same bytes, then the new one: {sent:?}"
    );
    let ids: Vec<_> = sent
        .iter()
        .map(|one| one.envelope.envelope().header.msg_id)
        .collect();
    talk.bob_reads_from_ada(&[(ids[0], 1, FIRST), (ids[1], 2, SECOND)]);
}

#[test]
fn message_whose_stored_was_lost_is_sent_again_with_the_same_bytes_and_read_once() {
    sent_again_with_the_same_bytes(
        Lost::Stored,
        &[4_136_061_101, 4_136_061_102],
        &["ada-loses-mls-stored", "bob-gets-mls-once"],
    );
}

#[test]
fn message_of_a_cli_killed_before_stored_is_sent_again_with_the_same_bytes_and_read_once() {
    sent_again_with_the_same_bytes(
        Lost::Killed,
        &[4_136_061_201, 4_136_061_202],
        &["ada-killed-mls-send", "bob-patient-mls"],
    );
}

/// The default channel of Ada's personal space, once her machine has signed it.
fn personal_default(cloud: &crate::cloud::Cloud, token: &str) -> ChannelId {
    let (status, body) = post_as(cloud, token, "/account/personal-space", None);
    assert_eq!(status, 200, "POST /account/personal-space: {body}");
    let space: SpaceId = body["space"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("a personal space: {body}"));
    let revision = latest(cloud, token, space);
    revision
        .body()
        .channels
        .first()
        .unwrap_or_else(|| panic!("no default channel in {space}"))
        .id
}

/// Bob, in `acme`, its default channel and a channel of Ada's, has no KeyPackage on the server, so
/// Ada's device is alone in both groups; Ada's personal space's default channel has no other
/// member. In each of these channels `skald send` fails, says no device can receive the message,
/// and sends no message, under `mls` or `hpke`.
#[test]
fn channel_with_no_other_device_in_its_group_sends_nothing_and_says_so() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_136_061_301, "ada-alone-in-group", ADA_HOST);
    let bob = world.sign_up(4_136_061_302, "bob-no-packages", BOB_HOST);
    without_key_packages(cloud, bob.device);
    let space = create_space(cloud, &ada, ACME);
    let channel = with_channel(cloud, space.id, &ada, &[(&bob, Role::Member)]);
    received(&ada.machine);
    let personal = personal_default(cloud, &ada.token);

    for of in [personal, space.default, channel] {
        let from = front.sent().len();

        let said = send(&ada.machine, of, FIRST);

        assert!(!said.success, "{of}: {said:?}");
        assert!(
            said.shown().to_lowercase().contains(NO_DEVICE),
            "{of}: does not say no device can receive it: {said:?}"
        );
        assert!(
            messages_sent(&front, of, from).is_empty(),
            "{of}: a message was sent"
        );
        assert_no_hpke(&sent_after(&front, from));
    }
}

/// With Bob's and Carol's devices in the channel's group, a revision Ada's device signed takes
/// Carol out of the channel. Ada's `skald send`, every `stored` lost, commits the Remove of Carol's
/// device, sends no message, fails and names Carol's device. With the answers forwarded, her next
/// `skald send` sends the Remove again and then its message, which Bob's device reads, and Carol's,
/// removed, does not.
#[test]
fn send_sends_nothing_while_the_remove_of_a_device_not_admitted_is_not_stored() {
    let mut talk = Talk::new(
        &[4_136_061_401, 4_136_061_402, 4_136_061_403],
        &["ada-removes-first", "bob-stays-member", "carol-taken-out"],
        true,
    );
    let cloud = &talk.world.cloud;
    let carol = talk.played[1].device();
    let previous = latest(cloud, &talk.ada.token, talk.space.id);
    let change = Change {
        leave: vec![(talk.channel, &talk.played[1].person)],
        ..Change::default()
    };
    revise(cloud, &previous, &talk.ada, &change);
    let from = talk.front.sent().len();
    talk.front.answer(Answers::Lose);

    let held = talk.send(FIRST);

    assert!(!held.success, "{held:?}");
    assert!(
        held.shown().contains(&carol.to_string()),
        "does not name Carol's device: {held:?}"
    );
    assert!(talk.front.lost() > 0, "no stored was lost: {held:?}");
    assert!(
        commit_sent(&talk.front, talk.channel, from),
        "no Remove sent: {held:?}"
    );
    assert!(
        messages_sent(&talk.front, talk.channel, from).is_empty(),
        "a message was sent with Carol's device in the group"
    );
    talk.front.answer(Answers::Forward);

    let said = talk.send(SECOND);

    assert!(said.success, "{said:?}");
    let msg_id = one_message_sent(&talk.front, talk.channel, from);
    talk.bob_reads_from_ada(&[(msg_id, 1, SECOND)]);
    let (ada, channel) = (talk.ada.device, talk.channel);
    let bob = &talk.played[0];
    assert_members(&bob.members(channel), &[ada, bob.device()]);
    let removed = &mut talk.played[1];
    removed.take_in(&talk.world.cloud);
    assert!(removed.removed_from(channel), "Carol's device not removed");
    assert!(
        removed.read_in(channel).is_empty(),
        "Carol's device read a message"
    );
}
