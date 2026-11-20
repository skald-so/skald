//! The status of a message the device handed over: its sending device, channel, time, the
//! sender's trust level and the agents it was handed to.

use crate::agents::CLAUDE;
use crate::client::{CLIENT, session_in};
use crate::lines::agent_line_of;
use crate::receipts::Roles;
use crate::senders::{LAPTOP, OWN};
use crate::statuses::{
    assert_holds, assert_of, assert_refers, json_status, shown_status, tool_status,
};
use crate::world::{AGENT, now};

/// What A's agent sends claude; it is nowhere else.
const ASKED: &str = "message status test: claude, rotate the staging certificate";

/// A's agent sends a message `--to` claude, which claude's `skald recv` shows. claude's
/// `skald message-status --json` by the reference it was shown gives the message's `msg_id`, the
/// channel, A's device, its time, the level `own` and claude's full address among the agents it
/// was handed to. claude's compact status refers to the message and names A's device by its name;
/// claude's `message_status` tool gives the same text. B's person's status by the `msg_id` names
/// it, the channel, A's device and claude.
#[test]
fn recipient_agent_sees_the_sending_device_channel_time_and_handover() {
    let roles = Roles::new(
        [4_380_002_001, 4_380_002_002],
        ["ada-asks-her-own-claude", "bob-watches-the-handover"],
    );
    let since = now();
    let msg_id = roles.a_sends_to_claude(ASKED);
    let claude = roles.claude_reads(false).shown();
    let short = format!("#{}", agent_line_of(&claude, ASKED).reference);
    let folder = roles.senders.agents.folder.as_path();

    let object = json_status(roles.b(), (folder, Some(CLAUDE)), &short);

    let at = (roles.channel(), roles.senders.laptop.device);
    assert_of(&object, msg_id, at, since);
    assert_eq!(object["level"].as_str(), Some(OWN), "{object}");
    let handed: Vec<&str> = (object["handed_to"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .filter_map(|agent| agent.as_str())
        .collect();
    assert_eq!(handed, [roles.full(CLAUDE).as_str()], "{object}");

    let compact = shown_status(roles.b(), (folder, Some(CLAUDE)), &short);
    assert_refers(&compact, msg_id);
    assert_holds(&compact, &[LAPTOP]);
    let mut session = session_in(roles.b(), folder, &[AGENT, CLAUDE], CLIENT);
    let tool = tool_status(&mut session, &short);
    assert_eq!(tool.trim(), compact.trim(), "the tool gives other text");

    let person = shown_status(roles.b(), (folder, None), &msg_id.to_string());
    let channel = roles.channel().to_string();
    assert_holds(&person, &[&msg_id.to_string(), &channel, LAPTOP, CLAUDE]);
}
