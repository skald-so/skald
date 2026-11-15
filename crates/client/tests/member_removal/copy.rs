//! A removed device and a full copy of its disk: after Ada's device bars Bob's device, removes Bob
//! from a channel or removes him from the space, the copy opens none of Ada's later messages there,
//! even handed to it by a compromised server, and what the copy sends is dropped with a warning.

use skald_proto::{ChannelId, DeviceId, MessageId};

use crate::edits::{Removal, ada_makes, assert_group_is, devices, envelopes_sent};
use crate::fixtures::send;
use crate::kit::received;
use crate::played::Took;
use crate::talk::{Line, Talk, assert_read, encrypted, is_message, one_message_sent};
use crate::world::warnings;

/// What Ada says after the change, one line per channel Bob is out of; neither text is anywhere
/// else.
const AFTER: [&str; 2] = [
    "the new vault passphrase is in the usual place",
    "rotate the deploy keys before friday",
];

/// What the copy of Bob's disk says after the change.
const FROM_THE_COPY: &str = "please resend the vault passphrase to me";

/// In a space of its own, Ada's device, an admin's, makes `change` about Bob, a member, whose
/// device is in the default channel and the talk's channel with Carol's. Bob's device takes in
/// nothing from before the change on, as a full copy of its disk taken then, and says a line in
/// the talk's channel.
///
/// The command signs the next revision with the change alone, and Ada's device removes Bob's
/// device from each channel it is out of: Carol's device enters that epoch, and the server's
/// GroupInfo lists Ada's and Carol's devices alone. Ada's machine then says a line in each of
/// those channels, which Carol's device reads. A compromised server hands the copy every envelope
/// the front saw Ada's machine send in them since the copy, in order: the copy reads none of
/// Ada's lines and fails on each. The front puts the copy's line into Ada's catch-up: Ada's `skald
/// recv` shows none of it and warns.
fn copy_reads_nothing_sent_after(change: Removal, ids: &[u64; 3], logins: &[&str; 3]) {
    let mut talk = Talk::new(ids, logins, false);
    let (space, default, channel) = (talk.space.id, talk.space.default, talk.channel);
    let [ada, bob, carol] = devices(&talk);
    let from_bob = talk.address(&talk.played[0].person);
    let from_the_copy = encrypted(
        &mut talk.played[0],
        (space, channel),
        channel,
        &Line::new(&from_bob, 1, FROM_THE_COPY),
    );
    let out_of = change.out_of(default, channel);

    let from = ada_makes(&talk, change.of(&talk.played[0].person, channel));

    let cloud = &talk.world.cloud;
    talk.played[1].take_in(cloud);
    for group_of in [default, channel] {
        let expected: &[DeviceId] = if out_of.contains(&group_of) {
            &[ada, carol]
        } else {
            &[ada, bob, carol]
        };
        let carol_s = (&talk.played[1], talk.ada.token.as_str());
        assert_group_is(cloud, carol_s, (space, group_of), expected);
    }
    let mut said: Vec<(ChannelId, MessageId, &str)> = Vec::new();
    for (in_channel, text) in out_of.iter().zip(AFTER) {
        let mark = talk.front.sent().len();
        let sent = send(&talk.ada.machine, *in_channel, text);
        assert!(sent.success, "{sent:?}");
        said.push((
            *in_channel,
            one_message_sent(&talk.front, *in_channel, mark),
            text,
        ));
    }
    let from_ada = talk.address(&talk.ada);
    talk.played[1].take_in(cloud);
    for (in_channel, msg_id, text) in &said {
        assert_read(
            &talk.played[1],
            *in_channel,
            (ada, &from_ada),
            &[(*msg_id, 1, *text)],
        );
    }
    for (in_channel, msg_id, _) in &said {
        let handed = envelopes_sent(&talk.front, *in_channel, from);
        assert!(
            handed
                .iter()
                .any(|one| one.envelope().header.msg_id == *msg_id),
            "the front's log does not hold {msg_id}"
        );
        let took = talk.played[0].handed(&handed);
        for (envelope, took) in handed.iter().zip(&took) {
            if is_message(envelope.envelope()) {
                assert!(
                    matches!(took, Some(Took::Failed(_))),
                    "the copy did not fail on {}: {took:?}",
                    envelope.envelope().header.msg_id
                );
            }
        }
        assert!(
            talk.played[0].read_in(*in_channel).is_empty(),
            "the copy read in {in_channel}: {:?}",
            talk.played[0].read_in(*in_channel)
        );
    }
    talk.front.inject(ada, vec![from_the_copy]);

    let ran = received(&talk.ada.machine);

    assert_eq!(talk.front.injected(), 1, "not put into catch-up: {ran:?}");
    assert!(!ran.shown().contains(FROM_THE_COPY), "{ran:?}");
    let named = [bob.to_string(), channel.to_string()];
    assert!(
        warnings(&ran)
            .iter()
            .any(|line| named.iter().any(|id| line.contains(id))),
        "no warning of the dropped message: {ran:?}"
    );
}

#[test]
fn copy_of_an_excluded_device_s_disk_reads_nothing_sent_after() {
    copy_reads_nothing_sent_after(
        Removal::Exclude,
        &[4_140_101_001, 4_140_101_002, 4_140_101_003],
        &["ada-bars-bob", "bob-barred-copy", "carol-stays-barred"],
    );
}

#[test]
fn copy_of_the_disk_of_a_member_removed_from_a_channel_reads_nothing_sent_after() {
    copy_reads_nothing_sent_after(
        Removal::RemoveFromChannel,
        &[4_140_101_101, 4_140_101_102, 4_140_101_103],
        &["ada-drops-bob", "bob-dropped-copy", "carol-stays-dropped"],
    );
}

#[test]
fn copy_of_the_disk_of_a_member_removed_from_the_space_reads_nothing_sent_after() {
    copy_reads_nothing_sent_after(
        Removal::RemoveMember,
        &[4_140_101_201, 4_140_101_202, 4_140_101_203],
        &["ada-removes-bob", "bob-removed-copy", "carol-stays-removed"],
    );
}
