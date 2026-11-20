//! What the send tool says of a message the server refuses over its device's send rate: the
//! message's ID, the problem `rate-limited` and what it means, as `skald send` says it
//! (`sending/rate.rs`), and no `sent #`. The front gives the refusal in place of the server's
//! `stored`.

use serde_json::json;

use crate::clients::Clients;
use crate::front::Answers;
use crate::rate_said::{RATE_LIMITED, assert_says_rate_limited};
use crate::talk::one_message_sent;
use crate::{
    CHANNEL_FIELD, EVERYONE, SEND_TOOL, TEXT_FIELD, TO_FIELD, assert_says_not_sent, session_in,
};

/// The text of the message the front refuses.
const REFUSED: &str = "send tool rate test: one status report too many";

/// The front refuses each message with `rate-limited`. A session in `~/work/clientA` calls the send
/// tool `*` in the talk's channel: its result names the message's ID and says the problem and
/// what it means, and not `sent #`.
#[test]
fn a_message_over_the_send_rate_says_what_the_refusal_means() {
    let clients = Clients::new(
        [4_216_063_001, 4_216_063_002],
        ["ada-mcp-over-send-rate", "bob-mcp-under-send-rate"],
    );
    let channel = clients.talk.channel;
    let mut session = session_in(&clients, &clients.folder_a);
    (clients.talk.front).answer(Answers::Refuse(RATE_LIMITED.to_owned()));
    let from = clients.mark();

    let called = session.call(
        SEND_TOOL,
        json!({ TO_FIELD: EVERYONE, CHANNEL_FIELD: channel.to_string(), TEXT_FIELD: REFUSED }),
    );

    let msg_id = one_message_sent(&clients.talk.front, channel, from);
    assert!(
        called.text.contains(&msg_id.to_string()),
        "no message ID: {called:?}"
    );
    assert_says_rate_limited(&called.text, "the send tool");
    assert_says_not_sent(&called.text, "refused over the send rate");
}
