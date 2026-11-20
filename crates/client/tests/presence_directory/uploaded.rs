//! The values a client uploads, as another device of the space lists them: an MCP session's agent
//! `available` while the session runs through the helper, `offline` once it ended.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::DeviceId;

use crate::agents::CLAUDE;
use crate::client::{CLIENT, MCP, Session};
use crate::cloud::WAIT;
use crate::helped::Helped;
use crate::senders::Senders;
use crate::values::{AVAILABLE, OFFLINE, listed_json, objects_of};
use crate::world::{AGENT, now};

/// How far a time a value states may be from the tests' clock, in seconds.
const SLACK: u64 = 3;

/// Waits for up to [`WAIT`] until the person's `skald agents --json` on Ada's second machine lists
/// `agent`, a full address, of `device` in `state`; gives that object.
#[track_caller]
fn await_listed(senders: &Senders, (agent, device): (&str, DeviceId), state: &str) -> Value {
    let deadline = Instant::now() + WAIT;
    let id = device.to_string();
    loop {
        let objects = listed_json(&senders.laptop.machine, &senders.laptop_folder);
        let found = (objects_of(&objects, agent).into_iter())
            .find(|object| object["device"].as_str() == Some(id.as_str()))
            .cloned();
        if let Some(object) = found.as_ref()
            && object["state"].as_str() == Some(state)
        {
            return object.clone();
        }
        assert!(
            Instant::now() < deadline,
            "{agent} not {state} within {WAIT:?}: {found:?} in {objects:?}"
        );
        thread::sleep(Duration::from_millis(300));
    }
}

/// Through a helper on Ada's first machine, claude runs an MCP session of `claude-code` in
/// `~/work/acme`. Her second machine's `skald agents` lists claude of the first machine's device
/// `available`, with the runtime `claude-code`, since about when the session started. Once the
/// session ends, it lists claude `offline`, since about when it ended.
#[test]
fn running_mcp_session_is_listed_available_at_another_device_and_offline_once_it_ended() {
    let senders = Senders::new(
        [4_731_061_001, 4_731_061_002],
        ["ada-lists-her-session", "bob-shares-the-space"],
    );
    let helped = Helped::new(&senders.agents.talk.ada.machine);
    let at = (
        senders.agents.address(CLAUDE),
        senders.agents.talk.ada.device,
    );
    let started = now();
    let mut command = helped.command_in(&senders.agents.folder, &[], &[AGENT, CLAUDE, MCP]);
    let session = Session::start(&mut command, CLIENT);

    let shown = await_listed(&senders, (&at.0, at.1), AVAILABLE);

    assert_eq!(shown["runtime"].as_str(), Some("claude-code"), "{shown}");
    let since = shown["since"].as_u64().expect("a since");
    assert!(
        started.saturating_sub(SLACK) <= since && since <= now() + SLACK,
        "available since {since}, not after {started}: {shown}"
    );
    let ended = now();
    drop(session);

    let shown = await_listed(&senders, (&at.0, at.1), OFFLINE);

    let since = shown["since"].as_u64().expect("a since");
    assert!(
        ended.saturating_sub(SLACK) <= since && since <= now() + SLACK,
        "offline since {since}, not after {ended}: {shown}"
    );
}
