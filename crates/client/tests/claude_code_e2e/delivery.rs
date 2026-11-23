//! What the tests of the delivery settings share (`docs/protocol/delivery.md`, "Device to agent";
//! `docs/agent-sessions.md`): the person's commands that set and list the delivery mode and the
//! text setting per channel and sender trust level, the wakes the listing gives per channel, and
//! messages sent in `clienta`'s other channel, the talk's, which Bob joins by a fixture revision.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::{ChannelId, MessageId};

use crate::client::Session;
use crate::cloud::WAIT;
use crate::fixtures::{CHANNEL, SEND, TO};
use crate::hooks::{Bob, REVIEW};
use crate::kit::{Change, latest, received, revise};
use crate::lines::{JSON_FLAG, agent_line, json_objects, refers_to};
use crate::machine::{Finished, Machine};
use crate::receipts::{READ, read_lines, shows_read};
use crate::senders::{DATA, OWN, PEER, holds_word};
use crate::solo::{API, Solo, WEB, assert_got};
use crate::talk::{BOB_HOST, one_message_sent};
use crate::world::AGENT;

/// The person's commands: set a channel's mode for a level, set its text setting for a level, and
/// list the settings and wakes of each channel of the working directory's space.
pub const SET_MODE: &str = "set-delivery-mode";
pub const SET_TEXT: &str = "set-delivery-text";
pub const LIST: &str = "delivery-modes";

/// The flag of `skald set-delivery-mode` that gives `batch` its window.
pub const WINDOW: &str = "--window";

/// The delivery modes.
pub const NOW: &str = "now";
pub const BATCH: &str = "batch";
pub const PULL: &str = "pull";

/// The text settings of `peer` and `service`.
pub const SIGNAL: &str = "signal";
pub const INLINE: &str = "inline";

/// The trust level of a service agent, beside [`OWN`] and [`PEER`].
pub const SERVICE: &str = "service";

/// The levels a setting is kept for.
pub const LEVELS: [&str; 3] = [OWN, PEER, SERVICE];

/// The word of the listing's line that gives a channel's wakes.
pub const WAKES: &str = "wakes";

/// Runs the person's `skald <args>` on Ada's machine in `~/work/clientA`; fails unless it succeeds.
#[track_caller]
pub fn person_sets(solo: &Solo, args: &[&str]) -> Finished {
    let run = solo.person_runs(args);
    assert!(run.success, "{args:?}: {run:?}");
    run
}

/// The person sets the delivery mode of `level` in `channel` to `mode`, with `window`, such as
/// `10s`, if any.
#[track_caller]
pub fn set_mode(solo: &Solo, channel: ChannelId, level: &str, mode: &str, window: Option<&str>) {
    let channel = channel.to_string();
    let mut args = vec![SET_MODE, channel.as_str(), level, mode];
    args.extend(window.map(|window| [WINDOW, window]).into_iter().flatten());
    person_sets(solo, &args);
}

/// The person sets the text setting of `level` in `channel` to `text`.
#[track_caller]
pub fn set_text(solo: &Solo, channel: ChannelId, level: &str, text: &str) {
    let channel = channel.to_string();
    person_sets(solo, &[SET_TEXT, channel.as_str(), level, text]);
}

/// The person's `skald delivery-modes --json` in `~/work/clientA`: one object per channel of
/// `clienta`; fails unless it succeeds.
#[track_caller]
pub fn listed(solo: &Solo) -> Vec<Value> {
    json_objects(&person_sets(solo, &[LIST, JSON_FLAG]).stdout)
}

/// The object of `channel` among `listed`; fails unless there is one.
#[track_caller]
pub fn of_channel(listed: &[Value], channel: ChannelId) -> &Value {
    let id = channel.to_string();
    let of: Vec<&Value> = (listed.iter())
        .filter(|object| object["channel"].as_str() == Some(id.as_str()))
        .collect();
    match of.as_slice() {
        [one] => one,
        _ => panic!("not one object of {id}: {listed:#?}"),
    }
}

/// The mode of `level` in a channel's `object`, and its window in seconds, if any.
#[track_caller]
pub fn mode_of(object: &Value, level: &str) -> (String, Option<u64>) {
    let setting = &object[level];
    let mode = (setting["mode"].as_str())
        .unwrap_or_else(|| panic!("no mode of {level}: {object}"))
        .to_owned();
    (mode, setting["window"].as_u64())
}

/// The text setting of `level` in a channel's `object`.
#[track_caller]
pub fn text_of(object: &Value, level: &str) -> String {
    (object[level]["text"].as_str())
        .unwrap_or_else(|| panic!("no text setting of {level}: {object}"))
        .to_owned()
}

/// The wakes of a channel's `object`.
#[track_caller]
pub fn wakes_of(object: &Value) -> u64 {
    (object[WAKES].as_u64()).unwrap_or_else(|| panic!("no {WAKES}: {object}"))
}

/// The wakes the person's listing gives for `channel`.
#[track_caller]
pub fn wakes(solo: &Solo, channel: ChannelId) -> u64 {
    wakes_of(of_channel(&listed(solo), channel))
}

/// Runs the person's listing every half second until it gives `count` wakes for `channel`, for up
/// to [`WAIT`]; fails if it gives more, or never `count`.
#[track_caller]
pub fn assert_wakes(solo: &Solo, channel: ChannelId, count: u64) {
    let deadline = Instant::now() + WAIT;
    loop {
        let got = wakes(solo, channel);
        assert!(got <= count, "{got} wakes of {channel}, not {count}");
        if got == count {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{got} wakes of {channel} after {WAIT:?}, not {count}"
        );
        thread::sleep(Duration::from_millis(500));
    }
}

