//! `skald send`: the device sends a text to a channel of a space it follows, by default the default
//! channel of the working directory's space; as an agent, only in the space of its folder. It
//! first accepts the space's latest revisions and goes through the mailbox as `skald recv` does,
//! showing none of it: it sends its Commits and Welcomes that wait,
//! writes each entry, goes through the Welcomes, Commits and application messages, brings the
//! channel's group in line ([`crate::groups`]) and uploads the links of the key chains the server
//! lacks ([`crate::key_chain`]); a later `skald recv` shows the messages. It then
//! encrypts the content in the channel's group and sends one `mls` envelope without `recipients`,
//! with `report` true, unless a member device of the group is not admitted, a Commit of its own
//! there waits for the server (`docs/protocol/membership.md`, "Admitted devices"), or no other
//! device is in the group, nothing on this device gets the message and no service agent gets a
//! copy. The content is from the person address or the agent's, to the agent `--to` names or, for
//! `--all`, to no one in particular, with `hops` 0 from the person and, from an agent, one more than
//! the highest `hops` of the messages handed over to it in the space since the device last sent a
//! message as it, or 0 when none was, the device's next `counter` in the channel and an A2A message
//! with the text (`docs/protocol/README.md`, "Content"); a member service agent counts as an agent
//! here. An agent's message with `hops` over [`MOST_HOPS`] is not sent: the run says why and warns
//! of it, which an agent's run holds for the person's next run. With it go its copies under `hpke`
//! to service agents outside the channel's group ([`crate::copies`]), each with the counter the
//! device keeps for that agent in the channel; the run warns when it sends an agent its first copy
//! of the channel's messages. A `service` device sends none: its token gets no grants. A stateless
//! service agent's run sends as [`crate::stateless`] says.
//!
//! The envelope goes into the outbox with the MLS state first. The device then sends every
//! envelope of the outbox, those an earlier run left first, over its connection as its device, and
//! sends the unanswered ones again with backoff and the same bytes until each gets `stored` or
//! `send-refused`, for as long as the request timeout (`docs/protocol/delivery.md`, "Send"). Over
//! HTTPS it sends them one at a time, each after the previous one is answered, so a channel's
//! envelopes are stored in `counter` order ("Gap detection"). With the `stored` of a message, the
//! device keeps it for its person and its agents, as a recipient device does
//! ([`recv::own_message`]). What is left waits in the outbox for the next run. It then uploads the
//! history entries of the messages the server stored ([`crate::history`]).
//!
//! `skald reply` sends as it does ([`crate::reply`]).

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_proto::{
    Address, AddressError, Body, Cbor as _, Cert, ChannelId, Content, DeviceId, DeviceKind, Frame,
    MessageId, Name, PERSON, Push, RawEnvelope, Roster, Segment, SendKey, ServerName, SpaceId,
    Unknown, VersionRange,
};
use tungstenite::Message;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;

use crate::agents::{NO_SERVER_NAME, server_name};
use crate::connection::{self, WebSocket, printable};
use crate::copies;
use crate::devices::text;
use crate::follow::{Own, SpaceDevices};
use crate::forms::{self, References};
use crate::groups::{self, Link, Rosters};
use crate::history;
use crate::key_chain;
use crate::mailbox::{self, Mailbox, Transport};
use crate::queue_keys;
use crate::recv;
use crate::secrets;
use crate::service;
use crate::spaces::{self, Follower, Scope, Synced};
use crate::state::{self, Outgoing, State};
use crate::stateless;
use crate::wraps;

/// The highest `hops` an agent's message carries: the limit on replies without a human
/// (`docs/protocol/README.md`, "Content").
pub const MOST_HOPS: u64 = 10;

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
                reason: None,
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

/// An envelope of a message whose run says what the server answered to it ([`report`]).
pub trait Shown: Wire {
    /// The channel it goes to.
    fn channel(&self) -> ChannelId;
    /// Whether it goes with `report` true: it is no receipt.
    fn reports(&self) -> bool;
    /// Whether an outbox keeps it while the server has not answered it, so a later run sends it.
    fn kept(&self) -> bool;
}

impl Shown for Queued {
    fn channel(&self) -> ChannelId {
        self.channel
    }

