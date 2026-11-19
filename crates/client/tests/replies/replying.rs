//! What the tests of replies share: what Bob's device writes in a channel of the talk, as one of
//! Bob's agents or as Bob, replies included, and what it reads of Ada's machine's messages; the
//! runs of `skald send` and `skald reply` on Ada's machine; and what they show of a reply.

use std::path::Path;

use serde_json::Value;
use skald_proto::{Body, Cbor as _, ChannelId, Content, Envelope, MessageId, RawEnvelope, SpaceId};

use crate::agents::Agents;
use crate::fixtures::SEND;
use crate::lines::{lines_referring_to, reference};
use crate::machine::{Finished, Machine};
use crate::played::mls_envelope;
use crate::talk::{Line, Talk, stored};
use crate::world::{AGENT, HOST};

/// The command that replies, and the status a reply gives the message it answers.
pub const REPLY: &str = "reply";
pub const REPLIED: &str = "replied";

/// The flags of `skald send`.
pub const TO: &str = "--to";
pub const ALL: &str = "--all";
pub const CHANNEL: &str = "--channel";

/// Bob's agents, by name.
pub const BOB_CLAUDE: &str = "claude-p6v3rk";
pub const BOB_CODEX: &str = "codex-w8j2tn";

/// What Bob's device writes in a channel: a line of text, with `to` if any, and the content's
/// `hops` and `reply_to`, the envelope's `msg_id` and `ttl`.
pub struct Writes<'a> {
    pub line: Line<'a>,
    pub hops: u64,
    pub reply_to: Option<MessageId>,
    /// `None`, a new one.
    pub msg_id: Option<MessageId>,
    pub ttl: Option<u64>,
}

impl<'a> Writes<'a> {
    /// `text` from `from`, to `to` if any, with `counter`, `hops` 0 and no `reply_to`.
    pub fn new(from: &str, to: Option<&str>, counter: u64, text: &'a str) -> Self {
        let mut line = Line::new(from, counter, text);
        line.to = to.map(str::to_owned);
        Writes {
            line,
            hops: 0,
            reply_to: None,
            msg_id: None,
            ttl: None,
        }
    }

    /// The same with `hops`.
    pub fn hops(mut self, hops: u64) -> Self {
        self.hops = hops;
        self
    }

    /// The same, a reply to `msg_id`.
    pub fn replying_to(mut self, msg_id: MessageId) -> Self {
        self.reply_to = Some(msg_id);
        self
    }
}

/// Bob's device writes `writes` in the channel `at`, encrypted in its group; fails unless the
/// server stores it. Gives its `msg_id`.
#[track_caller]
pub fn bob_writes(talk: &mut Talk, at: (SpaceId, ChannelId), writes: &Writes) -> MessageId {
    let mut header = mls_envelope(at, &[], &[]).envelope().header.clone();
    header.msg_id = writes.msg_id.unwrap_or(header.msg_id);
    header.ttl = writes.ttl;
    let mut content = writes.line.content(header.msg_id);
    content.hops = writes.hops;
    content.reply_to = writes.reply_to;
    let payload = (talk.played[0].mls)
        .encrypt(at.1, &content.to_cbor())
        .unwrap_or_else(|err| panic!("encrypt in {}: {err:?}", at.1));
    let msg_id = header.msg_id;
    let bytes = Envelope { header, payload }.to_cbor();
    let envelope = RawEnvelope::from_bytes(bytes).expect("an envelope");
    stored(talk.cloud(), &talk.played[0], &envelope);
    msg_id
}

/// The address of `person`'s agent `agent` in the space `space`; `person` is Ada's handle or Bob's.
pub fn agent_of(person: &str, space: &str, agent: &str) -> String {
    format!("{space}/{person}/{agent}@{HOST}")
}

/// Bob's handle and Ada's.
pub fn handles(talk: &Talk) -> (String, String) {
    (
        talk.played[0].person.handle.to_string(),
        talk.ada.handle.to_string(),
    )
}

/// The text parts of `content`'s A2A message, joined; `None` for another body, such as a receipt.
pub fn text_of(content: &Content) -> Option<String> {
    let Some(Body::A2a(json)) = &content.body else {
        return None;
    };
    let message: Value = serde_json::from_str(json).ok()?;
    let parts = message.get("parts").and_then(Value::as_array)?;
    Some(
        (parts.iter())
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
    )
}

/// Bob's device takes in its mailbox; gives each content it read so far in `channel`, with its
/// `msg_id`.
pub fn bob_reads(talk: &mut Talk, channel: ChannelId) -> Vec<(MessageId, Content)> {
    talk.played[0].take_in(&talk.world.cloud);
    let mut read = Vec::new();
    for (msg_id, decrypted) in talk.played[0].read_in(channel) {
        match Content::from_cbor(&decrypted.plaintext) {
            Ok(content) => read.push((msg_id, content)),
            Err(err) => panic!("no content in {msg_id}: {err:?}"),
        }
    }
    read
}

