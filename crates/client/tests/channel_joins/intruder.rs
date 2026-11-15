//! A device the server lets in but the members do not admit joins a channel by itself; a member
//! device removes it with a warning, and sends nothing in the channel until it is gone.

use crate::front::Answers;
use crate::joins::{intruder, intruder_joins, read_from};
use crate::kit::{assert_members, group, warnings_naming};
use crate::talk::{Talk, commit_sent, messages_sent, one_message_sent};

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "the quarterly numbers are final";
const SECOND: &str = "board deck goes out tonight";

/// A device on Bob's account, its certificate chained to another master key, joins the channel's
/// group by itself through the server. Ada's `skald send`, every `stored` lost, warns naming that
/// device, commits its Remove, sends no message and fails. With the answers forwarded, her next
/// `skald send` sends the Remove again and then its message, which Bob's device reads; Bob's device
/// gets the Remove from Ada's, and the group is at epoch 3 with Ada's and Bob's devices.
#[test]
fn joiner_the_members_do_not_admit_is_removed_with_a_warning_before_anything_is_sent() {
    let mut talk = Talk::new(
        &[4_138_082_001, 4_138_082_002],
        &["ada-spots-intruder", "bob-account-abused"],
        true,
    );
    let (at, channel) = (talk.at(), talk.channel);
    let ada = talk.ada.device;
    let intruder = intruder(talk.cloud(), &talk.played[0].person, "bob-server-device");
    intruder_joins(talk.cloud(), &intruder, at);
    let from = talk.front.sent().len();
    talk.front.answer(Answers::Lose);

    let refused = talk.send(FIRST);

    assert!(
        !warnings_naming(&refused, intruder.id()).is_empty(),
        "no warning naming the joiner: {refused:?}"
    );
    assert!(
        !refused.success && talk.front.lost() > 0,
        "not failed with a stored lost: {refused:?}"
    );
    assert!(
        commit_sent(&talk.front, channel, from),
        "no Remove sent: {refused:?}"
    );
    assert!(
        messages_sent(&talk.front, channel, from).is_empty(),
        "a message was sent with the joiner in the group"
    );
    talk.front.answer(Answers::Forward);

    let said = talk.send(SECOND);

    assert!(said.success, "{said:?}");
    one_message_sent(&talk.front, channel, from);
    let cloud = &talk.world.cloud;
    let bob = &mut talk.played[0];
    bob.take_in(cloud);
    assert_eq!(
        read_from(bob, channel, SECOND),
        [ada],
        "Bob's device does not read Ada's message once"
    );
    assert!(
        read_from(bob, channel, FIRST).is_empty(),
        "Bob's device read the message sent while the joiner was in the group"
    );
    let changes = bob.changes_of(channel);
    let last = changes.last().expect("Ada's Remove");
    let removed: Vec<_> = last.removed.iter().map(|member| member.device).collect();
    assert_eq!(
        (last.sender, removed),
        (ada, vec![intruder.id()]),
        "the Remove of the joiner by Ada's device: {changes:?}"
    );
    assert_members(&bob.members(channel), &[ada, bob.device()]);
    let held = group(cloud, &talk.ada.token, at.0, channel);
    held.assert_is(3, &[ada, bob.device()]);
}
