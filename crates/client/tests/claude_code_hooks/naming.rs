//! Without `--agent`, the hook handler acts as the agent the session's `skald mcp` registers under
//! its default name: the client name Claude Code gives at initialization, the device's certificate
//! `name` and the device's reference.

use skald_proto::MessageId;

use crate::client::{CLIENT, session_in};
use crate::hooks::{Event, api_sends, assert_own_line, hook_in};
use crate::lines::reference;
use crate::solo::{API, Solo, WEB};
use crate::talk::ADA_HOST;

/// The length of a device's reference in a default name.
const DEVICE_REFERENCE: usize = 6;

/// What api says to the session's agent; it is nowhere else.
const TO_DEFAULT: &str = "naming test: api asks the unnamed session for its status";

/// In `~/work/clientA/session`, `skald mcp` runs without `--agent`, initialized as `claude-code`, as
/// Claude Code names itself; its agent is `claude-code-ada-workstation-<reference>`. api sends that
/// agent a message. The hook handler without `--agent` in that folder gives it on `PostToolUse`,
/// as its line from api at `own`; the handler `--agent web` there gives nothing.
#[test]
fn hook_without_agent_acts_as_the_agent_skald_mcp_registered_by_default() {
    let solo = Solo::new(4_913_014_001, "ada-hooks-an-unnamed-session");
    let dir = solo.dir("session");
    let _session = session_in(solo.machine(), &dir, &[], CLIENT);
    let device = reference(MessageId::from_bytes(*solo.talk.ada.device.as_bytes()));
    let name = format!("{CLIENT}-{ADA_HOST}-{}", &device[..DEVICE_REFERENCE]);
    let msg_id = api_sends(&solo, &solo.short(&name), TO_DEFAULT);

    hook_in(solo.machine(), &dir, Some(WEB), Event::PostToolUse)
        .assert_silent("the hook of another agent");
    let context = hook_in(solo.machine(), &dir, None, Event::PostToolUse);

    let context = context.context(Event::PostToolUse);
    assert_own_line(&context, TO_DEFAULT, msg_id, &solo.short(API));
}
