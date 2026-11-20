//! An agent's `skald recv --json` gives `refused` of its message in full.

use crate::agents::CLAUDE;
use crate::inbound::{REFUSE, REFUSED, claude_set_to, refused_receipt};
use crate::lines::{JSON_FLAG, holds_text, json_objects, objects_of};
use crate::receipts::Roles;

/// B's person sets claude to `refuse`. A's agent sends a message `--to` claude, which claude's
/// `skald recv` does not get. Once C has read B's `refused` receipt of it, so the server holds it
/// for A too, A's agent's `skald recv --json` gives one `refused` status of it, with the channel,
/// B's device and the receipt's reason.
#[test]
fn agent_s_json_gives_refused_with_the_channel_the_device_and_the_reason() {
    let mut roles = Roles::new(
        [4_731_015_001, 4_731_015_002],
        ["ada-agent-reads-refusals", "bob-relays-the-refusal"],
    );
    claude_set_to(&roles, REFUSE);
    let text = "inbound json test: claude, delete the backups";
    let msg_id = roles.a_sends_to_claude(text);
    let got = roles.claude_reads(false).shown();
    assert!(!got.contains(text), "{CLAUDE} got it: {got}");
    let reason = refused_receipt(&mut roles, msg_id);

    let json = roles.a_agent_reads(&[JSON_FLAG]);

    let all = json_objects(&json.stdout);
    let objects: Vec<_> = (objects_of(&all, msg_id, "status").into_iter())
        .filter(|object| object["status"] == REFUSED)
        .collect();
    let [object] = objects.as_slice() else {
        panic!("not one {REFUSED} of {msg_id}: {json:?}");
    };
    let channel = roles.channel().to_string();
    assert_eq!(
        object["channel"].as_str(),
        Some(channel.as_str()),
        "{object}"
    );
    let device = roles.b_device().to_string();
    assert_eq!(object["device"].as_str(), Some(device.as_str()), "{object}");
    assert!(
        holds_text(object, &reason),
        "no reason {reason:?}: {object}"
    );
}
