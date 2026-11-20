//! Who lists a channel's queues: devices that may send in the channel, each proving its key.

use crate::kit::{Body, NOT_A_MEMBER, World, assert_problem};
use crate::presence::{assert_refused_with, assert_unproved};
use crate::tokens::{QUEUES, RECEIVE, SEND};

/// The bot on a token with `send` for `general` gets the queues alice's laptop gets. Refused: dave's
/// laptop, outside the channel, and the bot on a token without `send`, with `not-a-member`; eve's
/// laptop, outside the space; and, with `401` and `auth-required`, carol's session without her
/// laptop's proof.
#[test]
fn only_devices_that_may_send_in_the_channel_list_its_queues() {
    let w = World::new(3_907_700_001);
    let [alice, _, carol, dave, eve] = w.laptops();
    let sending = w.bot(&[SEND]);
    assert_eq!(sending.queues(), alice.queues(), "the bot's queues");

    let no_send = w.bot(&[RECEIVE, QUEUES]);
    for client in [&dave, &no_send] {
        assert_problem(&client.list(), NOT_A_MEMBER, &client.who);
    }
    assert_refused_with(&eve.list(), None, &eve.who);
    let unproved = carol.call("get", &carol.queues_path(), Body::None, false);
    assert_unproved(&unproved, "carol's session");
}
