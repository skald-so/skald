//! What the tests share: runs of `skald message-status`, calls of the `message_status` tool, and
//! what their JSON says.

use std::path::Path;

use serde_json::{Value, json};
use skald_proto::{ChannelId, DeviceId, MessageId, SpaceId};

use crate::agents::CLAUDE;
use crate::client::{MESSAGE_FIELD, Session};
use crate::lines::{JSON_FLAG, json_objects, lines_referring_to, reference};
use crate::machine::{Finished, Machine};
use crate::replying::{BOB_CLAUDE, Writes, agent_of, bob_writes, handles, run_in};
use crate::senders::holds_word;
use crate::talk::Talk;
use crate::world::now;

/// The command, and the MCP tool.
pub const MESSAGE_STATUS: &str = "message-status";
pub const STATUS_TOOL: &str = "message_status";

/// The statuses of a recipient device, as the CLI and the JSON name them.
pub const STORED: &str = "stored";
pub const EXPIRED: &str = "expired";

/// How many characters of a message's reference the tests give: more than the fewest, so no other
/// message the device keeps shares them.
const GIVEN: usize = 6;

/// `msg_id` as `#<ref>`, with [`GIVEN`] characters of its reference.
pub fn hashed(msg_id: MessageId) -> String {
    format!("#{}", bare(msg_id))
}

/// `msg_id` as `<ref>`, without the `#`.
pub fn bare(msg_id: MessageId) -> String {
    reference(msg_id)[..GIVEN].to_owned()
}

/// Runs `skald [--agent <agent>] message-status <flags> <message>` on `machine` in `dir`.
pub fn status_in(
    machine: &Machine,
    (dir, agent): (&Path, Option<&str>),
    flags: &[&str],
    message: &str,
) -> Finished {
    let mut args = flags.to_vec();
    args.push(message);
    run_in(machine, (dir, agent), MESSAGE_STATUS, &args)
}

/// As [`status_in`], without flags; fails unless it succeeds. Gives what it showed.
#[track_caller]
pub fn shown_status(machine: &Machine, at: (&Path, Option<&str>), message: &str) -> String {
    let finished = status_in(machine, at, &[], message);
    assert!(finished.success, "{message}: {finished:?}");
    finished.stdout
}

/// As [`status_in`], with `--json`; fails unless it succeeds and prints one JSON object. Gives it.
#[track_caller]
pub fn json_status(machine: &Machine, at: (&Path, Option<&str>), message: &str) -> Value {
    let finished = status_in(machine, at, &[JSON_FLAG], message);
    one_object(&finished, message)
}

/// The one JSON object `finished`, a run of `skald message-status --json <message>`, printed;
/// fails unless it succeeded and printed one.
#[track_caller]
pub fn one_object(finished: &Finished, message: &str) -> Value {
    assert!(finished.success, "{message}: {finished:?}");
    match <[Value; 1]>::try_from(json_objects(&finished.stdout)) {
        Ok([object]) => object,
        Err(objects) => panic!("{message}: not one JSON object: {objects:?}"),
    }
}

/// `session` calls `message_status` with `message`; fails if it is an error. Gives its text.
#[track_caller]
pub fn tool_status(session: &mut Session, message: &str) -> String {
    let called = session.call(STATUS_TOOL, json!({ MESSAGE_FIELD: message }));
    assert!(!called.error, "{message}: {called:?}: {}", session.errors());
    called.text
}

/// Fails unless `object` is of the message `msg_id` in `channel`, sent by `device` no earlier
/// than `since`, in Unix seconds, and no later than now; gives its time.
#[track_caller]
pub fn assert_of(
    object: &Value,
    msg_id: MessageId,
    (channel, device): (ChannelId, DeviceId),
    since: u64,
) -> u64 {
    for (field, value) in [
        ("msg_id", msg_id.to_string()),
        ("channel", channel.to_string()),
        ("device", device.to_string()),
    ] {
        assert_eq!(
            object[field].as_str(),
            Some(value.as_str()),
            "{field}: {object}"
        );
    }
    let short = object["ref"].as_str().unwrap_or_default();
    assert!(
        reference(msg_id).starts_with(short) && short.len() >= 4,
        "ref: {object}"
    );
    let sent = object["timestamp"].as_u64();
    let sent = sent.unwrap_or_else(|| panic!("no timestamp: {object}"));
    assert!(
        (since..=now()).contains(&sent),
        "timestamp {sent} not since {since}: {object}"
    );
    sent
}

