//! While a key change waits, every command, as the person and as an agent, one-shot and through a
//! helper, a waiting `skald recv --wait` and each tool call of the MCP server print the line once,
//! and `--json` carries it.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::{send_args, strs};
use crate::kit::RECV;
use crate::lines::{JSON_FLAG, json_objects};
use crate::machine::{Finished, Run};
use crate::mcp_client::{
    CHANNEL_FIELD, CLIENT, EVERYONE, MCP, SEND_TOOL, Session, TEXT_FIELD, TO_FIELD,
};
use crate::waits::{IDLE_WATCH, WAIT_FLAG, Waits};
use crate::world::{ACME, AGENT, GLOBEX, MEMBERS};
use crate::{SHOWN_BY, assert_says_waiting, bobs_two_masters};

/// How many key changes wait in these tests.
const ONE: u64 = 1;

/// The commands of these tests: `skald recv`, `skald send` of a text of its own to the talk's
/// channel and `skald members acme`. `tag` sets the text apart.
fn commands(agents: &Agents, tag: &str) -> Vec<Vec<String>> {
    let text = format!("key changes test: sent {tag} while a key change waits");
    vec![
        vec![RECV.to_owned()],
        send_args(agents.talk.channel, &text),
        vec![MEMBERS.to_owned(), ACME.to_owned()],
    ]
}

/// `args` with `--agent <claude>` before them.
fn as_claude(args: &[String]) -> Vec<String> {
    let mut all = vec![AGENT.to_owned(), CLAUDE.to_owned()];
    all.extend_from_slice(args);
    all
}

/// Fails unless `finished`, the run of `args`, succeeded and printed the line once, saying one key
/// change waits; as an agent, the line does not name `globex`.
#[track_caller]
fn assert_said_once(finished: &Finished, args: &[String]) {
    assert!(finished.success, "{args:?}: {finished:?}");
    let line = assert_says_waiting(&finished.shown(), Some(ONE));
    if args.first().map(String::as_str) == Some(AGENT) {
        assert!(!line.contains(GLOBEX), "{args:?}: {line:?}");
    }
}

/// One key change waits on Ada's machine. In one-shot mode, `skald recv`, `skald send` and `skald
/// members acme`, as the person and as claude in `~/work/acme`, each print the line once, saying one
/// key change waits.
#[test]
fn every_one_shot_command_says_how_many_key_changes_wait() {
    let agents = Agents::new(
        [4_188_701_001, 4_188_701_002],
        ["ada-one-shot-with-a-key-change", "bob-two-masters-one-shot"],
    );
    bobs_two_masters(&agents);
    let machine = &agents.talk.ada.machine;

    let by_claude = commands(&agents, "by claude, one-shot");
    for (args, claude) in commands(&agents, "by ada, one-shot").iter().zip(&by_claude) {
        assert_said_once(&machine.start(&strs(args)).finish(), args);
        let agent_args = as_claude(claude);
        let agent = machine
            .start_in(&agents.folder, &strs(&agent_args))
            .finish();
        assert_said_once(&agent, &agent_args);
    }
}

/// One key change waits on Ada's machine. Through a helper, `skald recv`, `skald send` and `skald
/// members acme`, as the person and as claude in `~/work/acme`, each print the line once, saying one
/// key change waits. One helper ran them.
#[test]
fn every_command_through_a_helper_says_how_many_key_changes_wait() {
    let waits = Waits::new(
        [4_188_702_001, 4_188_702_002],
        ["ada-helped-with-a-key-change", "bob-two-masters-helped"],
    );
    bobs_two_masters(&waits.agents);
    let agents = &waits.agents;

    let by_claude = commands(agents, "by claude, through a helper");
    for (args, claude) in commands(agents, "by ada, through a helper")
        .iter()
        .zip(&by_claude)
    {
        assert_said_once(&waits.helped.run(&strs(args)), args);
        let agent_args = as_claude(claude);
        let agent = waits.helped.run_in(&agents.folder, &strs(&agent_args));
        assert_said_once(&agent, &agent_args);
    }
    waits.helped.one_helper();
}

/// Waits until `run`, a `skald recv --wait`, prints the line, then up to [`IDLE_WATCH`] for it to
/// exit; kills it if it still waits. Gives what it printed.
#[track_caller]
fn settled(mut run: Run) -> Finished {
    run.wait_to_show(&[SHOWN_BY]);
    let until = Instant::now() + IDLE_WATCH;
    while !run.exited() && Instant::now() < until {
        thread::sleep(Duration::from_millis(50));
    }
    run.kill()
}

/// Fails unless `run`, a `skald recv --wait` of `args`, prints the line once, saying one key change
/// waits, as [`settled`] watches it.
#[track_caller]
fn wait_says_it_once(run: Run, args: &[String]) {
    let shown = settled(run);
    assert_says_waiting(&shown.shown(), Some(ONE));
    if args.first().map(String::as_str) == Some(AGENT) {
        assert!(!shown.shown().contains(GLOBEX), "{args:?}: {shown:?}");
    }
}

