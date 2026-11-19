//! A busy Claude Code session gets its agent's messages through its hooks: an `own` message after
//! the running tool as its line, a `peer` one only as the signal, whose `inbox` call then gives its
//! text labelled as data; one sent as the turn ends by `Stop`, and the turn goes on; a session of
//! an agent of that name in another space gets none. Without `--agent`, the hooks act as the agent
//! the real Claude Code's `skald mcp` registers.

use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::claude::{Claude, Ended, claude_code};
use crate::client::{TEXT_FIELD, TO_FIELD};
use crate::hooks::{Bob, INBOX_WORD, api_sends};
use crate::lines::{AgentLine, agent_line, agent_line_of, refers_to};
use crate::senders::{DATA, OWN, PEER, holds_word};
use crate::solo::{API, Solo, WEB};
use crate::stub::{Step, Stub, call_id, played};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::dir_of;

/// The step that calls `inbox` without arguments.
fn inbox() -> Step {
    Step::Skald("inbox", json!({}))
}

/// Every string of the conversation in `body`, a line apart.
fn conversation(body: &Value) -> String {
    let mut texts = Vec::new();
    strings(&body["messages"], &mut texts);
    texts.join("\n")
}

fn strings(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::String(text) => into.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| strings(item, into)),
        Value::Object(fields) => fields.values().for_each(|item| strings(item, into)),
        _ => {}
    }
}

/// What the conversation in `body` holds from the stub's tool call of step `step` on: the call,
/// its result and what came after them.
fn after_call(body: &Value, step: usize) -> String {
    let all = conversation(body);
    let id = call_id(step);
    let at = (all.find(&id)).unwrap_or_else(|| panic!("no {id} in the conversation"));
    all[at + id.len()..].to_owned()
}

/// The request of the session's turn once it has played `step` steps; fails unless there is one.
#[track_caller]
fn turn_at(stub: &Stub, step: usize) -> Value {
    (stub.turns().into_iter())
        .find(|body| played(body) == step)
        .unwrap_or_else(|| panic!("no request after step {step}"))
}

/// The message line in `text` whose text is `said`, wherever in a line it starts; fails unless
/// there is one.
#[track_caller]
fn line_in(text: &str, said: &str) -> AgentLine {
    let needle = format!(": {said}");
    let at = text
        .find(&needle)
        .unwrap_or_else(|| panic!("no {said:?} in {text}"));
    let start = text[..at]
        .rfind('[')
        .unwrap_or_else(|| panic!("no label of {said:?}: {text}"));
    let end = at + needle.len();
    agent_line(&text[start..end])
        .unwrap_or_else(|| panic!("not a message line: {:?}", &text[start..end]))
}

/// Fails unless `line` is the message `msg_id` from `from`, by the device `device`, at `level`.
#[track_caller]
fn assert_from(line: &AgentLine, msg_id: MessageId, (from, device, level): (&str, &str, &str)) {
    assert!(
        refers_to(&line.reference, msg_id),
        "{line:?} is not {msg_id}"
    );
    assert_eq!(
        (
            line.from.as_str(),
            line.device.as_str(),
            line.level.as_str()
        ),
        (from, device, level),
        "{line:?}"
    );
}

/// Fails unless `ended` succeeded and its stub got no credential other than the placeholder.
#[track_caller]
fn assert_clean_end(ended: &Ended, stub: &Stub, what: &str) {
    stub.assert_no_foreign_credential();
    assert!(ended.success, "{what}: {ended:?}");
}

/// What api and review say to web in `clienta`; neither is anywhere else.
const OWN_SAYS: &str = "e2e test: api asks web to rerun the migration";
const PEER_SAYS: &str = "e2e test: review asks web for the deploy log";

