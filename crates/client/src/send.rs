//! `skald send`: the device sends a text to a channel of a space it follows; as an agent, only in
//! the space of its folder. It first accepts the space's latest revisions and goes through the
//! mailbox as `skald recv` does, showing none of it: it sends its Commits and Welcomes that wait,
//! writes each entry, goes through the Welcomes, Commits and application messages, brings the
//! channel's group in line ([`crate::groups`]) and uploads the links of the key chains the server
//! lacks ([`crate::key_chain`]); a later `skald recv` shows the messages. It then
//! encrypts the content in the channel's group and sends one `mls` envelope without `recipients`,
//! with `report` true, unless a member device of the group is not admitted, a Commit of its own
//! there waits for the server or no other device is in the group (`docs/protocol/membership.md`,
//! "Admitted devices"). The content is from the person address or the agent's, with `hops` 0, the
//! device's next `counter` in the channel and an A2A message with the text
//! (`docs/protocol/README.md`, "Content").
//!
//! The envelope goes into the outbox with the MLS state first. The device then sends every
//! envelope of the outbox, those an earlier run left first, over its connection as its device, and
//! sends the unanswered ones again with backoff and the same bytes until each gets `stored` or
//! `send-refused`, for as long as the request timeout (`docs/protocol/delivery.md`, "Send"). Over
//! HTTPS it sends them one at a time, each after the previous one is answered, so a channel's
//! envelopes are stored in `counter` order ("Gap detection"). What is left waits in the outbox for
//! the next run. It then uploads the history entries of the messages the server stored
//! ([`crate::history`]).

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_proto::{
    Address, Body, Cbor as _, Cert, ChannelId, Content, DeviceId, DeviceKind, Frame, MessageId,
    Name, RawEnvelope, Roster, SendKey, SpaceId, Unknown, VersionRange,
};
use tungstenite::Message;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;

use crate::agents::{NO_SERVER_NAME, server_name};
use crate::connection::{self, WebSocket, printable};
use crate::devices::text;
use crate::follow::{Own, SpaceDevices};
use crate::forms::{self, References};
use crate::groups::{self, Link, Rosters};
use crate::history;
use crate::key_chain;
use crate::mailbox::{self, Mailbox, Transport};
use crate::recv;
use crate::secrets;
use crate::spaces::{self, Follower, Scope, Synced};
use crate::state::{self, Outgoing, State};
use crate::wraps;

/// The first pause between attempts, and the longest.
const FIRST_PAUSE: Duration = Duration::from_millis(500);
const LONGEST_PAUSE: Duration = Duration::from_secs(8);

/// What the server answered to an envelope.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Stored,
    Refused(String),
}

/// What the server answers an envelope by: its `msg_id` and, under `hpke`, its recipient device.
pub type Key = (MessageId, Option<DeviceId>);

/// An envelope as it goes out.
pub trait Wire {
    fn key(&self) -> Key;
    /// Its `send` frame.
    fn frame(&self) -> &[u8];
    /// Takes it out of the outbox of `state`, once the server answered it with `answer`.
    fn answered(&self, state: &State, answer: &Answer);
}

/// An envelope of the outbox, as it goes out.
pub struct Queued {
    outgoing: Outgoing,
    channel: ChannelId,
    /// Its `send` frame.
    frame: Vec<u8>,
}

impl Wire for Queued {
    fn key(&self) -> Key {
        (self.outgoing.msg_id, self.outgoing.recipient)
    }

    fn frame(&self) -> &[u8] {
        &self.frame
    }

    fn answered(&self, state: &State, answer: &Answer) {
        let (msg_id, recipient) = (self.outgoing.msg_id, self.outgoing.recipient);
        let refused = match answer {
            Answer::Stored => None,
            Answer::Refused(problem) => Some(forms::Status {
                msg_id: msg_id.to_string(),
                space: Some(self.outgoing.space.to_string()),
                channel: Some(self.channel.to_string()),
                status: forms::REFUSED.to_owned(),
                device: recipient.map(|device| device.to_string()),
                device_name: None,
                problem: Some(explained(problem)),
                timestamp: state::unix_now(),
            }),
        };
        let refused =
            refused.map(|status| serde_json::to_string(&status).expect("a status encodes"));
        if let Err(err) = state.remove_outgoing(msg_id, recipient, refused.as_deref()) {
            eprintln!(
                "skald: warning: the outbox keeps the message {msg_id}{}, which the server \
                 answered, so a later run sends it again: {err}",
                to_device(recipient)
            );
        }
    }
}

