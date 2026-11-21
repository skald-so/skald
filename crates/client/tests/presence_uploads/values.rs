//! What the tests read: Ada's values in a channel as Bob's device gets them from the server and
//! opens them, the uploads the relay saw, and Ada's runs and sessions.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_crypto::PresenceError;
use skald_proto::{
    AgentState, AgentStatus, Cbor as _, ChannelId, PROOF_HEADER, PROTOCOL_VERSION, Presence,
    PresenceBody, SpaceId, VERSION_HEADER,
};

use crate::agents::{Agents, CLAUDE};
use crate::client::{CLIENT, MCP, Session};
use crate::cloud::{WAIT, agent, get, unb64};
use crate::fixtures::cert_of;
use crate::helped::Helped;
use crate::machine::{Finished, Machine, Run};
use crate::presence::{presence_path, proof};
use crate::talk::Talk;
use crate::world::{AGENT, now};

/// The command that sets an agent's state and note, its flag of the note, and the states it takes.
pub const SET_STATUS: &str = "set-status";
pub const NOTE_FLAG: &str = "--note";

/// The MCP tool that sets its session's state and note, and its arguments.
pub const STATUS_TOOL: &str = "set_status";
pub const STATE_FIELD: &str = "state";
pub const NOTE_FIELD: &str = "note";

/// The longest note, in characters, as the refusal names it.
pub const NOTE_MAX: usize = 128;
pub const NOTE_LIMIT: &str = "128";

/// The setting of how far after an upload its value's `expires` is, and that span when unset.
pub const EXPIRES_SETTING: &str = "SKALD_PRESENCE_EXPIRES";
pub const EXPIRES_IN: u64 = 900;

/// More agents of Ada's device, beside those of the tests of handing messages over.
pub const GEMINI: &str = "gemini-p4r8vd";
pub const AIDER: &str = "aider-k2n6ws";

/// The line that starts the request uploading a value in `at`'s channel, for the relay's count.
pub fn upload_request(at: (SpaceId, ChannelId)) -> String {
    format!("PUT {} ", presence_path(at))
}

/// How many values the machines uploaded in `at`'s channel from `since` on.
pub fn uploads(talk: &Talk, at: (SpaceId, ChannelId), since: Instant) -> usize {
    talk.cloud().count_sent(since, &upload_request(at))
}

/// The values of `at`'s channel the server gives Bob's device, the talk's first played one, on a
/// request that proves it; fails unless the server gives them.
#[track_caller]
pub fn values_in(talk: &Talk, at: (SpaceId, ChannelId)) -> Vec<Presence> {
    let cloud = talk.cloud();
    let bob = &talk.played[0].person;
    let header = proof(cloud, bob);
    let path = presence_path(at);
    let mut answer = get(|| {
        agent()
            .get(format!("{}{path}", cloud.url))
            .header(VERSION_HEADER, PROTOCOL_VERSION.to_string())
            .header("authorization", format!("Bearer {}", bob.token))
            .header(PROOF_HEADER, &header)
            .call()
    })
    .unwrap_or_else(|err| panic!("GET {path}: {err}"));
    let code = answer.status().as_u16();
    let text = answer.body_mut().read_to_string().unwrap_or_default();
    assert_eq!(code, 200, "GET {path} as Bob's device: {text}");
    let listed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let Some(items) = listed["values"].as_array() else {
        panic!("GET {path}: no values in {text}");
    };
    items
        .iter()
        .map(|item| {
            let bytes = unb64(item.as_str().expect("a value in base64url"));
            Presence::from_cbor(&bytes)
                .unwrap_or_else(|err| panic!("a value of {path} that does not decode: {err}"))
        })
        .collect()
}

/// Ada's device's value in `at`'s channel, as the server gives it to Bob's device, if any.
#[track_caller]
pub fn ada_s_value(talk: &Talk, at: (SpaceId, ChannelId)) -> Option<Presence> {
    let device = talk.ada.device;
    values_in(talk, at)
        .into_iter()
        .find(|value| value.device == device)
}

