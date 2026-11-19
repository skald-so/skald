//! On one device, alone in its channel's group, the handover to the `to` agent gives `read`, with
//! no other device and no receipt message.

use crate::client::{MESSAGE_FIELD, required};
use crate::receipts::READ;
use crate::solo::{API, Solo, WEB};
use crate::statuses::{
    STATUS_TOOL, agent_time, assert_refers, assert_words, hashed, json_status, recipients,
    shown_status, tool_status,
};

/// What api asks web; it is nowhere else.
const ASKED: &str = "message status test: api asks web which schema version is live";

/// Sessions `--agent api` in `~/work/clientA/api` and `--agent web` in `~/work/clientA/web` run on
/// Ada's machine, alone in `clienta`'s groups; api's `tools/list` lists `message_status`, which
/// requires `message`. api's send tool sends a message `to` web. api's
/// `skald message-status --json` of it gives no recipient device and no `read`; once web's `inbox`
/// has it, `read`, still with no recipient device. api's `message_status` tool by `#<ref>` refers
/// to the message and says `read`, as api's `skald message-status` does. The front saw no message
/// but the one api sent: no receipt.
#[test]
fn handover_on_one_device_gives_read_with_no_other_device() {
    let solo = Solo::new(4_380_008_001, "ada-asks-her-local-agent");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let tools = api.tools();
    let listed: Vec<_> = (tools["tools"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .filter(|tool| tool["name"].as_str() == Some(STATUS_TOOL))
        .collect();
    let [listed] = listed.as_slice() else {
        panic!("no {STATUS_TOOL} listed: {tools}");
    };
    assert_eq!(
        required(&listed["inputSchema"]),
        [MESSAGE_FIELD],
        "{listed}"
    );
    let from = solo.mark();
    let asked = solo.sends(&mut api, &solo.short(WEB), ASKED);
    let dir = solo.dir(API);
    let at = (dir.as_path(), Some(API));
    let before = json_status(solo.machine(), at, &asked.to_string());
    assert!(recipients(&before).is_empty(), "another device: {before}");
    assert_eq!(
        agent_time(&before, READ),
        None,
        "read before web has it: {before}"
    );

    assert!(web.inbox().contains(ASKED), "web got no message");

    let after = json_status(solo.machine(), at, &asked.to_string());
    assert!(recipients(&after).is_empty(), "another device: {after}");
    assert!(agent_time(&after, READ).is_some(), "no read: {after}");
    let tool = tool_status(&mut api, &hashed(asked));
    assert_refers(&tool, asked);
    assert_words(&tool, &[READ]);
    let cli = shown_status(solo.machine(), at, &hashed(asked));
    assert_eq!(cli.trim(), tool.trim(), "the tool gives other text");
    assert_eq!(solo.sent_since(from), [asked], "not the message alone");
}