/// What sending the outbox gave: the server's answers, why the last attempt failed, if it did, and
/// the connection, if it is still open.
pub struct Delivered {
    answers: BTreeMap<Key, Answer>,
    failed: Option<String>,
    pub mailbox: Option<Mailbox>,
}

impl Delivered {
    /// The server's answer to the envelope with `key`, if it answered.
    pub fn answer(&self, key: &Key) -> Option<&Answer> {
        self.answers.get(key)
    }

    /// Why the last attempt failed, if it did.
    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }
}

/// Sends `text` to the channel whose ID is `channel`, by the state in `dir`, its secrets in
/// `storage`, as `agent` if given, whose run holds its warnings for the person
/// ([`recv::holding_warnings`]); retries for `timeout`. Reports as JSON if `json`.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    (channel, text): (&str, &str),
    agent: Option<&Name>,
    json: bool,
) -> Result<(), String> {
    recv::holding_warnings(dir, agent.is_some(), || {
        let reported = match (json, agent) {
            (true, agent) => Reported::Json(agent.map(Name::as_str)),
            (false, Some(_)) => Reported::Sent,
            (false, None) => Reported::Full,
        };
        sent(
            dir,
            storage,
            (speaks, timeout),
            (channel, text),
            agent,
            reported,
        )
    })
}

/// [`run`], reporting as `reported` says.
fn sent(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    (channel, text): (&str, &str),
    agent: Option<&Name>,
    reported: Reported,
) -> Result<(), String> {
    let channel = channel_id(channel)?;
    let scope = Scope::of_run(dir, agent.is_some())?;
    let Synced {
        mut follower,
        space,
        roster,
        devices,
    } = spaces::of_channel(dir, storage, speaks, timeout, channel, scope)
        .map_err(|err| with_waiting(dir, scope, err))?;
    if let Some(agent) = agent {
        // Messages that come from now on are also for this agent.
        (follower.state)
            .add_agent(space.id, agent.as_str())
            .map_err(|err| err.to_string())?;
    }
    let (roster, devices) = through_mailbox(&mut follower, scope, space.id, (roster, devices))?;
    let from = sender(&mut follower, &roster, agent)?;
    groups::encrypt(
        &follower,
        (&roster, &devices),
        (channel, agent.map(Name::as_str)),
        |msg_id, counter| content(&from, msg_id, counter, text),
    )
    .map_err(|err| with_waiting(dir, scope, err))?;
    let queued = queued(&follower.state, scope)?;
    let mut delivered = deliver(&mut follower, &queued, timeout, None);
    match Link::new(&follower) {
        Ok(mut link) => {
            link.mailbox = delivered.mailbox.take();
            history::upload(&mut follower, &mut link, scope);
        }
        Err(err) => eprintln!("skald: warning: {err}"),
    }
    let reported = report(&follower.state, &queued, &delivered, timeout, reported);
    if let Some(agent) = agent {
        // It showed them in full, or as JSON.
        for (msg_id, _) in queued.iter().map(Wire::key) {
            if let Err(err) = follower.state.refusals_taken(agent.as_str(), msg_id) {
                eprintln!("skald: warning: {err}");
            }
        }
    }
    reported
}

/// The channel ID `given` names, as a command takes it.
pub fn channel_id(given: &str) -> Result<ChannelId, String> {
    given
        .parse()
        .map_err(|_| format!("\"{}\" is not a channel ID", printable(given)))
}