/// `clienta`'s channel beside its default one: the talk's, which holds Ada's device alone until
/// Bob joins it.
pub fn other_channel(solo: &Solo) -> ChannelId {
    solo.talk.channel
}

/// `agent` sends web `text` in `channel` with `skald --agent <agent> send --to <web> --channel
/// <channel>` on `machine` in `dir`; fails unless it succeeds and the front saw one message in
/// `channel` meanwhile. Gives its `msg_id`.
#[track_caller]
pub fn sends_in(
    solo: &Solo,
    (machine, dir, agent): (&Machine, &Path, &str),
    channel: ChannelId,
    text: &str,
) -> MessageId {
    let mark = solo.mark();
    let (to, at) = (solo.short(WEB), channel.to_string());
    let args = [AGENT, agent, SEND, TO, &to, CHANNEL, &at, text];
    let sent = machine.start_in(dir, &args).finish();
    assert!(sent.success, "{sent:?}");
    one_message_sent(&solo.talk.front, channel, mark)
}

/// api sends web `text` in `channel` from its folder, as [`sends_in`] says.
#[track_caller]
pub fn api_sends_in(solo: &Solo, channel: ChannelId, text: &str) -> MessageId {
    sends_in(solo, (solo.machine(), &solo.dir(API), API), channel, text)
}

/// Bob's agent review sends web `text` in `channel` from Bob's machine, as [`sends_in`] says.
#[track_caller]
pub fn review_sends_in(solo: &Solo, bob: &Bob, channel: ChannelId, text: &str) -> MessageId {
    let from = (&bob.person.machine, bob.dir.as_path(), REVIEW);
    sends_in(solo, from, channel, text)
}

/// Fails unless `shown` holds `text` once, as the line of review's message `msg_id`, by Bob's
/// machine, at `peer`, wherever in a line the label starts, after a line that names data: the
/// text of a `peer` message under `inline`.
#[track_caller]
pub fn assert_inline(shown: &str, text: &str, msg_id: MessageId, bob: &Bob) {
    assert_eq!(shown.matches(text).count(), 1, "not once: {shown}");
    let needle = format!(": {text}");
    let end = shown.find(&needle).map(|at| at + needle.len());
    let end = end.unwrap_or_else(|| panic!("{text:?} is no message's text: {shown}"));
    let start =
        (shown[..end].rfind('[')).unwrap_or_else(|| panic!("{text:?} has no label: {shown}"));
    let label = &shown[start..end];
    let got = agent_line(label).unwrap_or_else(|| panic!("not a message line: {label:?}"));
    assert!(refers_to(&got.reference, msg_id), "{got:?} is not {msg_id}");
    let from = (got.from.as_str(), got.device.as_str(), got.level.as_str());
    assert_eq!(from, (bob.review().as_str(), BOB_HOST, PEER), "{shown}");
    let warned = (shown[..start].lines())
        .any(|line| !line.trim_start().starts_with('[') && holds_word(line, DATA));
    assert!(warned, "no line that names {DATA} before {text:?}: {shown}");
}

/// Bob, in `clienta` by [`Bob::join`], joins its other channel too, by a fixture revision Ada's
/// device signs; Ada's machine and Bob's take it in.
pub fn bob_joins_other_channel(solo: &Solo, bob: &Bob) {
    let (cloud, ada) = (solo.talk.cloud(), &solo.talk.ada);
    let change = Change {
        join: vec![(other_channel(solo), &bob.person)],
        ..Change::default()
    };
    revise(
        cloud,
        &latest(cloud, &ada.token, solo.talk.space.id),
        ada,
        &change,
    );
    for machine in [
        solo.machine(),
        &bob.person.machine,
        solo.machine(),
        &bob.person.machine,
    ] {
        received(machine);
    }
}

/// api's MCP session sends web ten messages, one after another, each `<prefix> <n> of ten`; gives
/// each one's text and `msg_id`, in order.
#[track_caller]
pub fn api_sends_ten(solo: &Solo, prefix: &str) -> Vec<(String, MessageId)> {
    let mut api = solo.session(API);
    let to = solo.short(WEB);
    (1..=10)
        .map(|n| {
            let text = format!("{prefix} {n} of ten");
            let msg_id = solo.sends(&mut api, &to, &text);
            (text, msg_id)
        })
        .collect()
}

/// Runs the person's `skald recv` on `machine` until the runs showed `read` of each of `msg_ids`,
/// as [`shows_read`] does; fails unless they showed each once and the next run shows none. One run
/// may show the `read` of several.
#[track_caller]
pub fn shows_reads_once(machine: &Machine, msg_ids: &[MessageId]) {
    let mut shown = String::new();
    for msg_id in msg_ids {
        if read_lines(&shown, *msg_id) == 0 {
            shown += &shows_read(machine, *msg_id);
        }
    }
    let next = received(machine).shown();
    for msg_id in msg_ids {
        assert_eq!(read_lines(&shown, *msg_id), 1, "{READ} not once: {shown}");
        assert_eq!(read_lines(&next, *msg_id), 0, "{READ} again: {next}");
    }
}

/// Fails unless the listing gives one wake for each of `clienta`'s two channels, and web's
/// `session`'s `inbox` gives each of `messages`, review's texts and `msg_id`s, at `peer`.
#[track_caller]
pub fn one_wake_each_and_inbox_gives(
    solo: &Solo,
    session: &mut Session,
    bob: &Bob,
    messages: [(&str, MessageId); 2],
) {
    for channel in [other_channel(solo), solo.default_channel()] {
        assert_wakes(solo, channel, 1);
    }
    let shown = session.inbox();
    for (text, msg_id) in messages {
        assert_got(&shown, text, msg_id, (&bob.review(), BOB_HOST, PEER));
    }
}
