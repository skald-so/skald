//! The `to` agent's `refused`, with its reason, in the sender's status: across devices by its
//! receipt, and on one device with no receipt message.

use crate::across::a_json_once;
use crate::agents::CLAUDE;
use crate::inbound::{REFUSE, REFUSED, SET_MODE};
use crate::kit::received;
use crate::receipts::{READ, Roles};
use crate::senders::LAPTOP_AGENT;
use crate::solo::{API, Solo, WEB};
use crate::statuses::{agent_time, assert_holds, hashed, json_status, shown_status};
use crate::world::now;

/// What A's agent sends claude, and what api sends web; neither is anywhere else.
const TO_CLAUDE: &str = "message status test: claude, rotate the staging keys";
const TO_WEB: &str = "message status test: web, purge the cache";

/// The reason of the `refused` in `object`; fails unless it is a non-empty string.
#[track_caller]
fn reason_of(object: &serde_json::Value) -> String {
    let reason = object["agent"]["reason"].as_str().unwrap_or_default();
    assert!(!reason.is_empty(), "no reason: {object}");
    reason.to_owned()
}

/// B's person sets claude to `refuse`. A's agent sends a message `--to` claude; claude's and B's
/// person's `skald recv` follow. A's agent's `skald message-status --json` of it then gives
/// `refused`, no earlier than the send, with a reason, and no `read`; A's person's status and A's
/// agent's say `refused` with that reason.
#[test]
fn refused_receipt_gives_refused_with_its_reason() {
    let roles = Roles::new(
        [4_380_011_001, 4_380_011_002],
        ["ada-is-turned-away", "bob-watches-the-refusal"],
    );
    let b_folder = roles.senders.agents.folder.clone();
    let set = roles
        .b()
        .start_in(&b_folder, &[SET_MODE, CLAUDE, REFUSE])
        .finish();
    assert!(set.success, "{set:?}");
    let sent = now();

    let msg_id = roles.a_sends_to_claude(TO_CLAUDE);
    assert!(roles.claude_reads(false).success, "claude's recv failed");
    assert!(received(roles.b()).success, "B's recv failed");

    let refused = a_json_once(&roles, msg_id, "no refused", |object| {
        agent_time(object, REFUSED).is_some()
    });
    let at = agent_time(&refused, REFUSED).unwrap_or_default();
    assert!((sent..=now()).contains(&at), "refused at {at}: {refused}");
    assert_eq!(agent_time(&refused, READ), None, "{refused}");
    let reason = reason_of(&refused);
    let person = shown_status(
        roles.a(),
        (&roles.senders.laptop_folder, None),
        &hashed(msg_id),
    );
    assert_holds(&person, &[REFUSED, &reason]);
    let agent_at = (roles.senders.laptop_folder.as_path(), Some(LAPTOP_AGENT));
    let agent = shown_status(roles.a(), agent_at, &hashed(msg_id));
    assert_holds(&agent, &[REFUSED, &reason]);
}

/// Ada's machine is alone in `clienta`'s groups; web's session runs in `~/work/clientA/web`, and
/// the person sets web to `refuse`. api's
/// `skald send --to ada/web` sends one message. api's `skald message-status --json` of it gives
/// `refused` with a reason and no `read`, and its status says so; the front saw no message but
/// that one: no receipt.
#[test]
fn on_one_device_refused_comes_with_its_reason_and_no_receipt() {
    let solo = Solo::new(4_380_010_001, "ada-web-refuses-api");
    solo.session(WEB).inbox();
    let set = solo.person_runs(&[SET_MODE, WEB, REFUSE]);
    assert!(set.success, "{set:?}");
    let mark = solo.mark();

    let msg_id = solo.api_sends_web(TO_WEB);

    let dir = solo.dir(API);
    let at = (dir.as_path(), Some(API));
    let object = json_status(solo.machine(), at, &msg_id.to_string());
    assert!(
        agent_time(&object, REFUSED).is_some(),
        "no refused: {object}"
    );
    assert_eq!(agent_time(&object, READ), None, "{object}");
    let reason = reason_of(&object);
    assert_holds(
        &shown_status(solo.machine(), at, &hashed(msg_id)),
        &[REFUSED, &reason],
    );
    assert_eq!(solo.sent_since(mark), [msg_id], "not the message alone");
}