/// Goes through the mailbox as `skald recv` does, within `scope`, showing none of it: sends the
/// device's Commits and Welcomes that wait, writes each entry after the cursor, goes through the
/// Welcomes and Commits, brings in line the groups of the channels of each space the run accepted
/// the revisions of and uploads the links of their key chains and the wraps of their current
/// epochs the server lacks. Gives the last accepted revision of `space` and its accounts' devices,
/// `synced` as the run synced them first, as they are then. Warns of what fails.
fn through_mailbox(
    follower: &mut Follower,
    scope: Scope,
    space: SpaceId,
    synced: (Roster, SpaceDevices),
) -> Result<(Roster, SpaceDevices), String> {
    let mut link = Link::new(follower)?;
    if let Err(err) = groups::flush(follower, &mut link, scope) {
        eprintln!("skald: warning: {err}");
    }
    let timeout = follower.timeout;
    // The connection is kept for the run's next one.
    if let Err(err) = recv::catch_up(follower, None, timeout) {
        eprintln!("skald: warning: {err}");
    }
    let mut rosters = Rosters::from([(space, synced)]);
    if let Err(err) = groups::take_in(follower, &mut link, scope, &mut rosters) {
        eprintln!("skald: warning: {err}");
    }
    groups::align_with(follower, &mut link);
    key_chain::upload_missing(follower, &mut link, scope);
    wraps::upload_missing(follower, &mut link, scope);
    Ok(rosters.remove(&space).expect("the channel's space"))
}

/// `err`, the reason a run sends nothing, saying how many envelopes earlier runs left in the
/// outbox of the state in `dir` within `scope` still wait, if any: only a run that sends a message
/// or receives sends them.
fn with_waiting(dir: &Path, scope: Scope, err: String) -> String {
    let outbox = dir
        .is_dir()
        .then(|| State::open(dir).and_then(|state| state.outbox()).ok())
        .flatten()
        .unwrap_or_default();
    let waiting = outbox
        .iter()
        .filter(|outgoing| !scope.excludes(outgoing.space))
        .count();
    if waiting == 0 {
        return err;
    }
    format!(
        "{err}; {waiting} envelopes of earlier messages still wait in the outbox: the next skald \
         recv, or skald send of a message, sends them first"
    )
}

/// The admitted `client` devices of `channel` in `roster`, the space's latest revision, but `this`
/// one, each with its valid certificate among `devices`: those of the accounts of the channel's
/// members that their member's `excluded` does not list. The certificates of `own`'s account are
/// checked against the master key it pinned, any other's against the revision's.
pub fn admitted<'d>(
    own: &Own,
    this: DeviceId,
    roster: &Roster,
    devices: &'d SpaceDevices,
    channel: ChannelId,
) -> BTreeMap<DeviceId, &'d Cert> {
    let body = roster.body();
    let mut admitted = BTreeMap::new();
    let Some(channel) = body.channels.iter().find(|it| it.id == channel) else {
        return admitted;
    };
    for entry in &channel.members {
        let Some(member) = body.member(entry.account) else {
            continue;
        };
        for cert in devices.of(roster, own, member) {
            let body = cert.body();
            if body.kind == DeviceKind::Client && body.device != this {
                admitted.insert(body.device, cert);
            }
        }
    }
    admitted
}

/// The address the message is from: the person's, or `agent`'s, of the device's account in
/// `roster`'s space, on the server of the machine's session.
fn sender(
    follower: &mut Follower,
    roster: &Roster,
    agent: Option<&Name>,
) -> Result<Address, String> {
    let body = roster.body();
    let member = body.member(follower.own.account).ok_or_else(|| {
        format!(
            "this account is no member of the space {} in its last accepted revision; nothing is \
             sent",
            body.name.as_str()
        )
    })?;
    let server = server_name(&mut follower.connection)?
        .ok_or_else(|| format!("{NO_SERVER_NAME}, so a message has no sender; nothing is sent"))?;
    let (space, owner) = (body.name.clone(), member.handle.clone());
    Ok(match agent {
        Some(agent) => Address::new(space, owner, agent.clone(), server),
        None => Address::person(space, owner, server),
    })
}

/// The content of a message of `text` from `from`, of `msg_id`, with `counter`.
fn content(
    from: &Address,
    msg_id: MessageId,
    counter: std::num::NonZeroU64,
    text: &str,
) -> Content {
    let a2a = serde_json::json!({
        "messageId": msg_id.to_string(),
        "role": "ROLE_USER",
        "parts": [{ "text": text }],
    })
    .to_string();
    Content {
        from: from.clone(),
        to: None,
        counter: Some(counter),
        hops: 0,
        reply_to: None,
        body: Some(Body::A2a(a2a)),
        delegation: None,
        grant: None,
        attachments: None,
        unknown: Unknown::default(),
    }
}

/// " to device <recipient>", or nothing for none.
fn to_device(recipient: Option<DeviceId>) -> String {
    recipient.map_or_else(String::new, |device| format!(" to device {device}"))
}