/// `value` of Ada's device opened by Bob's device with its history key of the value's epoch and
/// the signing key of the certificate of Ada's device, checked at `at_time`; fails if Bob's device
/// holds no key of that epoch.
#[track_caller]
pub fn opened(talk: &Talk, value: &Presence, at_time: u64) -> Result<PresenceBody, PresenceError> {
    let bob = &talk.played[0];
    let Some(key) = bob.mls.history_key(value.channel, value.epoch) else {
        panic!("Bob's device holds no key of epoch {}", value.epoch);
    };
    let signing = cert_of(talk.cloud(), &talk.ada).body().signing;
    key.presence_key().open(value, &signing, at_time)
}

/// The body of `value`, Ada's, as [`opened`] gives it now; fails unless it passes every check.
#[track_caller]
pub fn body_of(talk: &Talk, value: &Presence) -> PresenceBody {
    let body = opened(talk, value, now())
        .unwrap_or_else(|err| panic!("Ada's value in {} is dropped: {err}", value.channel));
    assert_eq!(
        body.device, talk.ada.device,
        "the device of the value: {body:?}"
    );
    assert_eq!(
        body.channel, value.channel,
        "the channel of the value: {body:?}"
    );
    assert!(
        body.serves.is_empty(),
        "a value that serves queues: {body:?}"
    );
    body
}

/// The body of Ada's value in `at`'s channel, as [`body_of`] gives it, if the server holds one.
#[track_caller]
pub fn ada_s_body(talk: &Talk, at: (SpaceId, ChannelId)) -> Option<PresenceBody> {
    ada_s_value(talk, at).map(|value| body_of(talk, &value))
}

/// Waits for up to [`WAIT`] until Ada's value in `at`'s channel passes `done`, Bob's device taking
/// in its mailbox whenever the value is of an epoch it holds no key of; fails unless one does.
/// Gives the value and its body.
#[track_caller]
pub fn await_value(
    talk: &mut Talk,
    at: (SpaceId, ChannelId),
    what: &str,
    done: impl Fn(&Presence, &PresenceBody) -> bool,
) -> (Presence, PresenceBody) {
    let deadline = Instant::now() + WAIT;
    let mut last = None;
    loop {
        if let Some(value) = ada_s_value(talk, at) {
            if (talk.played[0].mls)
                .history_key(value.channel, value.epoch)
                .is_none()
            {
                talk.played[0].take_in(&talk.world.cloud);
            }
            let body = body_of(talk, &value);
            if done(&value, &body) {
                return (value, body);
            }
            last = Some(body);
        }
        assert!(
            Instant::now() < deadline,
            "no value of Ada's in {} {what} within {WAIT:?}; the last: {last:?}",
            at.1
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// As [`await_value`] for each of `ats` in turn; gives the bodies, in the order of `ats`.
#[track_caller]
pub fn await_in_each(
    talk: &mut Talk,
    ats: &[(SpaceId, ChannelId)],
    what: &str,
    done: impl Fn(&PresenceBody) -> bool,
) -> Vec<PresenceBody> {
    (ats.iter())
        .map(|at| await_value(talk, *at, what, |_, body| done(body)).1)
        .collect()
}

/// The status of `address` in `body`, if it lists it.
pub fn status_in<'a>(body: &'a PresenceBody, address: &str) -> Option<&'a AgentStatus> {
    (body.agents.iter()).find(|status| status.agent.to_string() == address)
}

/// The status of `address` in `body`; fails unless it lists it once.
#[track_caller]
pub fn listed<'a>(body: &'a PresenceBody, address: &str) -> &'a AgentStatus {
    let times = (body.agents.iter())
        .filter(|status| status.agent.to_string() == address)
        .count();
    assert_eq!(times, 1, "{address} not listed once: {body:?}");
    status_in(body, address).expect("listed")
}

/// Whether `body` lists `address` in `state`.
pub fn shows(body: &PresenceBody, address: &str, state: AgentState) -> bool {
    status_in(body, address).is_some_and(|status| status.state == state)
}

/// Whether `body` lists `address` in `state` with `note`.
pub fn shows_with_note(body: &PresenceBody, address: &str, state: AgentState, note: &str) -> bool {
    status_in(body, address)
        .is_some_and(|status| status.state == state && status.note.as_deref() == Some(note))
}

/// The addresses `body` lists, in order.
pub fn addresses(body: &PresenceBody) -> Vec<String> {
    (body.agents.iter())
        .map(|status| status.agent.to_string())
        .collect()
}