    fn reports(&self) -> bool {
        self.outgoing.report
    }

    fn kept(&self) -> bool {
        true
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

/// What a `skald send` names: its `--to`, its `--channel` and its text.
pub type Named<'a> = (Option<&'a str>, Option<&'a str>, &'a str);

/// Sends `text` for the agent `to` names ([`addressee`]), or for every agent without it, to the
/// channel whose ID is `channel`, by default the default channel of the working directory's space,
/// by the state in `dir`, its secrets in `storage`, as `agent` if given, whose run holds its
/// warnings for the person ([`recv::holding_warnings`]); retries for `timeout`. Reports as JSON if
/// `json`.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    (to, channel, text): Named,
    agent: Option<&Name>,
    json: bool,
) -> Result<(), String> {
    recv::holding_warnings(dir, agent.is_some(), || {
        sent(
            dir,
            storage,
            (speaks, timeout),
            (to, channel, text),
            agent,
            reported(json, agent),
        )
    })
}

/// How a run as `agent`, if any, reports the messages it sent, as JSON if `json`.
pub fn reported(json: bool, agent: Option<&Name>) -> Reported<'_> {
    match (json, agent) {
        (true, agent) => Reported::Json(agent.map(Name::as_str)),
        (false, Some(agent)) => Reported::Sent(agent.as_str()),
        (false, None) => Reported::Full,
    }
}

/// What a run sends besides its sender and channel: the agent or person it is to, none for every
/// agent in the channel, its text and, for a reply, the message it answers and its `hops`.
pub struct Draft<'a> {
    pub to: Option<Address>,
    pub text: &'a str,
    pub reply_to: Option<MessageId>,
    pub hops: u64,
}

/// [`run`], reporting as `reported` says.
fn sent(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    (to, channel, text): Named,
    agent: Option<&Name>,
    reported: Reported,
) -> Result<(), String> {
    let to = to.map(addressee).transpose()?;
    let channel = channel.map(channel_id).transpose()?;
    let scope = Scope::of_run(dir, agent.is_some())?;
    let (mut synced, channel) = match channel {
        Some(channel) => of_channel(dir, storage, (speaks, timeout), channel, scope)
            .map(|synced| (synced, channel))?,
        None => spaces::of_working_dir(dir, storage, speaks, timeout, scope)
            .map_err(|err| with_waiting(dir, scope, err))?,
    };
    let server = server_of(&mut synced.follower)?;
    let to = (to.map(|to| to.in_space(&synced.roster.body().name, &server))).transpose()?;
    let hops = match counted(&synced.follower, agent) {
        Some(counted) => (synced.follower.state)
            .handed_hops(synced.space.id, counted)
            .map_err(|err| err.to_string())?
            .map_or(0, |highest| highest.saturating_add(1)),
        None => 0,
    };
    let draft = Draft {
        to,
        text,
        reply_to: None,
        hops,
    };
    send_synced(
        dir,
        (synced, channel),
        (scope, server),
        &draft,
        agent,
        reported,
    )
}

/// Under which name the device of `follower` counts the `hops` of what a run as `agent`, if any,
/// takes in: the agent's, a member service agent's ([`state::SERVICE_AGENT`]), or none for the
/// person, whose messages keep 0.
pub fn counted<'a>(follower: &Follower, agent: Option<&'a Name>) -> Option<&'a str> {
    match agent {
        Some(agent) => Some(agent.as_str()),
        None => follower.service.map(|_| state::SERVICE_AGENT),
    }
}

/// The space of the channel `channel` within `scope`, synced ([`spaces::of_channel`]), by the
/// state in `dir`, its secrets in `storage`; fails as a run that sends nothing does.
pub fn of_channel(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    channel: ChannelId,
    scope: Scope,
) -> Result<Synced, String> {
    spaces::of_channel(dir, storage, speaks, timeout, channel, scope)
        .map_err(|err| with_waiting(dir, scope, err))
}

/// The name of `follower`'s server, which a message's sender names; fails without one.
pub fn server_of(follower: &mut Follower) -> Result<ServerName, String> {
    server_name(&mut follower.connection)?
        .ok_or_else(|| format!("{NO_SERVER_NAME}, so a message has no sender; nothing is sent"))
}