/// The envelopes of the outbox of `state` within `scope`, in the order they go out.
pub fn queued(state: &State, scope: Scope) -> Result<Vec<Queued>, String> {
    let outbox = state.outbox().map_err(text)?;
    outbox
        .into_iter()
        .filter(|outgoing| !scope.excludes(outgoing.space))
        .map(|outgoing| {
            let envelope = RawEnvelope::from_bytes(outgoing.envelope.clone()).map_err(|err| {
                format!("the local state is malformed: an envelope in the outbox: {err}")
            })?;
            let channel = envelope.envelope().header.channel;
            let frame = Frame::Send {
                envelope,
                report: true,
            }
            .to_cbor();
            Ok(Queued {
                outgoing,
                channel,
                frame,
            })
        })
        .collect()
}

/// Sends each envelope of `queued` that the server has not answered, in order, over `mailbox` or a
/// connection as the device, again with backoff while some are unanswered, for `timeout`, unless
/// the server closes the connection for a policy violation or refuses a request's proof, which
/// another attempt does not change; takes each answered one out of the outbox. A failed attempt
/// reconnects in the transports' order ([`crate::mailbox`]).
pub fn deliver<W: Wire>(
    follower: &mut Follower,
    queued: &[W],
    timeout: Duration,
    mut mailbox: Option<Mailbox>,
) -> Delivered {
    let deadline = Instant::now() + timeout;
    let mut delivered = Delivered {
        answers: BTreeMap::new(),
        failed: None,
        mailbox: None,
    };
    let keys = match follower.keys() {
        Ok(keys) => keys,
        Err(err) => {
            delivered.failed = Some(err);
            return delivered;
        }
    };
    let mut pause = FIRST_PAUSE;
    loop {
        let unanswered: Vec<&W> = queued
            .iter()
            .filter(|queued| !delivered.answers.contains_key(&queued.key()))
            .collect();
        let now = Instant::now();
        if unanswered.is_empty() || now >= deadline {
            delivered.mailbox = mailbox;
            return delivered;
        }
        let until = (now + pause).min(deadline);
        pause = (pause * 2).min(LONGEST_PAUSE);
        let opened = match mailbox.take() {
            Some(mailbox) => Ok(mailbox),
            None => follower.connect(&keys),
        };
        let attempt = opened.map_err(|err| (err, false)).and_then(|mut opened| {
            let answers = &mut delivered.answers;
            let sent = match opened.websocket() {
                Some(socket) => (unanswered.iter())
                    .try_for_each(|queued| {
                        (socket.send(Message::Binary(queued.frame().to_vec().into())))
                            .map_err(|err| (broke(err), false))
                    })
                    .and_then(|()| read_answers(socket, follower, queued, answers, until)),
                None => one_by_one(follower, &keys, &unanswered, answers),
            };
            match sent {
                Ok(()) => Ok(opened),
                // The next attempt reconnects.
                Err(err) => {
                    opened.discard();
                    Err(err)
                }
            }
        });
        match attempt {
            Ok(opened) => {
                mailbox = Some(opened);
                delivered.failed = None;
            }
            Err((err, refused)) => {
                delivered.failed = Some(err);
                if refused {
                    return delivered;
                }
                follower.transport = Transport::WebSocket;
                thread::sleep(until.saturating_duration_since(Instant::now()));
            }
        }
    }
}

/// Sends each of `unanswered` in an HTTPS request as the device of `keys`, one at a time, each after
/// the previous one is answered, and keeps the answers in `answers`, taking each answered envelope
/// out of the outbox; stops at the first request that fails, saying whether the server refused the
/// device's proof.
fn one_by_one<W: Wire>(
    follower: &mut Follower,
    keys: &crate::device::Keys,
    unanswered: &[&W],
    answers: &mut BTreeMap<Key, Answer>,
) -> Result<(), (String, bool)> {
    for queued in unanswered {
        let answer = mailbox::request(
            follower,
            keys,
            mailbox::SEND,
            queued.frame(),
            Duration::ZERO,
        )
        .map_err(|err| {
            let refused = matches!(err, connection::Error::Refused { status: 401, .. });
            (err.to_string(), refused)
        })?;
        let (send_key, answer) = match Frame::from_cbor(&answer) {
            Ok(Frame::Stored(stored)) => (stored.send_key, Answer::Stored),
            Ok(Frame::SendRefused { send_key, problem }) => (send_key, Answer::Refused(problem)),
            _ => {
                let unexpected = "unexpected answer from the server: a send answered without its \
                                  stored or send-refused";
                return Err((unexpected.to_owned(), false));
            }
        };
        let SendKey { msg_id, recipient } = send_key;
        if (msg_id, recipient) == queued.key() {
            queued.answered(&follower.state, &answer);
            answers.insert(queued.key(), answer);
        }
    }
    Ok(())
}

