//! Whether a result of the send tool is an error: not for a sent message, but for each call that
//! sends nothing or whose message the server refuses.

use serde_json::json;

use crate::clients::{BOB_CLAUDE, Clients};
use crate::front::Answers;
use crate::world::CLIENT_A;
use crate::{CHANNEL_FIELD, EVERYONE, SEND_TOOL, TEXT_FIELD, TO_FIELD, sent_by, session_in};

/// The texts of this test; none is anywhere else.
const SENT: &str = "send tool result: this one is sent";
const REFUSED: &str = "send tool result: this one is refused";

/// A session in `~/work/clientA` sends `*` in the talk's channel: its result is no error. Calls
/// without `to` and `to` a person's address, a call from a folder in no space, and a call whose
/// message the front refuses in place of the server each give an error.
#[test]
fn sent_call_is_no_error_and_each_refused_one_is() {
    let clients = Clients::new(
        [4_154_011_001, 4_154_011_002],
        ["ada-mcp-result-flags", "bob-mcp-result-flags"],
    );
    let channel = clients.talk.channel;
    let in_channel = channel.to_string();
    let person = format!("{}/user", clients.bob());
    let nowhere = clients.bound("work/clientC", "clientc");
    let mut session = session_in(&clients, &clients.folder_a);
    let mut elsewhere = session_in(&clients, &nowhere);

    let arguments = json!({ TO_FIELD: EVERYONE, CHANNEL_FIELD: in_channel, TEXT_FIELD: SENT });
    sent_by(&mut session, &clients, arguments, channel);

    let cases = [
        (
            "no to",
            json!({ CHANNEL_FIELD: in_channel, TEXT_FIELD: REFUSED }),
        ),
        (
            "a person",
            json!({ TO_FIELD: person, CHANNEL_FIELD: in_channel, TEXT_FIELD: REFUSED }),
        ),
    ];
    for (what, arguments) in cases {
        let called = session.call(SEND_TOOL, arguments);
        assert!(called.error, "{what}: no error: {called:?}");
    }
    let claude = clients.bob_agent(CLIENT_A, BOB_CLAUDE);
    let called = elsewhere.call(SEND_TOOL, json!({ TO_FIELD: claude, TEXT_FIELD: REFUSED }));
    assert!(called.error, "in no space: no error: {called:?}");
    (clients.talk.front).answer(Answers::Refuse("not-a-member".to_owned()));
    let called = session.call(
        SEND_TOOL,
        json!({ TO_FIELD: EVERYONE, CHANNEL_FIELD: in_channel, TEXT_FIELD: REFUSED }),
    );
    assert!(called.error, "refused by the server: no error: {called:?}");
}
