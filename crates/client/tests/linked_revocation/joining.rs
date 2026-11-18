//! Joining a channel whose group holds only revoked devices: the valid device outside it joins by
//! itself, removes them, then sends once another device is in the group, never in its join epoch;
//! a device linked later joins from its GroupInfo and reads its messages from the history. With the
//! `stored` of the Remove lost, the next run removes them once, and the device sends nothing
//! meanwhile.

use crate::devices::second_machine;
use crate::fixtures::{retrying_for_the_timeout, send};
use crate::kit::{assert_members, group, received};
use crate::linked::{FIFTH_HOST, Kind, SIXTH_HOST, revoked_group, revoked_talk, sent_kinds};
use crate::reading::{assert_history, sends, shown_history};
use crate::talk::{NO_DEVICE, messages_sent, one_message_sent};
use crate::world::ACME;

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "the new desk takes over the channel";
const SECOND: &str = "the new desk reports in";

/// The default channel of `acme` holds Ada's first and second machines' devices, both revoked, at
/// epoch e; her fourth machine's device, valid, is outside it. The fourth machine's `skald send`
/// sends, in the channel, an external Commit of epoch e, then a Commit of epoch e + 1, and no
/// message, as no other device is then in the group: it fails and says no device can receive it.
/// The server's group is at e + 2 with the fourth machine's device alone, which signs its
/// GroupInfo. Ada's fifth machine, linked by her fourth, joins the channel by itself at e + 3; the
/// fourth's next `skald send` sends its message in epoch e + 3. Her sixth machine, linked by her
/// fourth after that, joins the channel by itself at e + 4, and, once the fourth has run since, the
/// sixth's `skald history` shows the message.
#[test]
fn device_joins_a_group_of_revoked_devices_by_itself_removes_them_then_sends() {
    let revoked = revoked_group((4_840_003_001, "ada-starts-over"), false);
    let (fourth, e) = (&revoked.fourth, revoked.epoch);
    let channel = revoked.space.default;
    revoked
        .group()
        .assert_is(e, &[revoked.ada.device, revoked.second.device]);
    let from = revoked.front.sent().len();

    let alone = send(&fourth.machine, channel, FIRST);

    assert!(!alone.success, "{alone:?}");
    assert!(
        alone.shown().to_lowercase().contains(NO_DEVICE),
        "does not say no device can receive it: {alone:?}"
    );
    assert_eq!(
        sent_kinds(&revoked.front, channel, from),
        [(Kind::Joining, e), (Kind::Commit, e + 1)],
        "not a join, then a Remove"
    );
    let held = revoked.group();
    held.assert_is(e + 2, &[fourth.device]);
    assert_eq!(held.signer, fourth.device, "the GroupInfo's signer");
    let fifth = second_machine(&revoked.world, fourth, FIFTH_HOST);
    received(&fifth.machine);
    revoked
        .group()
        .assert_is(e + 3, &[fourth.device, fifth.device]);
    let from = revoked.front.sent().len();

    let said = sends(fourth, (ACME, channel), FIRST);

    assert_eq!(
        sent_kinds(&revoked.front, channel, from),
        [(Kind::Message, e + 3)],
        "not the message alone"
    );
    let sixth = second_machine(&revoked.world, fourth, SIXTH_HOST);
    received(&sixth.machine);
    revoked
        .group()
        .assert_is(e + 4, &[fourth.device, fifth.device, sixth.device]);
    received(&fourth.machine);

    let shown = shown_history(&sixth.machine, channel);

    assert_history(&shown, channel, &[said], &[]);
}

/// As above, with the front losing the `stored` answers to the channel's envelopes from epoch
/// e + 1 on: the fourth machine's `skald send` joins at epoch e and sends a Commit of epoch e + 1,
/// whose `stored` is lost, and no message. With the answers forwarded, its next `skald send` sends
/// no message either, as its device is then alone in the group: the server's group is at e + 2
/// with the fourth machine's device alone, and the fourth sent one Commit of epoch e + 1, so it
/// removed the revoked devices once. Once Ada's fifth machine, linked by her fourth, joins, the
/// fourth's next `skald send` sends its one message, in epoch e + 3.
#[test]
fn remove_whose_stored_is_lost_is_committed_once_and_nothing_is_sent_meanwhile() {
    let mut revoked = revoked_group((4_840_004_001, "ada-loses-an-answer"), false);
    revoked.fourth = retrying_for_the_timeout(revoked.fourth);
    let (channel, e) = (revoked.space.default, revoked.epoch);
    let from = revoked.front.sent().len();
    revoked.front.lose_stored_from(Some((channel, e + 1)));

    // The run may fail, as the answer to its Remove is lost.
    let _ = send(&revoked.fourth.machine, channel, FIRST);

    let sent = sent_kinds(&revoked.front, channel, from);
    assert_eq!(
        sent.first(),
        Some(&(Kind::Joining, e)),
        "no external Commit first: {sent:?}"
    );
    assert!(
        sent.contains(&(Kind::Commit, e + 1)),
        "no Remove sent: {sent:?}"
    );
    assert!(
        messages_sent(&revoked.front, channel, from).is_empty(),
        "a message was sent before the Remove was stored: {sent:?}"
    );
    assert!(revoked.front.lost() > 0, "no answer lost");
    revoked.front.lose_stored_from(None);

    let alone = send(&revoked.fourth.machine, channel, SECOND);

    assert!(!alone.success, "{alone:?}");
    assert!(
        messages_sent(&revoked.front, channel, from).is_empty(),
        "a message was sent while the device is alone in the group: {alone:?}"
    );
    let sent = sent_kinds(&revoked.front, channel, from);
    assert_eq!(
        sent.iter()
            .filter(|(kind, _)| *kind == Kind::Commit)
            .count(),
        1,
        "not one Remove: {sent:?}"
    );
    revoked.group().assert_is(e + 2, &[revoked.fourth.device]);
    let fifth = second_machine(&revoked.world, &revoked.fourth, FIFTH_HOST);
    received(&fifth.machine);

    let said = send(&revoked.fourth.machine, channel, SECOND);

    assert!(said.success, "{said:?}");
    one_message_sent(&revoked.front, channel, from);
    let sent = sent_kinds(&revoked.front, channel, from);
    assert!(
        sent.iter()
            .filter(|(kind, _)| *kind == Kind::Message)
            .all(|(_, epoch)| *epoch == e + 3),
        "a message outside epoch {}: {sent:?}",
        e + 3
    );
}

/// Ada and Bob in a channel of `acme`, each with a device in its group; Ada's first machine's
/// device and Bob's are revoked, and her fourth machine, valid, is outside the groups. As Bob's
/// account is in the channel's `channel-member` entries, the fourth machine's `skald recv` joins
/// the channel by itself and removes both devices: the server's group holds its device alone.
#[test]
fn group_also_holding_a_revoked_device_of_another_channel_member_is_joined_and_it_removed() {
    let (talk, fourth) = revoked_talk(
        [4_840_007_001, 4_840_007_002],
        ["ada-joins-past-bob", "bob-revokes-himself"],
    );
    let (space, channel) = (talk.space.id, talk.channel);
    assert_members(
        &group(talk.cloud(), &talk.ada.token, space, channel).members,
        &[talk.ada.device, talk.played[0].device()],
    );

    received(&fourth.machine);

    assert_members(
        &group(talk.cloud(), &fourth.token, space, channel).members,
        &[fourth.device],
    );
}