/// Reads the server's answers to `queued` on `socket` into `answers` until each one is answered or
/// `until`, taking each answered envelope out of the outbox; fails if the connection ends, saying
/// whether the server closed it for a policy violation.
fn read_answers<W: Wire>(
    socket: &mut WebSocket,
    follower: &Follower,
    queued: &[W],
    answers: &mut BTreeMap<Key, Answer>,
    until: Instant,
) -> Result<(), (String, bool)> {
    while queued
        .iter()
        .any(|queued| !answers.contains_key(&queued.key()))
    {
        let wait = until.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            return Ok(());
        }
        set_read_timeout(socket, wait).map_err(|err| (broke(err.into()), false))?;
        let (send_key, answer) = match socket.read() {
            Ok(Message::Binary(bytes)) => match Frame::from_cbor(&bytes) {
                Ok(Frame::Stored(stored)) => (stored.send_key, Answer::Stored),
                Ok(Frame::SendRefused { send_key, problem }) => {
                    (send_key, Answer::Refused(problem))
                }
                // A frame it does not know or that does not decode, ignored.
                _ => continue,
            },
            Ok(Message::Close(frame)) => {
                let refused = frame
                    .as_ref()
                    .is_some_and(|frame| frame.code == CloseCode::Policy);
                return Err((closed(frame.as_ref()), refused));
            }
            Ok(_) => continue,
            Err(err) if timed_out(&err) => return Ok(()),
            Err(err) => return Err((broke(err), false)),
        };
        let SendKey { msg_id, recipient } = send_key;
        let key = (msg_id, recipient);
        if answers.contains_key(&key) {
            continue;
        }
        let Some(answered) = queued.iter().find(|queued| queued.key() == key) else {
            continue;
        };
        answered.answered(&follower.state, &answer);
        answers.insert(key, answer);
    }
    Ok(())
}

/// Whether `err`, of a read of a connection, is that its wait ran out.
pub fn timed_out(err: &tungstenite::Error) -> bool {
    matches!(
        err,
        tungstenite::Error::Io(err)
            if matches!(err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
    )
}

/// Sets how long a read of `socket` waits, at least a millisecond.
pub fn set_read_timeout(socket: &mut WebSocket, wait: Duration) -> io::Result<()> {
    let wait = Some(wait.max(Duration::from_millis(1)));
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(wait),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(wait),
        _ => Ok(()),
    }
}

/// Why a connection broke, by `err`.
pub fn broke(err: tungstenite::Error) -> String {
    format!("the connection to the server broke: {err}")
}

/// Why the server closed a connection, by its close `frame`.
pub fn closed(frame: Option<&CloseFrame>) -> String {
    match frame.map(|frame| frame.reason.as_str()) {
        Some(reason) if !reason.is_empty() => {
            format!("the server closed the connection: {}", printable(reason))
        }
        _ => "the server closed the connection".to_owned(),
    }
}

/// The refusal `problem`, printable, with what it means for the message if it is over a limit.
fn explained(problem: &str) -> String {
    let meaning = match problem {
        "too-large" => "the message is too large for the server",
        "quota-exceeded" => "its space has stored as many messages this month as its limit allows",
        _ => return printable(problem),
    };
    format!("{problem} ({meaning})")
}

/// How a run reports the messages it sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reported<'a> {
    /// Each message in full: the person's runs.
    Full,
    /// Each message as JSON; only those of the agent, if given.
    Json(Option<&'a str>),
    /// `sent #<ref>` per message once every one is stored, otherwise each in full: an agent's
    /// `skald send`.
    Sent,
    /// Nothing: an agent's `skald recv`, which shows the refusals of its messages as statuses.
    Quiet,
}