/// Ada's agents in `~/work/acme` and the channels of `acme`: the default one and the talk's.
pub fn acme_channels(agents: &Agents) -> [(SpaceId, ChannelId); 2] {
    let talk = &agents.talk;
    [(talk.space.id, talk.space.default), talk.at()]
}

/// Runs `skald <args>` on `machine` in `dir`, one-shot, with `settings`; gives how it finished.
pub fn run_with(
    machine: &Machine,
    dir: &Path,
    settings: &[(&str, &str)],
    args: &[&str],
) -> Finished {
    let mut command = machine.command_in(dir, args);
    command.envs(settings.iter().copied());
    Run::spawn(&mut command).finish()
}

/// Runs `skald --agent <agent> <args>` one-shot in `~/work/acme`; fails unless it succeeds.
#[track_caller]
pub fn ran_as(agents: &Agents, agent_name: &str, args: &[&str]) -> Finished {
    let mut all = vec![AGENT, agent_name];
    all.extend_from_slice(args);
    let finished = run_with(&agents.talk.ada.machine, &agents.folder, &[], &all);
    assert!(finished.success, "{all:?}: {finished:?}");
    finished
}

/// Starts `skald --agent <agent> mcp` of `helped`'s copy in `dir`, with `settings`, and
/// initializes it as the MCP client named `client`.
pub fn helped_session(
    helped: &Helped,
    dir: &Path,
    settings: &[(&str, &str)],
    (agent_name, client): (&str, &str),
) -> Session {
    let mut command = helped.command_in(dir, settings, &[AGENT, agent_name, MCP]);
    Session::start(&mut command, client)
}

/// As [`helped_session`] in `~/work/acme`, without settings, as `claude-code`.
pub fn claude_code_session(helped: &Helped, agents: &Agents, agent_name: &str) -> Session {
    helped_session(helped, &agents.folder, &[], (agent_name, CLIENT))
}

/// As [`helped_session`] of claude in `~/work/acme`, as `claude-code`, with the setting `name` of
/// `seconds`.
pub fn claude_session_with(
    helped: &Helped,
    agents: &Agents,
    (name, seconds): (&str, u64),
) -> Session {
    let value = seconds.to_string();
    let settings = [(name, value.as_str())];
    helped_session(helped, &agents.folder, &settings, (CLAUDE, CLIENT))
}

/// Waits for up to [`WAIT`] until Ada's value in each channel of `acme` lists `agent_name`
/// `available`; gives its address and the channels.
#[track_caller]
pub fn await_available(
    agents: &mut Agents,
    agent_name: &str,
) -> (String, [(SpaceId, ChannelId); 2]) {
    let address = agents.address(agent_name);
    let acme = acme_channels(agents);
    await_in_each(
        &mut agents.talk,
        &acme,
        "with the agent available",
        |body| shows(body, &address, AgentState::Available),
    );
    (address, acme)
}

/// The processes of `helped`'s copy beside the servers of `sessions`: its helpers.
pub fn helpers_beside(helped: &Helped, sessions: &[&Session]) -> Vec<u32> {
    (helped.helpers().into_iter())
        .filter(|pid| sessions.iter().all(|session| session.id() != *pid))
        .collect()
}

/// The one helper of `helped`'s copy beside `sessions`, once the other processes of the copy have
/// exited; fails unless there is one within [`WAIT`].
#[track_caller]
pub fn one_helper_beside(helped: &Helped, sessions: &[&Session]) -> u32 {
    let started = Instant::now();
    let mut beside = helpers_beside(helped, sessions);
    while beside.len() != 1 {
        assert!(
            started.elapsed() < WAIT,
            "not one helper beside the sessions: {beside:?}"
        );
        thread::sleep(Duration::from_millis(100));
        beside = helpers_beside(helped, sessions);
    }
    beside[0]
}

/// Calls `set_status` in `session` with `state` and, if given, `note`.
pub fn set_status_tool(
    session: &mut Session,
    state: &str,
    note: Option<&str>,
) -> crate::client::Called {
    let mut arguments = json!({ STATE_FIELD: state });
    if let Some(note) = note {
        arguments[NOTE_FIELD] = json!(note);
    }
    session.call(STATUS_TOOL, arguments)
}

/// A note of `length` ASCII characters, none elsewhere.
pub fn note_of(length: usize) -> String {
    let words = "presence test: reviewing the migration of the invoices table ";
    words.chars().cycle().take(length).collect()
}