/// One key change waits on Ada's machine. Her `skald recv --wait` and claude's in `~/work/acme`,
/// one-shot, then through a helper, each print the line once.
#[test]
fn waiting_recv_says_how_many_key_changes_wait() {
    let waits = Waits::new(
        [4_188_703_001, 4_188_703_002],
        ["ada-waits-with-a-key-change", "bob-two-masters-for-waits"],
    );
    let agents = &waits.agents;
    agents.run_as(CLAUDE);
    bobs_two_masters(agents);
    let (machine, folder) = (&agents.talk.ada.machine, &agents.folder);
    let person = [RECV, WAIT_FLAG].map(str::to_owned).to_vec();
    let agent = as_claude(&person);

    wait_says_it_once(machine.start(&strs(&person)), &person);
    wait_says_it_once(machine.start_in(folder, &strs(&agent)), &agent);
    wait_says_it_once(waits.helped.start(&[], &strs(&person)), &person);
    wait_says_it_once(waits.helped.start_in(folder, &[], &strs(&agent)), &agent);
}

/// Whether `value` holds `number` at some depth.
fn holds_number(value: &Value, number: u64) -> bool {
    match value {
        Value::Number(held) => held.as_u64() == Some(number),
        Value::Array(items) => items.iter().any(|item| holds_number(item, number)),
        Value::Object(fields) => fields.values().any(|field| holds_number(field, number)),
        _ => false,
    }
}

/// Whether `value` holds a string that contains `text` at some depth.
fn holds_part(value: &Value, text: &str) -> bool {
    match value {
        Value::String(held) => held.contains(text),
        Value::Array(items) => items.iter().any(|item| holds_part(item, text)),
        Value::Object(fields) => fields.values().any(|field| holds_part(field, text)),
        _ => false,
    }
}

/// Fails unless `stdout`, of the run `what` names, holds only JSON objects, one of which holds one
/// key change waiting and the command that shows them.
#[track_caller]
fn assert_carries(what: &str, stdout: &str) {
    let objects = json_objects(stdout);
    let carrying = (objects.iter())
        .filter(|object| holds_number(object, ONE) && holds_part(object, SHOWN_BY))
        .count();
    assert_eq!(carrying, 1, "{what}: {objects:#?}");
}

/// One key change waits on Ada's machine. `skald recv --json` as the person and as claude in
/// `~/work/acme`, her `skald send --json` and her `skald recv --wait --json` print only JSON objects
/// on their standard output, one of which carries the key change waiting.
#[test]
fn json_carries_how_many_key_changes_wait() {
    let agents = Agents::new(
        [4_188_704_001, 4_188_704_002],
        [
            "ada-reads-json-with-a-key-change",
            "bob-two-masters-in-json",
        ],
    );
    bobs_two_masters(&agents);
    let machine = &agents.talk.ada.machine;
    let mut send = send_args(agents.talk.channel, "key changes test: sent as JSON");
    send.insert(1, JSON_FLAG.to_owned());

    let recv = [RECV.to_owned(), JSON_FLAG.to_owned()];
    let sent = machine.start(&strs(&send)).finish();
    let person = machine.start(&strs(&recv)).finish();
    let agent = machine
        .start_in(&agents.folder, &strs(&as_claude(&recv)))
        .finish();
    let waited = settled(machine.start(&[RECV, WAIT_FLAG, JSON_FLAG]));

    for (what, finished) in [
        ("skald recv --json", &person),
        ("claude's skald recv --json", &agent),
        ("skald send --json", &sent),
    ] {
        assert!(finished.success, "{what}: {finished:?}");
        assert_carries(what, &finished.stdout);
    }
    assert!(
        !agent.shown().contains(GLOBEX),
        "claude's run names globex: {agent:?}"
    );
    assert_carries("skald recv --wait --json", &waited.stdout);
}

/// One key change waits on Ada's machine. A session of `skald mcp` as claude in `~/work/acme` starts
/// and calls `inbox`, then sends a message with the send tool to `*` in the talk's channel: the text of
/// each result says once that one key change waits, without naming `globex`.
#[test]
fn each_mcp_tool_result_says_how_many_key_changes_wait() {
    let agents = Agents::new(
        [4_188_705_001, 4_188_705_002],
        [
            "ada-agent-on-mcp-with-a-key-change",
            "bob-two-masters-on-mcp",
        ],
    );
    bobs_two_masters(&agents);
    let command = &mut (agents.talk.ada.machine).command_in(&agents.folder, &[AGENT, CLAUDE, MCP]);
    let mut session = Session::start(command, CLIENT);

    let inbox = session.inbox();
    let arguments = json!({
        TO_FIELD: EVERYONE,
        CHANNEL_FIELD: agents.talk.channel.to_string(),
        TEXT_FIELD: "key changes test: sent with the send tool",
    });
    let sent = session.call(SEND_TOOL, arguments);

    assert!(!sent.error, "{sent:?}: {}", session.errors());
    for text in [&inbox, &sent.text] {
        let line = assert_says_waiting(text, Some(ONE));
        assert!(!line.contains(GLOBEX), "{line:?}");
    }
}