/// A message of the outbox: its ID, its channel and its envelopes.
type Queue<'q> = Vec<(MessageId, ChannelId, Vec<&'q Queued>)>;

/// Says what the server answered to each message of `queued`, as `reported` says, by the state
/// `state` and the answers in `delivered`; fails unless every one is stored, saying which are not.
pub fn report(
    state: &State,
    queued: &[Queued],
    delivered: &Delivered,
    timeout: Duration,
    reported: Reported,
) -> Result<(), String> {
    let mut messages: Queue = Vec::new();
    for queued in queued {
        match messages
            .iter_mut()
            .find(|(id, ..)| *id == queued.outgoing.msg_id)
        {
            Some((.., envelopes)) => envelopes.push(queued),
            None => messages.push((queued.outgoing.msg_id, queued.channel, vec![queued])),
        }
    }
    let outcome = outcome(&messages, delivered, timeout);
    let references = || state.kept_ids().map(References::new).map_err(text);
    let of = |agent: &str, id: MessageId| {
        let sent = state.sent(id).map_err(text)?;
        Ok::<_, String>(sent.and_then(|sent| sent.agent).as_deref() == Some(agent))
    };
    match reported {
        Reported::Sent if outcome.is_ok() => {
            let references = references()?;
            for (id, ..) in &messages {
                println!("sent #{}", references.of(*id));
            }
        }
        Reported::Full | Reported::Sent => in_full(&messages, delivered),
        Reported::Json(agent) => {
            let references = references()?;
            for (id, channel, envelopes) in &messages {
                if let Some(agent) = agent
                    && !of(agent, *id)?
                {
                    continue;
                }
                let envelopes: Vec<Value> = (envelopes.iter())
                    .map(|queued| {
                        let recipient = queued.outgoing.recipient.map(|device| device.to_string());
                        match delivered.answers.get(&queued.key()) {
                            Some(Answer::Stored) => {
                                json!({ "recipient": recipient, "answer": "stored" })
                            }
                            Some(Answer::Refused(problem)) => json!({
                                "recipient": recipient,
                                "answer": "refused",
                                "problem": problem,
                            }),
                            None => json!({ "recipient": recipient, "answer": "waiting" }),
                        }
                    })
                    .collect();
                let fields = json!({
                    "msg_id": id.to_string(),
                    "ref": references.of(*id),
                    "channel": channel.to_string(),
                    "envelopes": envelopes,
                });
                println!("{}", forms::object("sent", fields));
            }
        }
        Reported::Quiet => {}
    }
    outcome
}

/// Says, for each of `messages`, its ID and, per envelope, by its recipient device under `hpke`,
/// whether the server stored it, refused it or has not answered it in `delivered`.
fn in_full(messages: &Queue, delivered: &Delivered) {
    for (id, channel, envelopes) in messages {
        println!("The message {id} to the channel {channel}:");
        for queued in envelopes {
            let device = (queued.outgoing.recipient)
                .map_or_else(|| "channel".to_owned(), |device| device.to_string());
            match delivered.answers.get(&queued.key()) {
                Some(Answer::Stored) => println!("  {device}  stored"),
                Some(Answer::Refused(problem)) => {
                    println!("  {device}  refused: {}", explained(problem));
                }
                None => println!("  {device}  no answer yet: it waits in the outbox"),
            }
        }
    }
}