/// The contents of `read` whose text is `text`.
pub fn of_text<'r>(read: &'r [(MessageId, Content)], text: &str) -> Vec<&'r (MessageId, Content)> {
    (read.iter())
        .filter(|(_, content)| text_of(content).as_deref() == Some(text))
        .collect()
}

/// The one content of `read` whose text is `text`; fails unless there is one.
#[track_caller]
pub fn the_one(read: &[(MessageId, Content)], text: &str) -> (MessageId, Content) {
    match of_text(read, text).as_slice() {
        [one] => (*one).clone(),
        other => panic!("{text:?} not read once: {other:?}"),
    }
}

/// Bob's device takes in its mailbox; fails if it read `texts` in one of `channels`.
#[track_caller]
pub fn assert_bob_never_reads(talk: &mut Talk, channels: &[ChannelId], texts: &[&str]) {
    for channel in channels {
        let read = bob_reads(talk, *channel);
        for text in texts {
            let found = of_text(&read, text);
            assert!(found.is_empty(), "{text:?} sent in {channel}: {found:?}");
        }
    }
}

/// Fails unless `content` is a reply to `original` from `from` to `to` with `hops`.
#[track_caller]
pub fn assert_reply(content: &Content, (from, to): (&str, &str), original: MessageId, hops: u64) {
    assert_eq!(content.from.to_string(), from, "from: {content:?}");
    assert_eq!(
        content.to.as_ref().map(ToString::to_string).as_deref(),
        Some(to),
        "to: {content:?}"
    );
    assert_eq!(content.reply_to, Some(original), "reply_to: {content:?}");
    assert_eq!(content.hops, hops, "hops: {content:?}");
}

/// Runs `skald [--agent <agent>] <command> <args>` on `machine` in `dir`.
pub fn run_in(
    machine: &Machine,
    (dir, agent): (&Path, Option<&str>),
    command: &str,
    args: &[&str],
) -> Finished {
    let mut all: Vec<&str> = Vec::new();
    if let Some(agent) = agent {
        all.extend([AGENT, agent]);
    }
    all.push(command);
    all.extend(args);
    machine.start_in(dir, &all).finish()
}

/// Runs `skald [--agent <agent>] reply <message> <text>` on Ada's machine in `~/work/acme`.
pub fn replies(agents: &Agents, agent: Option<&str>, message: &str, text: &str) -> Finished {
    let machine = &agents.talk.ada.machine;
    run_in(machine, (&agents.folder, agent), REPLY, &[message, text])
}

/// Runs `skald [--agent <agent>] send <addressee> --channel <the talk's channel> <text>` on Ada's
/// machine in `~/work/acme`, `addressee` `--to <address>` or `--all`; fails unless it succeeds.
#[track_caller]
pub fn ada_sends(agents: &Agents, agent: Option<&str>, addressee: &[&str], text: &str) {
    let channel = agents.talk.channel.to_string();
    let mut args = addressee.to_vec();
    args.extend([CHANNEL, &channel, text]);
    let machine = &agents.talk.ada.machine;
    let sent = run_in(machine, (&agents.folder, agent), SEND, &args);
    assert!(sent.success, "{text:?} not sent: {sent:?}");
}

/// Fails unless `finished` failed, said why and named none of `names`, in any case.
#[track_caller]
pub fn assert_refused(finished: &Finished, names: &[&str], what: &str) {
    assert!(!finished.success, "{what}: not refused: {finished:?}");
    let shown = finished.shown().to_lowercase();
    assert!(!shown.trim().is_empty(), "{what}: refused without a reason");
    for name in names {
        assert!(
            !shown.contains(&name.to_lowercase()),
            "{what}: the refusal names {name:?}: {shown}"
        );
    }
}

/// Fails unless `shown` holds `text` on one line, which refers to the reply `reply` and to the
/// message `original` it answers.
#[track_caller]
pub fn assert_reply_line(shown: &str, text: &str, (reply, original): (MessageId, MessageId)) {
    let mut holding = shown.lines().filter(|line| line.contains(text));
    let (Some(line), None) = (holding.next(), holding.next()) else {
        panic!("{text:?} is not on one line: {shown}");
    };
    for (what, msg_id) in [("itself", reply), ("the message it answers", original)] {
        assert!(
            !lines_referring_to(line, msg_id).is_empty(),
            "{line:?} names not {what} by #{}...",
            reference(msg_id)
        );
    }
}

/// The lines of `shown`, the person's output, that say `replied` of `msg_id`: they hold its
/// `msg_id` and the word, and none of `replies`, the texts of replies.
pub fn replied_lines(shown: &str, msg_id: MessageId, replies: &[&str]) -> Vec<String> {
    let id = msg_id.to_string();
    (shown.lines())
        .filter(|line| line.contains(&id) && line.to_lowercase().contains(REPLIED))
        .filter(|line| replies.iter().all(|text| !line.contains(text)))
        .map(str::to_owned)
        .collect()
}