/// A recipient device of a message the device sent, as `recipients` gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipient {
    pub device: String,
    pub device_name: Option<String>,
    pub account: Option<String>,
    pub status: String,
    pub timestamp: Option<u64>,
}

/// The `recipients` of `object`; fails unless it is an array of objects, each with `device` and
/// `status`.
#[track_caller]
pub fn recipients(object: &Value) -> Vec<Recipient> {
    let Some(all) = object["recipients"].as_array() else {
        panic!("no recipients: {object}");
    };
    (all.iter())
        .map(|one| Recipient {
            device: (one["device"].as_str())
                .unwrap_or_else(|| panic!("a recipient without device: {object}"))
                .to_owned(),
            device_name: one["device_name"].as_str().map(str::to_owned),
            account: one["account"].as_str().map(str::to_owned),
            status: (one["status"].as_str())
                .unwrap_or_else(|| panic!("a recipient without status: {object}"))
                .to_owned(),
            timestamp: one["timestamp"].as_u64(),
        })
        .collect()
}

/// The recipient `device` of `object`; fails unless there is one.
#[track_caller]
pub fn recipient(object: &Value, device: DeviceId) -> Recipient {
    let id = device.to_string();
    let found: Vec<Recipient> = (recipients(object).into_iter())
        .filter(|one| one.device == id)
        .collect();
    match <[Recipient; 1]>::try_from(found) {
        Ok([one]) => one,
        Err(found) => panic!("not one recipient {id}: {found:?} in {object}"),
    }
}

/// The time of the `to` agent's `status`, `read` or `replied`, in `object`; `None` until it comes.
/// Fails unless `agent` is an object.
#[track_caller]
pub fn agent_time(object: &Value, status: &str) -> Option<u64> {
    assert!(object["agent"].is_object(), "no agent: {object}");
    object["agent"][status].as_u64()
}

/// Fails unless `shown` holds a reference of `msg_id`: `#` and its leading characters.
#[track_caller]
pub fn assert_refers(shown: &str, msg_id: MessageId) {
    let found = lines_referring_to(shown, msg_id);
    assert!(
        !found.is_empty(),
        "no #{}... in: {shown}",
        &reference(msg_id)[..4]
    );
}

/// Fails unless `shown` holds each of `texts`.
#[track_caller]
pub fn assert_holds(shown: &str, texts: &[&str]) {
    for text in texts {
        assert!(shown.contains(text), "no {text:?} in: {shown}");
    }
}

/// Fails unless a line of `shown` holds each of `words` as a word, in any case.
#[track_caller]
pub fn assert_words(shown: &str, words: &[&str]) {
    for word in words {
        assert!(
            shown.lines().any(|line| holds_word(line, word)),
            "no {word:?} in: {shown}"
        );
    }
}

/// What Bob's device sets in a message to Ada's claude beside its text: its envelope's `msg_id`
/// and `ttl`, if given.
#[derive(Default)]
pub struct Asked {
    pub msg_id: Option<MessageId>,
    pub ttl: Option<u64>,
}

/// Bob's device says `text`, as message `counter`, to Ada's claude in the channel `at` of the
/// space named `space`, as Bob's claude there, with `hops` 1 and what `asked` sets; fails unless
/// the server stores it. Gives its `msg_id`.
#[track_caller]
pub fn bob_asks_claude(
    talk: &mut Talk,
    (space, at): (&str, (SpaceId, ChannelId)),
    (counter, text): (u64, &str),
    asked: Asked,
) -> MessageId {
    let (bob, ada) = handles(talk);
    let from = agent_of(&bob, space, BOB_CLAUDE);
    let to = agent_of(&ada, space, CLAUDE);
    let mut writes = Writes::new(&from, Some(&to), counter, text).hops(1);
    (writes.msg_id, writes.ttl) = (asked.msg_id, asked.ttl);
    bob_writes(talk, at, &writes)
}
