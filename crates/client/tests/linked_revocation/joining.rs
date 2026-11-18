//! Joining a channel whose group holds only revoked devices: the valid device outside it joins by
//! itself, removes them, then sends, never in its join epoch; a device linked later joins from its
//! GroupInfo and reads its messages from the history. With the `stored` of the Remove lost, the
//! next run removes them once, and the device sends nothing meanwhile.

use crate::devices::second_machine;
use crate::fixtures::{retrying_for_the_timeout, send};
use crate::kit::received;
use crate::linked::{FIFTH_HOST, Kind, revoked_group, sent_kinds};
use crate::reading::{assert_history, sends, shown_history};
use crate::talk::{messages_sent, one_message_sent};
use crate::world::ACME;

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "the new desk takes over the channel";
const SECOND: &str = "the new desk reports in";

/// The default channel of `acme` holds Ada's first and second machines' devices, both revoked, at
/// epoch e; her fourth machine's device, valid, is outside it. The fourth machine's `skald send`
/// sends, in the channel, an external Commit of epoch e, then a Commit of epoch e + 1, then its
/// message in epoch e + 2: the server's group is at e + 2 with the fourth machine's device alone,
/// which signs its GroupInfo. Ada's fifth machine, linked by her fourth, joins the channel by
/// itself at e + 3; once the fourth has run since, the fifth's `skald history` shows the fourth's
/// message.
#[test]
fn device_joins_a_group_of_revoked_devices_by_itself_removes_them_then_sends() {
    let revoked = revoked_group((4_840_003_001, "ada-starts-over"), false);
    let (fourth, e) = (&revoked.fourth, revoked.epoch);
    revoked
        .group()
        .assert_is(e, &[revoked.ada.device, revoked.second.device]);
    let from = revoked.front.sent().len();

    let said = sends(fourth, (ACME, revoked.space.default), FIRST);

    assert_eq!(
        sent_kinds(&revoked.front, revoked.space.default, from),
        [
            (Kind::Joining, e),
            (Kind::Commit, e + 1),
            (Kind::Message, e + 2)
        ],
        "not a join, a Remove, then the message"
    );
    let held = revoked.group();
    held.assert_is(e + 2, &[fourth.device]);
    assert_eq!(held.signer, fourth.device, "the GroupInfo's signer");
    let fifth = second_machine(&revoked.world, fourth, FIFTH_HOST);
    received(&fifth.machine);
    revoked
        .group()
        .assert_is(e + 3, &[fourth.device, fifth.device]);
    received(&fourth.machine);

    let shown = shown_history(&fifth.machine, revoked.space.default);

    assert_history(&shown, revoked.space.default, &[said], &[]);
}

/// As above, with the front losing the `stored` answers to the channel's envelopes from epoch
/// e + 1 on: the fourth machine's `skald send` joins at epoch e and sends a Commit of epoch e + 1,
/// whose `stored` is lost, and no message. With the answers forwarded, its next `skald send` sends
/// its one message in epoch e + 2: the server's group is at e + 2 with the fourth machine's device
/// alone, so it removed the revoked devices once.
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

    let said = send(&revoked.fourth.machine, channel, SECOND);

    assert!(said.success, "{said:?}");
    one_message_sent(&revoked.front, channel, from);
    let sent = sent_kinds(&revoked.front, channel, from);
    assert!(
        sent.iter()
            .filter(|(kind, _)| *kind == Kind::Message)
            .all(|(_, epoch)| *epoch == e + 2),
        "a message outside epoch {}: {sent:?}",
        e + 2
    );
    revoked.group().assert_is(e + 2, &[revoked.fourth.device]);
}