/// Sends `draft` to `channel` of the space `synced` holds, within `scope`, from the person or
/// `agent` on `server`, by the state in `dir`, and reports as `reported` says. Sends nothing for
/// an agent's draft with `hops` over [`MOST_HOPS`] ([`over_the_limit`]).
pub fn send_synced(
    dir: &Path,
    (synced, channel): (Synced, ChannelId),
    (scope, server): (Scope, ServerName),
    draft: &Draft,
    agent: Option<&Name>,
    reported: Reported,
) -> Result<(), String> {
    if counted(&synced.follower, agent).is_some() && draft.hops > MOST_HOPS {
        return Err(over_the_limit(agent, channel, draft.to.as_ref(), "sent"));
    }
    let Synced {
        mut follower,
        space,
        roster,
        devices,
    } = synced;
    let timeout = follower.timeout;
    if let Some(agent) = agent {
        // Messages that come from now on are also for this agent.
        (follower.state)
            .add_agent(space.id, agent.as_str())
            .map_err(|err| err.to_string())?;
    }
    // It goes through no mailbox: what it would write there, it would ack unshown.
    if follower.stateless() {
        let to = (channel, server);
        return stateless::send(&mut follower, (&roster, &devices), to, draft, reported);
    }
    let (roster, devices) = through_mailbox(&mut follower, scope, space.id, (roster, devices))?;
    // A service agent's message carries its token's grant; only a member's has copies.
    let (from, grant, readers) = match follower.service {
        Some(_) => {
            let (from, grant) =
                service::sender(&mut follower, (&roster, &devices), (channel, server))?;
            (from, Some(grant), Vec::new())
        }
        None => {
            let from = sender(&follower, &roster, server.clone(), agent)?;
            let to = draft.to.as_ref();
            let readers =
                copies::readers(&mut follower, (&roster, &devices), (channel, to), &server);
            (from, None, readers)
        }
    };
    let (_, first) = groups::encrypt(
        &follower,
        (&roster, &devices),
        (channel, agent.map(Name::as_str)),
        &readers,
        |msg_id, counter| {
            let mut content = content(&from, draft, msg_id, counter);
            content.grant = grant;
            content
        },
    )
    .map_err(|err| with_waiting(dir, scope, err))?;
    for reader in first {
        eprintln!("skald: warning: {}", copies::first_copy(reader, channel));
    }
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

/// Why what `agent`, none for a member service agent, sends to `to`, none for everyone, in
/// `channel` as `verb` says, such as `sent` or `enqueued`, goes nowhere: it is past the limit on
/// replies without a human. Warns of it, which an agent's run holds for its person.
pub fn over_the_limit(
    agent: Option<&Name>,
    channel: ChannelId,
    to: Option<&Address>,
    verb: &str,
) -> String {
    let to = to.map_or_else(|| "every agent".to_owned(), Address::to_string);
    let agent = agent.map_or_else(
        || "this service agent".to_owned(),
        |it| format!("the agent {it}"),
    );
    eprintln!(
        "skald: warning: {agent} {verb} nothing to {to} in the channel {channel}: the \
         agents answered each other {MOST_HOPS} times without a person; a person's message to one \
         of them starts the count again"
    );
    format!(
        "the agents answered each other {MOST_HOPS} times without a person; a person's message \
         starts the count again; nothing is {verb}"
    )
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
/// the revisions of and uploads the links of their key chains, the wraps of their current epochs
/// and their queue key objects the server lacks. Gives the last accepted revision of `space` and
/// its accounts' devices, `synced` as the run synced them first, as they are then. Warns of what
/// fails.
pub fn through_mailbox(
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
    queue_keys::upload_missing(follower, &mut link, scope);
    Ok(rosters.remove(&space).expect("the channel's space"))
}

/// `err`, the reason a run sends nothing, saying how many envelopes earlier runs left in the
/// outbox of the state in `dir` within `scope` still wait, if any: only a run that sends a message
/// or receives sends them.
fn with_waiting(dir: &Path, scope: Scope, err: String) -> String {
    // A stateless service agent's run has no outbox of earlier runs.
    if service::stateless() {
        return err;
    }
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

/// The devices admitted to `channel` in `roster`, the space's latest revision, but `this` one,
/// each with its valid certificate among `devices` ("Admitted devices"): the `client` devices of
/// the accounts of the channel's members that their member's `excluded` does not list, and the
/// `service` devices of the channel's `services`, of a member's account. The certificates of
/// `own`'s account are checked against the master key it pinned, any other's against the
/// revision's.
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
    for member in body.members.iter() {
        for cert in devices.of(roster, own, member) {
            let body = cert.body();
            if body.kind == DeviceKind::Service
                && body.device != this
                && channel.services.contains(&body.device)
            {
                admitted.insert(body.device, cert);
            }
        }
    }
    admitted
}

/// The address the message is from: the person's, or `agent`'s, of the device's account in
/// `roster`'s space, on `server`, the server of the machine's session.
pub fn sender(
    follower: &Follower,
    roster: &Roster,
    server: ServerName,
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
    let (space, owner) = (body.name.clone(), member.handle.clone());
    Ok(match agent {
        Some(agent) => Address::new(space, owner, agent.clone(), server),
        None => Address::person(space, owner, server),
    })
}

/// The content of `draft` from `from`, of `msg_id`, with `counter`.
pub fn content(
    from: &Address,
    draft: &Draft,
    msg_id: MessageId,
    counter: std::num::NonZeroU64,
) -> Content {
    let a2a = serde_json::json!({
        "messageId": msg_id.to_string(),
        "role": "ROLE_USER",
        "parts": [{ "text": draft.text }],
    })
    .to_string();
    Content {
        from: from.clone(),
        to: draft.to.clone(),
        counter: Some(counter),
        hops: draft.hops,
        reply_to: draft.reply_to,
        body: Some(Body::A2a(a2a)),
        delegation: None,
        grant: None,
        attachments: None,
        unknown: Unknown::default(),
    }
}

/// What `--to` names, lowercased.
#[derive(Debug, PartialEq)]
pub enum Addressee {
    Full(Address),
    /// `owner/agent`, in the channel's space on the machine's server.
    Short(Name, Name),
}

/// The agent `given` to `--to` names, lowercased (`docs/protocol/README.md`, "Agent address"):
/// its full address or `owner/agent`. Fails for an invalid address, and for a person's: only a
/// reply goes to a person.
pub fn addressee(given: &str) -> Result<Addressee, String> {
    let lowered = given.to_lowercase();
    let invalid = |err: &dyn std::fmt::Display| {
        format!("--to \"{}\": {err}; nothing is sent", printable(given))
    };
    let form = "invalid agent address: use space/owner/agent@server, or owner/agent in the \
                channel's space";
    let addressee = parsed(&lowered, &invalid, form)?;
    let agent = match &addressee {
        Addressee::Full(address) => address.agent(),
        Addressee::Short(_, agent) => agent,
    };
    if agent.as_str() == PERSON {
        return Err(format!(
            "--to \"{}\" is a person's address: --to takes an agent's, as only a reply goes to a \
             person; nothing is sent",
            printable(given)
        ));
    }
    Ok(addressee)
}

/// What `lowered`, a lowercased address, names: a full address or `owner/agent`. Fails with
/// `invalid` of the problem, `form` for an address of neither form.
pub fn parsed(
    lowered: &str,
    invalid: &dyn Fn(&dyn std::fmt::Display) -> String,
    form: &str,
) -> Result<Addressee, String> {
    if lowered.contains('@') {
        return Ok(Addressee::Full(
            lowered.parse().map_err(|err| invalid(&err))?,
        ));
    }
    let Some((owner, agent)) = lowered.split_once('/').filter(|(_, it)| !it.contains('/')) else {
        return Err(invalid(&form));
    };
    let segment = |segment, name: &str| {
        name.parse::<Name>()
            .map_err(|err| invalid(&AddressError::Segment(segment, err)))
    };
    Ok(Addressee::Short(
        segment(Segment::Owner, owner)?,
        segment(Segment::Agent, agent)?,
    ))
}

impl Addressee {
    /// The full address in the space `space` on the server `server`; fails for an address of
    /// another space or server.
    pub fn in_space(self, space: &Name, server: &ServerName) -> Result<Address, String> {
        let address = match self {
            Addressee::Full(address) => address,
            Addressee::Short(owner, agent) => {
                Address::new(space.clone(), owner, agent, server.clone())
            }
        };
        if address.space() != space {
            // The agent's output names nothing of another space.
            return Err(format!(
                "--to names an agent of another space than the channel's, {space}: a message is \
                 only for the agents of its channel's space; nothing is sent"
            ));
        }
        if address.server() != server {
            return Err(format!(
                "--to names an agent on another server than this device's, {server}; nothing is \
                 sent"
            ));
        }
        Ok(address)
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
                report: outgoing.report,
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
                Some((socket, pushed)) => (unanswered.iter())
                    .try_for_each(|queued| {
                        (socket.send(Message::Binary(queued.frame().to_vec().into())))
                            .map_err(|err| (broke(err), false))
                    })
                    .and_then(|()| {
                        let read = (&*follower, &mut *answers, until);
                        read_answers((socket, pushed), queued, read)
                    }),
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
/// `until`, taking each answered envelope out of the outbox and keeping each push it meets in
/// `pushed`; fails if the connection ends, saying whether the server closed it for a policy
/// violation.
fn read_answers<W: Wire>(
    (socket, pushed): (&mut WebSocket, &mut Vec<Push>),
    queued: &[W],
    (follower, answers, until): (&Follower, &mut BTreeMap<Key, Answer>, Instant),
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
        let (send_key, answer) = match mailbox::read_answer(socket, pushed) {
            Ok(Message::Binary(bytes)) => match Frame::from_cbor(&bytes) {
                Ok(Frame::Stored(stored)) => (stored.send_key, Answer::Stored),
                Ok(Frame::SendRefused { send_key, problem }) => {
                    (send_key, Answer::Refused(problem))
                }
                // A push, which `read_answer` kept, or a frame it does not know or that does not
                // decode, ignored.
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
        tungstenite::Error::Io(err) if connection::timed_out(err)
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

/// Why the server closed a connection, by its close `frame`, which it notes
/// ([`mailbox::note_closing`]).
pub fn closed(frame: Option<&CloseFrame>) -> String {
    mailbox::note_closing(frame);
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
        "rate-limited" => "its device sends too many messages too fast for the server's rate limit",
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
    /// `sent #<ref>` per message of the agent once every one is stored, otherwise each in full:
    /// an agent's `skald send`.
    Sent(&'a str),
    /// Nothing: an agent's `skald recv`, which shows the refusals of its messages as statuses.
    Quiet,
}

/// A message a run sent: its ID, its channel and its envelopes.
type Queue<'q, W> = Vec<(MessageId, ChannelId, Vec<&'q W>)>;

/// Says what the server answered to each message of `queued`, but receipts, as `reported` says,
/// by the state `state` and the answers in `delivered`; fails unless every one is stored, saying
/// which are not.
pub fn report<W: Shown>(
    state: &State,
    queued: &[W],
    delivered: &Delivered,
    timeout: Duration,
    reported: Reported,
) -> Result<(), String> {
    let mut messages: Queue<W> = Vec::new();
    // A receipt is no message of the device's user.
    for queued in queued.iter().filter(|queued| queued.reports()) {
        let (msg_id, _) = queued.key();
        match messages.iter_mut().find(|(id, ..)| *id == msg_id) {
            Some((.., envelopes)) => envelopes.push(queued),
            None => messages.push((msg_id, queued.channel(), vec![queued])),
        }
    }
    let outcome = outcome(&messages, delivered, timeout);
    let references = || state.kept_ids().map(References::new).map_err(text);
    if let Reported::Json(Some(agent)) | Reported::Sent(agent) = reported {
        let mut own = Vec::new();
        for message in messages {
            let sent = state.sent(message.0).map_err(text)?;
            if sent.and_then(|sent| sent.agent).as_deref() == Some(agent) {
                own.push(message);
            }
        }
        messages = own;
    }
    match reported {
        Reported::Sent(_) if self::outcome(&messages, delivered, timeout).is_ok() => {
            let references = references()?;
            for (id, ..) in &messages {
                println!("sent #{}", references.of(*id));
            }
        }
        Reported::Full | Reported::Sent(_) => in_full(&messages, delivered),
        Reported::Json(_) => {
            let references = references()?;
            for (id, channel, envelopes) in &messages {
                let envelopes: Vec<Value> = (envelopes.iter())
                    .map(|queued| {
                        let recipient = queued.key().1.map(|device| device.to_string());
                        match delivered.answers.get(&queued.key()) {
                            Some(Answer::Stored) => {
                                json!({ "recipient": recipient, "answer": "stored" })
                            }
                            Some(Answer::Refused(problem)) => json!({
                                "recipient": recipient,
                                "answer": "refused",
                                "problem": problem,
                            }),
                            None if queued.kept() => {
                                json!({ "recipient": recipient, "answer": "waiting" })
                            }
                            None => json!({ "recipient": recipient, "answer": "unanswered" }),
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
fn in_full<W: Shown>(messages: &Queue<W>, delivered: &Delivered) {
    for (id, channel, envelopes) in messages {
        println!("The message {id} to the channel {channel}:");
        for queued in envelopes {
            let device =
                (queued.key().1).map_or_else(|| "channel".to_owned(), |device| device.to_string());
            match delivered.answers.get(&queued.key()) {
                Some(Answer::Stored) => println!("  {device}  stored"),
                Some(Answer::Refused(problem)) => {
                    println!("  {device}  refused: {}", explained(problem));
                }
                None if queued.kept() => {
                    println!("  {device}  no answer yet: it waits in the outbox");
                }
                None => println!("  {device}  no answer"),
            }
        }
    }
}

/// Fails unless the server stored every envelope of `messages` in `delivered`, saying which it has
/// not answered within `timeout`, and why the last attempt failed, or else which it refused.
fn outcome<W: Shown>(
    messages: &Queue<W>,
    delivered: &Delivered,
    timeout: Duration,
) -> Result<(), String> {
    let (mut left, mut refused) = (Vec::new(), Vec::new());
    let mut kept = true;
    for (id, _, envelopes) in messages {
        let mut waiting = Vec::new();
        for queued in envelopes {
            let recipient = queued.key().1;
            match delivered.answers.get(&queued.key()) {
                Some(Answer::Stored) => {}
                Some(Answer::Refused(problem)) => refused.push(format!(
                    "the message {id}{}: {}",
                    to_device(recipient),
                    explained(problem)
                )),
                None => {
                    kept &= queued.kept();
                    waiting.extend(recipient.map(|device| device.to_string()));
                }
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
        let then = if kept {
            "the outbox keeps it, and the next skald send or skald recv sends it first"
        } else {
            "a stateless run keeps nothing, so nothing sends it later"
        };
        return Err(format!(
            "the server has not stored {} within {} s{why}; {then}",
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

    /// `--to` takes a full address or `owner/agent`, lowercased, and completes the short form with
    /// the channel's space and server; it refuses a person's address, an invalid one, and one of
    /// another space or server.
    #[test]
    fn to_names_an_agent_of_the_channel_s_space_and_server() {
        let space: Name = "acme".parse().unwrap();
        let server: ServerName = "skald.example".parse().unwrap();
        let full: Address = "acme/bob/claude@skald.example".parse().unwrap();
        let in_acme = |given: &str| addressee(given)?.in_space(&space, &server);

        for given in [
            "acme/bob/claude@skald.example",
            "Bob/Claude",
            "ACME/BOB/CLAUDE@SKALD.EXAMPLE",
        ] {
            assert_eq!(in_acme(given).as_ref(), Ok(&full), "{given}");
        }
        for person in ["bob/user", "bob/USER", "acme/bob/user@skald.example"] {
            let err = addressee(person).expect_err(person);
            assert!(err.contains("person's address"), "{person}: {err}");
        }
        for invalid in [
            "claude",
            "bob/",
            "/claude",
            "a/b/c",
            "bob/claude_1",
            "acme/bob/claude@",
        ] {
            let err = addressee(invalid).expect_err(invalid);
            assert!(err.contains("invalid agent address"), "{invalid}: {err}");
        }
        let err = in_acme("other/bob/claude@skald.example").unwrap_err();
        assert!(err.contains("another space"), "{err}");
        let err = in_acme("acme/bob/claude@elsewhere.example").unwrap_err();
        assert!(err.contains("another server"), "{err}");
    }

    /// Puts `outgoing`, the first message in `channel`, in the outbox of `state`.
    fn put(state: &State, channel: ChannelId, outgoing: Outgoing) {
        let message = Counted {
            outgoing,
            channel,
            counter: 1,
            agent: None,
            addressee: None,
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
                github: None,
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
    /// and the `service` devices of the channel's `services` with one of a member's account; this
    /// one, a browser, one the member's `excluded` lists, one with a certificate by another key,
    /// one of a member outside the channel, a `service` device the channel does not list and one
    /// with a forged certificate aside.
    #[test]
    fn admitted_devices_are_the_channels_valid_client_and_service_devices_but_this_one() {
        let (ada, bob, carol) = (Account::new(), Account::new(), Account::new());
        let id = |cert: &Cert| cert.body().device;
        let (this, _, this_key) = ada.client();
        let (laptop, laptop_hpke, _) = ada.client();
        let (browser, ..) = ada.device(DeviceKind::Browser, &this_key, Some(id(&this)));
        let (phone, phone_hpke, _) = bob.client();
        let (barred, ..) = bob.client();
        let (forged, ..) = bob.device(DeviceKind::Client, &SigningKey::generate(), None);
        let (outside, ..) = carol.client();
        let (bot, bot_hpke, _) = ada.device(DeviceKind::Service, &this_key, Some(id(&this)));
        let (unlisted, ..) = ada.device(DeviceKind::Service, &this_key, Some(id(&this)));
        let (fake, ..) = ada.device(DeviceKind::Service, &SigningKey::generate(), None);
        let certs = vec![
            this, laptop, browser, phone, barred, forged, outside, bot, unlisted, fake,
        ];

        let mut body = first(ada.id).body().clone();
        let mut bobs = member(bob.id, "bob", bob.master.public_key());
        bobs.excluded = Some(NonEmpty::new(id(&certs[4])));
        body.members.push(bobs);
        body.members
            .push(member(carol.id, "carol", carol.master.public_key()));
        let channel = ChannelId::generate();
        body.channels = vec![channel_of(channel, &[ada.id, bob.id])];
        body.channels[0].services = vec![id(&certs[7]), id(&certs[9])];
        let roster = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
        let own = Own {
            account: ada.id,
            master: ada.master.public_key(),
        };
        let devices = SpaceDevices {
            ids: certs.iter().map(id).collect(),
            certs: certs.clone(),
            revocations: Vec::new(),
            records: Vec::new(),
        };

        let given: BTreeMap<DeviceId, [u8; 32]> =
            admitted(&own, id(&certs[0]), &roster, &devices, channel)
                .into_iter()
                .map(|(device, cert)| (device, cert.body().hpke))
                .collect();

        assert_eq!(
            given,
            BTreeMap::from([
                (id(&certs[1]), laptop_hpke),
                (id(&certs[3]), phone_hpke),
                (id(&certs[7]), bot_hpke),
            ])
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
                report: true,
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
    /// outbox; a person's sends every one. A receipt goes with `report` false, a message with
    /// `report` true.
    #[test]
    fn queued_envelopes_are_those_of_the_scope_in_order() {
        let dir = TestDir::new("send-queued");
        let state = State::open(&dir).unwrap();
        let (own, other) = (SpaceId::generate(), SpaceId::generate());
        let mut messages = Vec::new();
        for (space, report) in [(own, true), (other, true), (own, false)] {
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
                report,
            };
            put(&state, channel, outgoing);
            messages.push((msg_id, channel, report));
        }
        let ids = |scope| -> Vec<(MessageId, ChannelId, bool)> {
            queued(&state, scope)
                .unwrap()
                .iter()
                .map(|queued| {
                    let Ok(Frame::Send { report, .. }) = Frame::from_cbor(&queued.frame) else {
                        panic!("no send frame");
                    };
                    (queued.outgoing.msg_id, queued.channel, report)
                })
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