/// Fails unless the server stored every envelope of `messages` in `delivered`, saying which it has
/// not answered within `timeout`, and why the last attempt failed, or else which it refused.
fn outcome(messages: &Queue, delivered: &Delivered, timeout: Duration) -> Result<(), String> {
    let (mut left, mut refused) = (Vec::new(), Vec::new());
    for (id, _, envelopes) in messages {
        let mut waiting = Vec::new();
        for queued in envelopes {
            let recipient = queued.outgoing.recipient;
            match delivered.answers.get(&queued.key()) {
                Some(Answer::Stored) => {}
                Some(Answer::Refused(problem)) => refused.push(format!(
                    "the message {id}{}: {}",
                    to_device(recipient),
                    explained(problem)
                )),
                None => waiting.extend(recipient.map(|device| device.to_string())),
            }
        }
        if !waiting.is_empty() {
            left.push(format!("the message {id} to {}", waiting.join(", ")));
        } else if envelopes
            .iter()
            .any(|queued| !delivered.answers.contains_key(&queued.key()))
        {
            left.push(format!("the message {id}"));
        }
    }
    if !left.is_empty() {
        let why = delivered
            .failed
            .as_ref()
            .map_or_else(String::new, |err| format!(" ({err})"));
        return Err(format!(
            "the server has not stored {} within {} s{why}; the outbox keeps it, and the next \
             skald send or skald recv sends it first",
            left.join("; "),
            timeout.as_secs()
        ));
    }
    if !refused.is_empty() {
        return Err(format!(
            "the server refused {}; it is not sent again",
            refused.join("; ")
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::num::NonZeroU64;

    use skald_crypto::{HpkeKey, SigningKey, issue};
    use skald_proto::{
        AccountId, CertBody, ChannelMember, Envelope, Header, NonEmpty, Scheme, SpaceId,
    };

    use super::*;
    use crate::spaces::tests::{first, member};
    use crate::state::Counted;
    use crate::test_dir::TestDir;

    /// Puts `outgoing`, the first message in `channel`, in the outbox of `state`.
    fn put(state: &State, channel: ChannelId, outgoing: Outgoing) {
        let message = Counted {
            outgoing,
            channel,
            counter: 1,
            agent: None,
        };
        crate::state::tests::put(state, &[&message]).unwrap();
    }

    /// An account and its master key.
    pub(crate) struct Account {
        pub(crate) id: AccountId,
        pub(crate) master: SigningKey,
    }

    impl Account {
        pub(crate) fn new() -> Self {
            Account {
                id: AccountId::generate(),
                master: SigningKey::generate(),
            }
        }

        /// A device of the account of `kind`, its certificate issued with `issuer`, the key of
        /// the device `by` or else the master key: the certificate, its HPKE key and its signing
        /// key.
        pub(crate) fn device(
            &self,
            kind: DeviceKind,
            issuer: &SigningKey,
            by: Option<DeviceId>,
        ) -> (Cert, [u8; 32], SigningKey) {
            let signing = SigningKey::generate();
            let hpke = HpkeKey::generate().public_key();
            let body = CertBody {
                account: self.id,
                device: DeviceId::from_signing_key(&signing.public_key()),
                kind,
                signing: signing.public_key(),
                hpke,
                issuer: by,
                issued: 1_800_000_000,
                ssh: None,
                name: "laptop".to_owned(),
                unknown: Unknown::default(),
            };
            let cert = issue(&body, issuer, &signing).expect("a certificate");
            (cert, hpke, signing)
        }

        pub(crate) fn client(&self) -> (Cert, [u8; 32], SigningKey) {
            self.device(DeviceKind::Client, &self.master, None)
        }
    }

    /// The channel `id` whose members are `accounts`.
    pub(crate) fn channel_of(id: ChannelId, accounts: &[AccountId]) -> skald_proto::Channel {
        skald_proto::Channel {
            id,
            members: accounts
                .iter()
                .map(|&account| ChannelMember {
                    account,
                    history: false,
                })
                .collect(),
            services: Vec::new(),
            unknown: Unknown::default(),
        }
    }

    /// Admitted: the `client` devices with a valid certificate of the channel's members' accounts,
    /// this one, a browser, one the member's `excluded` lists, one with a certificate by another
    /// key and one of a member outside the channel aside.
    #[test]
    fn admitted_devices_are_the_channels_valid_client_devices_but_this_one() {
        let (ada, bob, carol) = (Account::new(), Account::new(), Account::new());
        let id = |cert: &Cert| cert.body().device;
        let (this, _, this_key) = ada.client();
        let (laptop, laptop_hpke, _) = ada.client();
        let (browser, ..) = ada.device(DeviceKind::Browser, &this_key, Some(id(&this)));
        let (phone, phone_hpke, _) = bob.client();
        let (barred, ..) = bob.client();
        let (forged, ..) = bob.device(DeviceKind::Client, &SigningKey::generate(), None);
        let (outside, ..) = carol.client();
        let certs = vec![this, laptop, browser, phone, barred, forged, outside];

        let mut body = first(ada.id).body().clone();
        let mut bobs = member(bob.id, "bob", bob.master.public_key());
        bobs.excluded = Some(NonEmpty::new(id(&certs[4])));
        body.members.push(bobs);
        body.members
            .push(member(carol.id, "carol", carol.master.public_key()));
        let channel = ChannelId::generate();
        body.channels = vec![channel_of(channel, &[ada.id, bob.id])];
        let roster = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
        let own = Own {
            account: ada.id,
            master: ada.master.public_key(),
        };
        let devices = SpaceDevices {
            ids: certs.iter().map(id).collect(),
            certs: certs.clone(),
            revocations: Vec::new(),
        };

        let given: BTreeMap<DeviceId, [u8; 32]> =
            admitted(&own, id(&certs[0]), &roster, &devices, channel)
                .into_iter()
                .map(|(device, cert)| (device, cert.body().hpke))
                .collect();

        assert_eq!(
            given,
            BTreeMap::from([(id(&certs[1]), laptop_hpke), (id(&certs[3]), phone_hpke)])
        );
        let elsewhere = admitted(
            &own,
            id(&certs[0]),
            &roster,
            &devices,
            ChannelId::generate(),
        );
        assert!(elsewhere.is_empty(), "{elsewhere:?}");
    }

    /// A run that sends nothing says how many envelopes of its scope wait in the outbox, if any.
    #[test]
    fn run_that_sends_nothing_says_what_waits_in_the_outbox() {
        let dir = TestDir::new("send-waiting");
        let (own, other) = (SpaceId::generate(), SpaceId::generate());
        let err = || "nothing is sent".to_owned();
        assert_eq!(with_waiting(&dir, Scope::All, err()), err());
        let state = State::open(&dir).unwrap();
        for (number, space) in [own, other, own].into_iter().enumerate() {
            let outgoing = Outgoing {
                msg_id: MessageId::generate(),
                recipient: None,
                space,
                envelope: vec![u8::try_from(number).unwrap()],
            };
            put(&state, ChannelId::generate(), outgoing);
        }
        let waits = |count| {
            format!(
                "nothing is sent; {count} envelopes of earlier messages still wait in the outbox: \
                 the next skald recv, or skald send of a message, sends them first"
            )
        };

        assert_eq!(with_waiting(&dir, Scope::All, err()), waits(3));
        assert_eq!(with_waiting(&dir, Scope::Only(own), err()), waits(2));
        assert_eq!(
            with_waiting(&dir, Scope::Only(SpaceId::generate()), err()),
            err()
        );
    }

    /// An agent's run sends only the envelopes of its space, in the order they went into the
    /// outbox; a person's sends every one.
    #[test]
    fn queued_envelopes_are_those_of_the_scope_in_order() {
        let dir = TestDir::new("send-queued");
        let state = State::open(&dir).unwrap();
        let (own, other) = (SpaceId::generate(), SpaceId::generate());
        let mut messages = Vec::new();
        for space in [own, other, own] {
            let channel = ChannelId::generate();
            let msg_id = MessageId::generate();
            let header = Header {
                v: NonZeroU64::MIN,
                space,
                channel,
                msg_id,
                scheme: Scheme::Mls,
                recipients: None,
                ttl: None,
                unknown: Unknown::default(),
            };
            let outgoing = Outgoing {
                msg_id,
                recipient: None,
                space,
                envelope: Envelope {
                    header,
                    payload: vec![1, 2, 3],
                }
                .to_cbor(),
            };
            put(&state, channel, outgoing);
            messages.push((msg_id, channel));
        }
        let ids = |scope| -> Vec<(MessageId, ChannelId)> {
            queued(&state, scope)
                .unwrap()
                .iter()
                .map(|queued| (queued.outgoing.msg_id, queued.channel))
                .collect()
        };

        assert_eq!(ids(Scope::All), messages);
        assert_eq!(ids(Scope::Only(own)), [messages[0], messages[2]]);
    }

    /// A read whose wait ran out times out, whichever way the system says it; a broken or closed
    /// connection does not.
    #[test]
    fn only_a_read_whose_wait_ran_out_times_out() {
        let io = |kind| tungstenite::Error::Io(io::Error::from(kind));
        assert!(timed_out(&io(io::ErrorKind::WouldBlock)));
        assert!(timed_out(&io(io::ErrorKind::TimedOut)));
        assert!(!timed_out(&io(io::ErrorKind::ConnectionReset)));
        assert!(!timed_out(&tungstenite::Error::ConnectionClosed));
    }
}
