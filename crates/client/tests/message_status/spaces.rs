//! As an agent, the device gives the status only of messages in its folder's space: one handed to
//! the agent of the same name in another space is unknown, and the answer names nothing of that
//! space.

use serde_json::json;

use crate::agents::{CLAUDE, run_as_in};
use crate::client::{CLIENT, MESSAGE_FIELD, session_in};
use crate::helped::with_client_b;
use crate::lines::agent_line_of;
use crate::replying::assert_refused;
use crate::statuses::{Asked, STATUS_TOOL, bob_asks_claude, json_status, status_in};
use crate::talk::Talk;
use crate::world::{ADD_RULE, AGENT, CLIENT_A, CLIENT_B, dir_of, path, run};

/// What Bob's device says in clientb; it is nowhere else.
const IN_B: &str = "space status test: bob's claude sends clientb's claude the contract";

/// Ada's machine follows `clienta` and `clientb`, each with a channel that holds Bob's device;
/// `~/work/clientA` is bound to `clienta`, `~/work/clientB` to `clientb`, and claude has run in
/// both. Bob's device says something in clientb's channel to Ada's claude there; claude's
/// `skald recv` in `~/work/clientB` shows it. As claude in `~/work/clientA`, `skald message-status`
/// of it by that reference and by its `msg_id`, and the `message_status` tool of a session there,
/// say why and name nothing of clientb; the command fails. As claude in `~/work/clientB`, it gives
/// the message in clientb's channel.
#[test]
fn agent_gets_no_status_of_a_message_in_another_space() {
    let mut talk = Talk::in_space(
        CLIENT_A,
        &[4_380_007_001, 4_380_007_002],
        &["ada-asks-within-a-client", "bob-writes-clientb-status"],
        false,
    );
    let (other, elsewhere) = with_client_b(&mut talk);
    let machine = &talk.ada.machine;
    let [in_a, in_b] =
        [(CLIENT_A, "work/clientA"), (CLIENT_B, "work/clientB")].map(|(space, dir)| {
            let folder = dir_of(machine, dir);
            let bound = run(machine, &[ADD_RULE, path(&folder), space]);
            assert!(bound.success, "{space} not bound: {bound:?}");
            run_as_in(machine, &folder, CLAUDE);
            folder
        });
    let at = (CLIENT_B, (other.id, elsewhere));
    let asked = bob_asks_claude(&mut talk, at, (1, IN_B), Asked::default());
    let machine = &talk.ada.machine;
    let shown = run_as_in(machine, &in_b, CLAUDE).shown();
    let short = format!("#{}", agent_line_of(&shown, IN_B).reference);
    let id = asked.to_string();
    let names: [&str; 5] = [
        CLIENT_B,
        IN_B,
        &other.id.to_string(),
        &elsewhere.to_string(),
        &talk.played[0].device().to_string(),
    ];

    let by_ref = status_in(machine, (&in_a, Some(CLAUDE)), &[], &short);
    let by_id = status_in(machine, (&in_a, Some(CLAUDE)), &[], &id);
    let mut session = session_in(machine, &in_a, &[AGENT, CLAUDE], CLIENT);
    let tool = session.call(STATUS_TOOL, json!({ MESSAGE_FIELD: short }));

    assert_refused(
        &by_ref,
        &[&names[..], &[id.as_str()]].concat(),
        "clientb's message by #<ref>",
    );
    assert_refused(&by_id, &names, "clientb's message by msg_id");
    let said = tool.text.to_lowercase();
    assert!(!said.trim().is_empty(), "the tool says nothing: {tool:?}");
    for name in names.iter().chain([&id.as_str()]) {
        assert!(
            !said.contains(&name.to_lowercase()),
            "the tool names {name:?}: {said}"
        );
    }
    let object = json_status(machine, (&in_b, Some(CLAUDE)), &short);
    assert_eq!(object["msg_id"].as_str(), Some(id.as_str()), "{object}");
    let channel = elsewhere.to_string();
    assert_eq!(
        object["channel"].as_str(),
        Some(channel.as_str()),
        "{object}"
    );
}
