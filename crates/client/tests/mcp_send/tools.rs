//! The answer to `tools/list` with the send tool: `to` is its one required field and says that an
//! addressee does not make a message private, and the answer stays within the bound of `inbox`'s
//! tests.

use serde_json::Value;

use crate::client::INBOX;
use crate::clients::Clients;
use crate::{CHANNEL_FIELD, SEND_TOOL, TEXT_FIELD, TO_FIELD, TOOLS_BOUND, session_in};

/// A session in `~/work/clientA` lists its tools: `inbox` and `send`. The send tool's input schema
/// requires `to` and `text`, not `channel`, and its description of `to` says `private`. The result
/// is at most [`TOOLS_BOUND`] bytes as compact JSON.
#[test]
fn tools_list_has_send_whose_required_to_says_an_addressee_is_not_private() {
    let clients = Clients::new(
        [4_154_010_001, 4_154_010_002],
        ["ada-mcp-lists-send", "bob-mcp-not-listed"],
    );
    let mut session = session_in(&clients, &clients.folder_a);

    let listed = session.tools();

    let tools = (listed["tools"].as_array().map(Vec::as_slice)).unwrap_or_default();
    let named = |name: &str| tools.iter().find(|tool| tool["name"] == name);
    assert!(named(INBOX).is_some(), "no {INBOX}: {listed}");
    let send = named(SEND_TOOL).unwrap_or_else(|| panic!("no {SEND_TOOL}: {listed}"));
    let schema = &send["inputSchema"];
    let required: Vec<&str> = (schema["required"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for field in [TO_FIELD, TEXT_FIELD] {
        assert!(required.contains(&field), "{field} not required: {send}");
    }
    assert!(
        !required.contains(&CHANNEL_FIELD),
        "channel required: {send}"
    );
    assert!(
        schema["properties"][CHANNEL_FIELD].is_object(),
        "no channel: {send}"
    );
    let of_to = schema["properties"][TO_FIELD]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(
        of_to.to_lowercase().contains("private"),
        "the description of to does not say an addressee does not make a message private: {send}"
    );
    let size = Value::to_string(&listed).len();
    assert!(size <= TOOLS_BOUND, "{size} bytes: {listed}");
}