/// Bob joins `clienta`; Ada also has `clientb`, bound to `~/work/clientB`. Claude Code sessions of
/// web run in `~/work/clientA/web` and `~/work/clientB/web`, each with `--agent web`; each stub calls
/// `inbox`, then holds its turn. api and review each send web in `clienta` a message; then each
/// stub runs `true`. In clientA's session, the request after `true` holds api's message as its line
/// from api at `own`, and of review's only a line that names `inbox`; the stub calls `inbox`, and the
/// request after it holds review's line at `peer`, after a warning line that names data. Its last
/// request holds each text once. clientB's session's requests hold neither text. Both sessions end
/// with success.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn busy_session_gets_own_text_after_its_tool_and_a_peer_s_only_as_the_signal() {
    let Some(claude) = claude_code() else { return };
    let solo = Solo::new(4_913_101_001, "ada-runs-claude-code");
    let bob = Bob::join(&solo, 4_913_101_002, "bob-writes-to-claude-code");
    solo.with_client_b();
    let (dir_a, dir_b) = (solo.dir(WEB), dir_of(solo.machine(), "work/clientB/web"));
    let stub_a = Stub::start(vec![inbox(), Step::Bash("true"), inbox()], Some(1));
    let stub_b = Stub::start(vec![inbox(), Step::Bash("true")], Some(1));
    let session_a = Claude::start(&claude, solo.machine(), &dir_a, Some(WEB), &stub_a);
    let session_b = Claude::start(&claude, solo.machine(), &dir_b, Some(WEB), &stub_b);
    stub_a.wait_held();
    stub_b.wait_held();

    let own = solo.api_sends_web(OWN_SAYS);
    let peer = bob.sends(&solo, WEB, PEER_SAYS);
    stub_b.release();
    let ended_b = session_b.finish();
    stub_a.release();
    let ended_a = session_a.finish();

    assert_clean_end(&ended_a, &stub_a, "clientA's session");
    assert_clean_end(&ended_b, &stub_b, "clientB's session");
    let after_tool = after_call(&turn_at(&stub_a, 2), 1);
    let line = line_in(&after_tool, OWN_SAYS);
    assert_from(&line, own, (&solo.short(API), ADA_HOST, OWN));
    assert!(
        after_tool.contains(INBOX_WORD),
        "no signal after the tool: {after_tool}"
    );
    assert!(
        !after_tool.contains(PEER_SAYS),
        "the peer's text after the tool: {after_tool}"
    );
    let after_inbox = after_call(&turn_at(&stub_a, 3), 2);
    assert_from(
        &line_in(&after_inbox, PEER_SAYS),
        peer,
        (&bob.review(), BOB_HOST, PEER),
    );
    let warned = after_inbox
        .lines()
        .take_while(|line| !line.contains(PEER_SAYS));
    assert!(
        warned.into_iter().any(|line| holds_word(line, DATA)),
        "no warning line before the peer's: {after_inbox}"
    );
    let last = conversation(stub_a.turns().last().expect("a request"));
    assert_eq!(
        last.matches(OWN_SAYS).count(),
        1,
        "api's text not once: {last}"
    );
    assert_eq!(
        last.matches(PEER_SAYS).count(),
        1,
        "review's text not once: {last}"
    );
    for body in stub_b.turns() {
        let seen = conversation(&body);
        assert!(
            !seen.contains(OWN_SAYS) && !seen.contains(PEER_SAYS),
            "clientB got it: {seen}"
        );
    }
}

/// What api says to web as its turn ends; it is nowhere else.
const AT_THE_END: &str = "e2e test: api asks web to write the changelog";

/// A Claude Code session of web runs in `~/work/clientA/web`; its stub calls `inbox`, then holds its
/// reply that ends the turn. api sends web a message; the stub replies. A later request of the same
/// turn, the one `Stop` kept going, holds api's message as its line from api at `own`, which the held
/// one did not; the session ends with success.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn message_sent_as_the_turn_ends_reaches_it_by_stop_and_the_turn_goes_on() {
    let Some(claude) = claude_code() else { return };
    let solo = Solo::new(4_913_102_001, "ada-ends-a-claude-code-turn");
    let stub = Stub::start(vec![inbox()], Some(1));
    let session = Claude::start(&claude, solo.machine(), &solo.dir(WEB), Some(WEB), &stub);
    stub.wait_held();

    let msg_id = solo.api_sends_web(AT_THE_END);
    stub.release();
    let ended = session.finish();

    assert_clean_end(&ended, &stub, "the session");
    let ends: Vec<Value> = (stub.turns().into_iter())
        .filter(|body| played(body) == 1)
        .collect();
    assert!(
        ends.len() >= 2,
        "the turn did not go on: {} requests at its end",
        ends.len()
    );
    assert!(
        !conversation(&ends[0]).contains(AT_THE_END),
        "the message before Stop"
    );
    let line = line_in(&conversation(&ends[ends.len() - 1]), AT_THE_END);
    assert_from(&line, msg_id, (&solo.short(API), ADA_HOST, OWN));
}

/// What the session says to api, and api to the session's agent; neither is anywhere else.
const HELLO: &str = "e2e test: the unnamed session says hello to api";
const TO_SESSION: &str = "e2e test: api answers the unnamed session";

/// api's MCP session runs. A Claude Code session runs in `~/work/clientA/session` with neither its
/// `skald mcp` nor its hooks given `--agent`; its stub calls the send tool `to` api, then holds its
/// turn. api's `inbox` shows the message from the agent the session's `skald mcp` registered; api
/// sends that agent a message; the stub runs `true`. The request after `true` holds api's message
/// as its line from api at `own`.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn hooks_without_agent_act_as_the_agent_claude_code_s_skald_mcp_registers() {
    let Some(claude) = claude_code() else { return };
    let solo = Solo::new(4_913_103_001, "ada-runs-an-unnamed-session");
    let mut api = solo.session(API);
    let hello = Step::Skald(
        "send",
        json!({ TO_FIELD: solo.short(API), TEXT_FIELD: HELLO }),
    );
    let stub = Stub::start(vec![hello, Step::Bash("true")], Some(1));
    let dir = solo.dir("session");
    let session = Claude::start(&claude, solo.machine(), &dir, None, &stub);
    stub.wait_held();
    let registered = agent_line_of(&api.inbox(), HELLO).from;
    assert!(
        registered.starts_with(&format!("{}/", solo.talk.ada.handle)),
        "{registered}"
    );

    let msg_id = api_sends(&solo, &registered, TO_SESSION);
    stub.release();
    let ended = session.finish();

    assert_clean_end(&ended, &stub, "the session");
    let after_tool = after_call(&turn_at(&stub, 2), 1);
    assert_from(
        &line_in(&after_tool, TO_SESSION),
        msg_id,
        (&solo.short(API), ADA_HOST, OWN),
    );
}
