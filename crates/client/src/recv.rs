//! `skald recv`: the device gets its mailbox (`docs/protocol/delivery.md`, "Mailbox"). It first
//! sends what is left in the outbox and the history entries of the messages the server stored
//! ([`crate::history`]), then states its cursor, writes each entry it gets after the cursor to the
//! local state with the new cursor at once, acks, and stops once catch-up has sent every entry. It
//! then accepts the revisions of each space it follows, and those of the space of each
//! `space-notice` that came meanwhile ("Frames"). It then goes through the Welcomes, Commits
//! and `mls` application messages it wrote, decrypting each message in its channel's group, and
//! brings the group of each channel of those spaces in line ([`crate::groups`]), uploading the links
//! of their key chains the server lacks ([`crate::key_chain`]). It then shows the other entries it
//! wrote: a `message` only if it passes the checks of `docs/protocol/README.md`, "Envelope",
//! "Scheme `mls`" and "Scheme `hpke`", with the space's latest revisions accepted
//! first, the sending device's certificate valid and the channel's members taken from its group:
//! the sending device under `mls`, the group as this device holds it under `hpke`; only if
//! [`crate::gaps`] accepts its `counter`, warning first of the counters it makes missing and
//! marking it as following missing ones, or, without `counter`, once per sending device and
//! `msg_id`; a duplicate or a replay without a word; a warning for any other; and each `delivered`
//! and `expired` of the device's messages. As an agent, only those of its folder's space: the rest
//! wait in the local state.
//!
//! A message that passes the checks is kept for the person and for the agents it is for
//! (`docs/protocol/delivery.md`, "Device to agent"), among the agents this device had run as in
//! the space when it wrote the entry: with `to`, the one it names; without `to`, each; a message to
//! a person, none. The person's run shows each kept message once,
//! whichever run checked it. An agent's run shows the messages kept for it and marks each handed
//! over only once its line is written, so a run killed in between shows it again; a message waits
//! for an agent until its envelope's `ttl` ends, at most [`AGENT_WAIT`], after the device wrote it.
//!
//! Each status of the device's messages is kept for the person and, for a message an agent sent,
//! for that agent. The person's run shows each kept status once, and each warning an agent's run
//! held back for it; an agent's run shows those of its messages it has yet to take. A run shows
//! them in the form [`crate::forms`] gives: in full as the person, short as an agent, where a
//! status is shown only if `expired` and a warning in a line of its own without IDs, and as JSON
//! with `--json`.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use std::path::Path;
use std::time::Duration;

use skald_crypto::{HpkeError, HpkeMessage, MemberDevice, StateKey, open_message};
use skald_proto::{
    AccountId, Address, Body, Cbor as _, Cert, ChannelId, Content, DeviceId, DeviceKind, Envelope,
    Frame, Header, MessageId, Name, Push, Roster, RosterBody, ServerName, ServerStatus, SpaceId,
    VersionRange,
};

use crate::agents::server_name;
use crate::connection::printable;
use crate::devices::text;
use crate::follow::{Own, SpaceDevices};
use crate::forms::{self, Form, References};
use crate::gaps::{self, Accepted, Source};
use crate::groups::{self, Link, Opened, Rosters};
use crate::history;
use crate::key_chain;
use crate::mailbox::{self, Mailbox};
use crate::output::{self, Stream};
use crate::secrets;
use crate::send::{self, Reported};
use crate::spaces::{self, Follower, Scope};
use crate::state::{self, Kept, State};
use crate::wraps;

/// How long a message waits for an agent at most: the cloud's mailbox retention
/// (`architecture.md`, "Cloud limits").
pub const AGENT_WAIT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Receives the mailbox of the device of the state in `dir`, its secrets in `storage`, as `agent`
/// if given, whose run holds its warnings for the person ([`holding_warnings`]); waits for each
/// frame for `timeout`; shows as JSON if `json`. Gives whether it showed what `skald recv --wait`
/// waits for: as an agent, a message handed over to it; as the person, anything. For such a run,
/// if `waits`, leaves its connection to watch ([`mailbox::leave_watched`]).
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    agent: Option<&Name>,
    (waits, json): (bool, bool),
) -> Result<bool, String> {
    holding_warnings(dir, agent.is_some(), || {
        let form = Form::of(agent.is_some(), json);
        received(dir, storage, (speaks, timeout), agent, (waits, form))
    })
}

/// Runs `run`, a run as an agent if `agent`; such a run holds back each of its warnings and keeps
/// it in the state in `dir` for the person's run, or, if it cannot, shows it.
pub fn holding_warnings<R>(dir: &Path, agent: bool, run: impl FnOnce() -> R) -> R {
    if !agent {
        return run();
    }
    let (ran, held) = output::holding_warnings(run);
    if !held.is_empty()
        && let Err(err) = State::open(dir).and_then(|state| state.keep_warnings(&held))
    {
        for warning in &held {
            eprintln!("{warning}");
        }
        eprintln!("skald: warning: the local state keeps no warning for the person: {err}");
    }
    ran
}

/// [`run`], showing in `form`.
fn received(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    agent: Option<&Name>,
    (waits, form): (bool, Form),
) -> Result<bool, String> {
    let written = output::written();
    let scope = Scope::of_run(dir, agent.is_some())?;
    let mut follower = spaces::following(dir, storage, speaks, timeout, scope)?;
    let ran_as = match (scope, agent) {
        (Scope::Only(space), Some(agent)) => {
            follower
                .state
                .add_agent(space, agent.as_str())
                .map_err(text)?;
            Some((space, agent))
        }
        _ => None,
    };
    let mut link = Link::new(&follower)?;
    if let Err(err) = groups::flush(&mut follower, &mut link, scope) {
        eprintln!("skald: warning: {err}");
    }
    let queued = send::queued(&follower.state, scope)?;
    let mut delivered = send::deliver(&mut follower, &queued, timeout, None);
    let reported = match (form, agent) {
        (Form::Json, agent) => Reported::Json(agent.map(Name::as_str)),
        (_, Some(agent)) => Reported::Refusals(agent.as_str()),
        (_, None) => Reported::Full,
    };
    if let Err(err) = send::report(&follower.state, &queued, &delivered, timeout, reported) {
        eprintln!("skald: warning: {err}");
    }
    // The entries go out on the outbox's connection, which then gets the mailbox.
    let flushed = std::mem::replace(&mut link.mailbox, delivered.mailbox.take());
    history::upload(&mut follower, &mut link, scope);
    let mailbox = std::mem::replace(&mut link.mailbox, flushed);
    let mut synced = BTreeMap::new();
    let received = receive(&mut follower, mailbox, (timeout, scope), &mut synced);
    let aligned = align(&mut follower, &mut link, scope, &mut synced);
    let shown = show(&mut follower, (scope, ran_as.is_some()), synced);
    let handed = match ran_as {
        Some((space, agent)) => hand_over(&follower, (space, agent), form),
        None => show_to_person(&follower, form),
    };
    if waits {
        mailbox::leave_watched(follower);
    }
    received?;
    aligned?;
    shown?;
    let handed = handed?;
    Ok(match ran_as {
        Some(_) => handed > 0,
        None => output::written() > written,
    })
}

/// Goes through the Welcomes, Commits and application messages the state holds within `scope`,
/// then brings the groups of the channels of each space the run accepted the revisions of in line
/// ([`crate::groups`]) and uploads the links of their key chains ([`crate::key_chain`]) and the
/// wraps of their current epochs ([`crate::wraps`]) the server lacks.
pub fn align(
    follower: &mut Follower,
    link: &mut Link,
    scope: Scope,
    synced: &mut Rosters,
) -> Result<(), String> {
    groups::take_in(follower, link, scope, synced)?;
    groups::align_with(follower, link);
    key_chain::upload_missing(follower, link, scope);
    wraps::upload_missing(follower, link, scope);
    Ok(())
}

/// Gets the mailbox over `mailbox`, or a new connection as the device, until catch-up ends,
/// waiting for each frame for `timeout`, as [`catch_up`] does. Then it syncs into `synced` each
/// space within `scope` the device follows, and the space of each `space-notice` that came
/// meanwhile.
fn receive(
    follower: &mut Follower,
    mailbox: Option<Mailbox>,
    (timeout, scope): (Duration, Scope),
    synced: &mut Rosters,
) -> Result<(), String> {
    let mut mailbox = catch_up(follower, mailbox, timeout)?;
    let keys = follower.keys()?;
    // By the end of catch-up the server sends the connection its notices: a revision stored before
    // these fetches is in them, and one stored later comes as a notice. A notice in catch-up names a
    // space fetched here.
    let mut due: BTreeSet<SpaceId> = follower
        .state
        .spaces()
        .map_err(text)?
        .iter()
        .map(|space| space.id)
        .collect();
    while !due.is_empty() {
        sync_due(follower, scope, &mut due, synced)?;
        due.extend(mailbox.waiting_notices(follower, &keys));
    }
    Ok(())
}

/// Gets the mailbox over `mailbox`, or a new connection as the device, until catch-up ends,
/// waiting for each frame for `timeout`, over the transports as [`crate::mailbox`] says: writes
/// each entry after the cursor with the new cursor and acks it. Gives the connection.
pub fn catch_up(
    follower: &mut Follower,
    mailbox: Option<Mailbox>,
    timeout: Duration,
) -> Result<Mailbox, String> {
    let keys = follower.keys()?;
    let mailbox = match mailbox {
        Some(mailbox) => mailbox,
        None => follower.connect(&keys)?,
    };
    mailbox.catch_up(follower, &keys, timeout, |follower, entries, cursor| {
        let written = after(entries, cursor);
        if written.is_empty() {
            return Ok(cursor);
        }
        follower.state.write_entries(&written).map_err(text)
    })
}

/// Syncs into `synced` each space of `due` within `scope` that the device follows, and empties
/// `due`. A space that fails to sync is warned of, and synced again at its first message.
fn sync_due(
    follower: &mut Follower,
    scope: Scope,
    due: &mut BTreeSet<SpaceId>,
    synced: &mut Rosters,
) -> Result<(), String> {
    if due.is_empty() {
        return Ok(());
    }
    let followed = follower.state.spaces().map_err(text)?;
    for space in followed
        .iter()
        .filter(|space| due.contains(&space.id) && !scope.excludes(space.id))
    {
        match follower.sync(space) {
            Ok(done) => {
                synced.insert(space.id, done);
            }
            Err(err) => {
                synced.remove(&space.id);
                eprintln!("skald: warning: {err}");
            }
        }
    }
    due.clear();
    Ok(())
}

/// The spaces of the `space-notice` entries of `entries`.
pub fn noticed(entries: &[Push]) -> impl Iterator<Item = SpaceId> + '_ {
    entries.iter().filter_map(|push| match push {
        Push::SpaceNotice { space } => Some(*space),
        _ => None,
    })
}

/// What a frame from the server gives a run that waits for the end of catch-up.
#[derive(Debug, PartialEq)]
pub enum Got {
    /// Entries of the mailbox.
    Entries(Vec<Push>),
    /// The end of catch-up: an empty `batch`.
    End,
    /// Nothing: a frame it does not know, does not await or that does not decode.
    Nothing,
}

/// What `frame`, a frame from the server if it decodes, gives.
pub fn got(frame: Option<Frame>) -> Got {
    match frame {
        Some(Frame::Push(push)) => Got::Entries(vec![push]),
        Some(Frame::Batch(entries)) if entries.is_empty() => Got::End,
        Some(Frame::Batch(entries)) => Got::Entries(entries),
        _ => Got::Nothing,
    }
}

/// The `message` and `status` entries of `entries` after `cursor`, by `seq`, each as its frame:
/// one up to it is a redelivery after a lost ack.
fn after(entries: Vec<Push>, cursor: u64) -> Vec<(u64, Vec<u8>)> {
    entries
        .into_iter()
        .filter_map(|push| {
            let seq = match &push {
                Push::Message { seq, .. } | Push::Status { seq, .. } => seq.get(),
                Push::SpaceNotice { .. } | Push::Unknown(_) => return None,
            };
            (seq > cursor).then(|| (seq, Frame::Push(push).to_cbor()))
        })
        .collect()
}

/// What a run checks messages with, besides the state: the spaces it synced, each one's last
/// accepted revision and its accounts' devices, the server's name, once it asked for it, and the
/// key that opens what decrypting the mailbox's application messages gave, once it needed it.
struct Checks {
    keys: crate::device::Keys,
    followed: Vec<state::Space>,
    synced: Rosters,
    server: Option<Option<ServerName>>,
    inbox_key: Option<StateKey>,
}

/// Goes through each entry the state holds within `scope`, in `seq` order, and takes it out,
/// keeping each message that passes the checks for the person and the agents it is for, and each
/// status for the person and the agent that sent its message; checks messages against the spaces
/// as `synced` holds them, syncing any other at its first message. Warns of each message it drops
/// and of the messages missing before one; as an agent, if `agent`, also in a short line.
fn show(
    follower: &mut Follower,
    (scope, agent): (Scope, bool),
    synced: Rosters,
) -> Result<(), String> {
    let mut checks = Checks {
        keys: follower.keys()?,
        followed: follower.state.spaces().map_err(text)?,
        synced,
        server: None,
        inbox_key: None,
    };
    for entry in follower.state.inbox().map_err(text)? {
        let (seq, bytes) = (entry.seq, &entry.frame);
        // A Welcome or Commit waits for a run that can check it.
        if groups::is_handshake(bytes) {
            continue;
        }
        match Frame::from_cbor(bytes) {
            Ok(Frame::Push(Push::Message { envelope, .. })) => {
                let envelope = envelope.envelope();
                if scope.excludes(envelope.header.space) {
                    continue;
                }
                let verdict = if groups::is_application(bytes) {
                    let key = match &checks.inbox_key {
                        Some(key) => key,
                        None => checks.inbox_key.insert(groups::inbox_key(follower)?),
                    };
                    // An application message waits for a run that decrypts it in its group.
                    let Some(opened) = groups::opened(follower, key, &entry) else {
                        continue;
                    };
                    decrypted(follower, &mut checks, (seq, envelope), opened)?
                } else {
                    verdict(follower, &mut checks, (seq, envelope))?
                };
                let header = &envelope.header;
                match verdict {
                    Verdict::Show(sender, message, counted) => {
                        let gap = counted.as_ref().and_then(|(_, accepted)| accepted.gap);
                        if let Some(gap) = gap {
                            eprintln!("skald: warning: {}", missing(header.channel, sender, gap));
                            if agent {
                                let reference = reference(follower, header.msg_id)?;
                                eprintln!(
                                    "skald: {}",
                                    missing_before(&message.device, gap, &reference)
                                );
                            }
                        }
                        let kept =
                            kept(follower, &mut checks, (seq, header, sender), &message, gap)?;
                        match counted {
                            Some((source, accepted)) => (follower.state).keep_counted(
                                seq,
                                (&source, &accepted.counters),
                                &kept,
                            ),
                            None => follower.state.keep(seq, (sender, header.msg_id), &kept),
                        }
                        .map_err(text)?;
                    }
                    Verdict::Repeat => follower.state.remove_entry(seq).map_err(text)?,
                    Verdict::Drop(sender, why) => {
                        let from = sender.map_or_else(String::new, |sender| {
                            format!(" from the device {sender}")
                        });
                        eprintln!(
                            "skald: warning: dropped the message {} in the channel {}{from}: \
                             {why}; it is not shown",
                            header.msg_id, header.channel
                        );
                        if agent {
                            let reference = reference(follower, header.msg_id)?;
                            let name = sender
                                .and_then(|sender| device_name(&checks, header.space, sender));
                            eprintln!("skald: {}", dropped(&reference, name.as_deref(), &why));
                        }
                        follower.state.remove_entry(seq).map_err(text)?;
                    }
                }
            }
            Ok(Frame::Push(Push::Status { status, .. })) => {
                let sent = follower.state.sent(status.msg_id).map_err(text)?;
                let space = sent.as_ref().map(|sent| sent.space);
                if let Scope::Only(only) = scope
                    && space != Some(only)
                {
                    continue;
                }
                let kept = forms::Status {
                    msg_id: status.msg_id.to_string(),
                    space: space.map(|space| space.to_string()),
                    channel: sent.as_ref().map(|sent| sent.channel.to_string()),
                    status: match status.status {
                        ServerStatus::Delivered => "delivered".to_owned(),
                        ServerStatus::Expired => forms::EXPIRED.to_owned(),
                    },
                    device: status.device.to_string(),
                    device_name: space.and_then(|space| device_name(&checks, space, status.device)),
                    timestamp: status.timestamp,
                };
                let json = serde_json::to_string(&kept).expect("a status encodes");
                let agent = sent.as_ref().and_then(|sent| sent.agent.as_deref());
                (follower.state)
                    .keep_status(seq, (space, agent), &json)
                    .map_err(text)?;
            }
            // This client wrote only entries it reads.
            _ => follower.state.remove_entry(seq).map_err(text)?,
        }
    }
    Ok(())
}

/// How the message of the entry `seq` with `header` from the device `sender`, which passed the
/// checks, is kept: its line, sealed, and the agents it is for; `gap`, the counters it makes
/// missing, if any.
fn kept(
    follower: &Follower,
    checks: &mut Checks,
    (seq, header, sender): (u64, &Header, DeviceId),
    message: &Received,
    gap: Option<(u64, u64)>,
) -> Result<Kept, String> {
    let record = forms::Message {
        msg_id: header.msg_id.to_string(),
        space: header.space.to_string(),
        channel: header.channel.to_string(),
        from: message.from.to_string(),
        device: sender.to_string(),
        device_name: message.device.clone(),
        text: message.text.clone(),
        after_missing: gap.is_some(),
    };
    let record = serde_json::to_vec(&record).expect("a message encodes");
    let key = match &checks.inbox_key {
        Some(key) => key,
        None => checks.inbox_key.insert(groups::inbox_key(follower)?),
    };
    Ok(Kept {
        space: header.space,
        msg_id: header.msg_id,
        line: key.seal(follower.this, &kept_item(seq), &record),
        wait: agent_wait(header.ttl),
        agents: agents_of(follower, checks, (seq, header), message.to.as_ref())?,
    })
}

/// How long a message whose envelope has the `ttl` `ttl`, if any, waits for an agent, in seconds:
/// its `ttl`, at most [`AGENT_WAIT`].
fn agent_wait(ttl: Option<u64>) -> u64 {
    let most = AGENT_WAIT.as_secs();
    ttl.map_or(most, |ttl| ttl.min(most))
}

/// The agents the message of the entry `seq` with `header` is for, `to` its content's `to`, among
/// the agents this device had run as in the space when it wrote the entry: the one `to` names, if
/// it is an agent of this account in the space on this server; without `to`, each; none for a
/// person.
fn agents_of(
    follower: &Follower,
    checks: &Checks,
    (seq, header): (u64, &Header),
    to: Option<&Address>,
) -> Result<Vec<String>, String> {
    let agents = (follower.state).agents(header.space, seq).map_err(text)?;
    let Some(to) = to else {
        return Ok(agents);
    };
    let Some((roster, _)) = checks.synced.get(&header.space) else {
        return Ok(Vec::new());
    };
    let body = roster.body();
    let handle = body
        .member(follower.own.account)
        .map(|member| &member.handle);
    let server = checks.server.as_ref().and_then(Option::as_ref);
    let ours = !to.is_person()
        && to.space() == &body.name
        && Some(to.owner()) == handle
        && Some(to.server()) == server;
    Ok(agents
        .into_iter()
        .filter(|agent| ours && agent == to.agent().as_str())
        .collect())
}

/// The name the state key seals the line of the kept message of the entry `seq` under.
fn kept_item(seq: u64) -> Vec<u8> {
    [b"kept:".as_slice(), &seq.to_be_bytes()].concat()
}

/// Shows each warning an agent's run kept for the person, each status and then each kept message
/// the person's run has yet to show, in `form`, in order, and marks each shown; gives how many
/// messages it showed.
fn show_to_person(follower: &Follower, form: Form) -> Result<usize, String> {
    let state = &follower.state;
    for (position, warning) in state.warnings().map_err(text)? {
        output::write(Stream::Err, format!("{warning}\n").as_bytes())
            .map_err(|err| format!("cannot write a warning to the output: {err}"))?;
        state.warning_shown(position).map_err(text)?;
    }
    let references = References::new(state.kept_ids().map_err(text)?);
    let statuses = state.statuses_for_person().map_err(text)?;
    shown_statuses(statuses, (form, &references), |seq| {
        state.status_shown_to_person(seq)
    })?;
    let kept = state.for_person().map_err(text)?;
    shown_kept(follower, kept, (form, &references), |seq| {
        state.shown_to_person(seq)
    })
}

/// Hands each kept status of a message `agent` sent in `space`, then each kept message that waits
/// for it there, over to it, in `form`, in `seq` order: shows it, then marks it handed over. A
/// message whose wait ended is dropped first. Gives how many messages it handed over.
fn hand_over(
    follower: &Follower,
    (space, agent): (SpaceId, &Name),
    form: Form,
) -> Result<usize, String> {
    let state = &follower.state;
    let references = References::new(state.kept_ids().map_err(text)?);
    let statuses = (state.statuses_for_agent(space, agent.as_str())).map_err(text)?;
    shown_statuses(statuses, (form, &references), |seq| {
        state.status_taken_by_agent(seq)
    })?;
    let kept = (state.for_agent(space, agent.as_str(), state::unix_now())).map_err(text)?;
    shown_kept(follower, kept, (form, &references), |seq| {
        state.handed_over(seq, agent.as_str())
    })
}

/// Writes each of `kept`, kept statuses by `seq` as JSON, in `form` with `references`, to the
/// output, unless the form leaves it out, then calls `mark` with its `seq`; one that does not
/// decode is warned of and marked.
fn shown_statuses(
    kept: Vec<(u64, String)>,
    (form, references): (Form, &References),
    mark: impl Fn(u64) -> Result<(), state::Error>,
) -> Result<(), String> {
    for (seq, json) in kept {
        let status = serde_json::from_str::<forms::Status>(&json).ok();
        let msg_id = (status.as_ref()).and_then(|status| status.msg_id.parse().ok());
        match (status, msg_id) {
            (Some(status), Some(msg_id)) => {
                if let Some(line) = status.shown(form, &references.of(msg_id)) {
                    output::write(Stream::Out, format!("{line}\n").as_bytes())
                        .map_err(|err| format!("cannot write a status to the output: {err}"))?;
                }
            }
            _ => eprintln!(
                "skald: warning: a status this device kept does not decode, so it is not shown"
            ),
        }
        mark(seq).map_err(text)?;
    }
    Ok(())
}

/// Writes each of `kept`, kept messages by `seq` with their sealed lines, in `form` with
/// `references`, to the output, then calls `mark` with its `seq`; one that does not open is warned
/// of and marked. A line an older client kept is written as it is. Gives how many it wrote.
fn shown_kept(
    follower: &Follower,
    kept: Vec<(u64, Vec<u8>)>,
    (form, references): (Form, &References),
    mark: impl Fn(u64) -> Result<(), state::Error>,
) -> Result<usize, String> {
    if kept.is_empty() {
        return Ok(0);
    }
    let key = groups::inbox_key(follower)?;
    let mut shown = 0;
    for (seq, sealed) in kept {
        match key.open(follower.this, &kept_item(seq), &sealed) {
            Some(opened) => {
                let line = kept_line(&opened, form, references);
                output::write(Stream::Out, format!("{line}\n").as_bytes())
                    .map_err(|err| format!("cannot write a message to the output: {err}"))?;
                shown += 1;
            }
            None => eprintln!(
                "skald: warning: a message this device kept does not open, so it is not shown"
            ),
        }
        mark(seq).map_err(text)?;
    }
    Ok(shown)
}

/// The line of a kept message, `opened` its record, in `form` with `references`; one an older
/// client kept is its line as it is.
fn kept_line(opened: &[u8], form: Form, references: &References) -> String {
    let record = serde_json::from_slice::<forms::Message>(opened).ok();
    let msg_id = (record.as_ref()).and_then(|record| record.msg_id.parse::<MessageId>().ok());
    match (record, msg_id) {
        (Some(record), Some(msg_id)) => record.shown(form, &references.of(msg_id)),
        _ => String::from_utf8_lossy(opened).into_owned(),
    }
}

/// The reference of `msg_id` among the messages the device of `follower` keeps.
fn reference(follower: &Follower, msg_id: MessageId) -> Result<String, String> {
    let kept = follower.state.kept_ids().map_err(text)?;
    Ok(References::new(kept).of(msg_id))
}

/// The certificate's `name` of `device`, a device of an account of `space`, as the run's checks
/// hold the space, if they do.
fn device_name(checks: &Checks, space: SpaceId, device: DeviceId) -> Option<String> {
    let (_, devices) = checks.synced.get(&space)?;
    (devices.certs.iter())
        .find(|cert| cert.body().device == device)
        .map(|cert| cert.body().name.clone())
}

/// An agent's warning of the message `reference` it dropped from the device named `name`, if
/// known, and why.
fn dropped(reference: &str, name: Option<&str>, why: &str) -> String {
    let from = name.map_or_else(String::new, |name| format!(" from {}", printable(name)));
    format!("dropped #{reference}{from}: {why}")
}

/// An agent's warning of the counters `first..=last` missing from the device named `name` before
/// the message `reference`.
fn missing_before(name: &str, (first, last): (u64, u64), reference: &str) -> String {
    let name = printable(name);
    if first == last {
        format!("a message from {name} is missing before #{reference}")
    } else {
        format!(
            "{} messages from {name} are missing before #{reference}",
            last - first + 1
        )
    }
}

/// What a run does with a `message` entry.
enum Verdict {
    /// Shows the message from the sending device: it passed every check. With `counter`, what
    /// accepting it gave.
    Show(DeviceId, Box<Received>, Option<(Source, Accepted)>),
    /// Drops it without a word: a duplicate or a replay.
    Repeat,
    /// Drops it with a warning that names the sending device, if the payload names one, and why.
    Drop(Option<DeviceId>, String),
}

/// What `envelope`, the `hpke` `message` entry `seq` of a space within the run's scope, gives, by
/// the checks of the run: the space is synced at the first message of it.
fn verdict(
    follower: &mut Follower,
    checks: &mut Checks,
    (seq, envelope): (u64, &Envelope),
) -> Result<Verdict, String> {
    let header = &envelope.header;
    let opened = match open_message(header, &envelope.payload, follower.this, checks.keys.hpke()) {
        Ok(opened) => opened,
        Err(HpkeError::Content { sender, error }) => {
            return Ok(Verdict::Drop(
                Some(sender),
                format!("its content breaks the protocol: {error}"),
            ));
        }
        Err(_) => {
            return Ok(Verdict::Drop(
                None,
                "it does not open as sealed to this device".to_owned(),
            ));
        }
    };
    let sender = opened.sender();
    let held = groups::members(follower, &checks.keys, header.channel)?;
    judge(
        follower,
        checks,
        (seq, header, sender, Group::Held(held.as_deref())),
        signed(opened),
    )
}

/// What gives the content of `opened` with the sending device's certificate: it, once its
/// signature verifies with it.
fn signed(opened: HpkeMessage) -> impl FnOnce(&Cert) -> Result<Content, String> {
    |cert| {
        opened.verify(cert).map_err(|_| {
            "its signature does not verify with the sending device's certificate".to_owned()
        })
    }
}

/// What `envelope`, the `mls` application message entry `seq` of a space within the run's scope,
/// gives, by what decrypting it in its channel's group gave, `opened`, and the checks of the run.
fn decrypted(
    follower: &mut Follower,
    checks: &mut Checks,
    (seq, envelope): (u64, &Envelope),
    opened: Opened,
) -> Result<Verdict, String> {
    match opened {
        Opened::Replay => Ok(Verdict::Repeat),
        Opened::Failed(why) => Ok(Verdict::Drop(None, why)),
        Opened::Read(sender, plaintext) => judge(
            follower,
            checks,
            (seq, &envelope.header, sender, Group::Sender),
            |_| {
                Content::from_cbor(&plaintext)
                    .map_err(|err| format!("its content breaks the protocol: {err}"))
            },
        ),
    }
}

/// What a message of the entry `seq` with `header` from the device `sender`, whose content
/// `content` gives with the sending device's certificate, gives, by the checks of the run, the
/// channel's members taken from `group`, and its `counter`: the space is synced at its first
/// message.
fn judge(
    follower: &mut Follower,
    checks: &mut Checks,
    (seq, header, sender, group): (u64, &Header, DeviceId, Group),
    content: impl FnOnce(&Cert) -> Result<Content, String>,
) -> Result<Verdict, String> {
    let Some(space) = checks
        .followed
        .iter()
        .find(|space| space.id == header.space)
    else {
        return Ok(Verdict::Drop(
            Some(sender),
            format!("its space {} is none this device follows", header.space),
        ));
    };
    if let Entry::Vacant(vacant) = checks.synced.entry(space.id) {
        vacant.insert(follower.sync(space)?);
    }
    if checks.server.is_none() {
        checks.server = Some(server_name(&mut follower.connection)?);
    }
    let (roster, devices) = &checks.synced[&space.id];
    let context = Context {
        own: &follower.own,
        roster,
        devices,
        server: checks.server.as_ref().and_then(Option::as_ref),
        group,
    };
    let message = match check(sender, content, header, &context) {
        Err(why) => return Ok(Verdict::Drop(Some(sender), why)),
        Ok(message) => message,
    };
    let Some(counter) = message.counter else {
        // Content without `counter`: once per sending device and `msg_id`.
        let repeat = (follower.state)
            .received(sender, header.msg_id)
            .map_err(text)?;
        return Ok(if repeat {
            Verdict::Repeat
        } else {
            Verdict::Show(sender, Box::new(message), None)
        });
    };
    let source = Source {
        channel: header.channel,
        device: sender,
        scheme: header.scheme,
    };
    let previous = follower.state.counters(&source, seq).map_err(text)?;
    Ok(match gaps::accept(source.scheme, previous, counter) {
        Some(accepted) => Verdict::Show(sender, Box::new(message), Some((source, accepted))),
        None => Verdict::Repeat,
    })
}

/// The warning of the counters `first..=last` missing from the device `sender` in `channel`.
fn missing(channel: ChannelId, sender: DeviceId, (first, last): (u64, u64)) -> String {
    let from = format!("from the device {sender} in the channel {channel}");
    if first == last {
        format!(
            "the message with the counter {first} {from} is missing: the server refused, expired \
             or dropped it"
        )
    } else {
        format!(
            "{} messages with the counters {first} to {last} {from} are missing: the server \
             refused, expired or dropped them",
            last - first + 1
        )
    }
}

/// Where a check takes the members of a message's channel from: its MLS group.
#[derive(Clone, Copy)]
enum Group<'g> {
    /// The sending device, which decrypting an `mls` message in the group shows a member.
    Sender,
    /// The member devices of the group as this device holds it, none if it holds none: for a
    /// message under `hpke`.
    Held(Option<&'g [MemberDevice]>),
}

/// What a message is checked against: the device's account and the master key it pinned, the
/// space's last accepted revision and its accounts' devices, the server's name, and where the
/// channel's members come from.
struct Context<'c> {
    own: &'c Own,
    roster: &'c Roster,
    devices: &'c SpaceDevices,
    server: Option<&'c ServerName>,
    group: Group<'c>,
}

/// A message that passed the checks.
#[derive(Debug, PartialEq)]
struct Received {
    from: Address,
    to: Option<Address>,
    /// The sending device's certificate `name`.
    device: String,
    text: String,
    counter: Option<NonZeroU64>,
}

/// The valid certificate of `device` of an account of the space in `context`'s revision that its
/// member's `excluded` does not list, checked against the master key the device pinned for its own
/// account and the revision's for any other.
fn valid_cert<'c>(context: &Context<'c>, device: DeviceId) -> Option<&'c Cert> {
    context.roster.body().members.iter().find_map(|member| {
        let certs = context.devices.of(context.roster, context.own, member);
        certs.into_iter().find(|cert| cert.body().device == device)
    })
}

/// Whether a `client` device of `account` is in the channel's group, as `context` takes its
/// members, `cert` the sending device's.
fn in_group(context: &Context, cert: &Cert) -> bool {
    let account = cert.body().account;
    match context.group {
        Group::Sender => cert.body().kind == DeviceKind::Client,
        Group::Held(members) => members.into_iter().flatten().any(|member| {
            valid_cert(context, member.device).is_some_and(|cert| {
                let body = cert.body();
                body.signing == member.signing
                    && body.account == account
                    && body.kind == DeviceKind::Client
            })
        }),
    }
}

/// The message from the device `sender` whose content `content` gives with its certificate, of an
/// envelope with `header`, once it passes the checks against `context`; why it fails otherwise.
fn check(
    sender: DeviceId,
    content: impl FnOnce(&Cert) -> Result<Content, String>,
    header: &Header,
    context: &Context,
) -> Result<Received, String> {
    let body = context.roster.body();
    let (name, revision) = (body.name.as_str(), body.revision);
    let cert = valid_cert(context, sender).ok_or_else(|| {
        format!(
            "revision {revision} of the space {name} lists no account with such a device with a \
             valid certificate that it does not bar"
        )
    })?;
    let content = content(cert)?;
    // Under `hpke`, only the revision binds the channel to the space.
    if matches!(context.group, Group::Held(_))
        && !body.channels.iter().any(|it| it.id == header.channel)
    {
        return Err(format!(
            "revision {revision} of the space {name} has no such channel"
        ));
    }
    if !of_account(&content.from, body, cert.body().account, context.server) {
        return Err(format!(
            "its from is no address of the sending device's account in the space {name}"
        ));
    }
    if !in_group(context, cert) {
        return Err(
            "no client device of the sending device's account is in the channel's group on this \
             device"
                .to_owned(),
        );
    }
    if content.counter.is_none() {
        return Err("its content has no counter".to_owned());
    }
    let text = a2a_text(&content, header.msg_id)?;
    Ok(Received {
        from: content.from,
        to: content.to,
        device: cert.body().name.clone(),
        text,
        counter: content.counter,
    })
}

/// Whether `from` is an address of `account` in the space of `body`, a revision of it, on
/// `server`: check 2 of "Envelope" for a device other than a `service` one.
pub fn of_account(
    from: &Address,
    body: &RosterBody,
    account: AccountId,
    server: Option<&ServerName>,
) -> bool {
    let handle = body.member(account).map(|member| &member.handle);
    from.space() == &body.name && Some(from.owner()) == handle && Some(from.server()) == server
}

/// The text of `content`'s A2A message, its text parts each on a line, once its `messageId` is
/// `msg_id` in text form: check 4 of "Envelope".
pub fn a2a_text(content: &Content, msg_id: MessageId) -> Result<String, String> {
    let Some(Body::A2a(json)) = &content.body else {
        return Err("it holds no A2A message".to_owned());
    };
    let message: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "its A2A message is no JSON".to_owned())?;
    if message["messageId"].as_str() != Some(msg_id.to_string().as_str()) {
        return Err("its A2A messageId is not its msg_id".to_owned());
    }
    Ok(message["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use skald_crypto::{HpkeKey, SigningKey, seal_message};
    use skald_proto::{
        Content, DeliveryStatus, HpkePayload, HpkePlaintext, MessageId, NonEmpty, RawEnvelope,
        Receipt, ReceiptStatus, Scheme, Unknown,
    };

    use super::*;
    use crate::send::tests::{Account, channel_of};
    use crate::spaces::tests::{first, member};

    /// A message waits for an agent for its `ttl`, at most 7 days, and 7 days without one.
    #[test]
    fn message_waits_for_an_agent_for_its_ttl_at_most_seven_days() {
        let week = 7 * 24 * 60 * 60;
        assert_eq!(agent_wait(Some(10)), 10);
        assert_eq!(agent_wait(Some(week + 1)), week);
        assert_eq!(agent_wait(None), week);
    }

    /// Ada's device, this one, in `acme` with Bob, whose device sends in the channel they are both
    /// in, and whose group holds both devices.
    struct World {
        own: Own,
        this: DeviceId,
        hpke: HpkeKey,
        roster: Roster,
        devices: SpaceDevices,
        channel: ChannelId,
        bob: DeviceId,
        bob_key: SigningKey,
        server: ServerName,
        members: Vec<MemberDevice>,
        bob_account: Account,
    }

    impl World {
        fn new() -> Self {
            let (ada, bob) = (Account::new(), Account::new());
            let (this_cert, _, this_key) = ada.client();
            let (bob_cert, _, bob_key) = bob.client();
            let mut body = first(ada.id).body().clone();
            body.members
                .push(member(bob.id, "bob", bob.master.public_key()));
            let channel = ChannelId::generate();
            body.channels = vec![channel_of(channel, &[ada.id, bob.id])];
            let certs = vec![this_cert, bob_cert];
            let members = certs
                .iter()
                .map(|cert| MemberDevice {
                    device: cert.body().device,
                    signing: cert.body().signing,
                })
                .collect();
            World {
                own: Own {
                    account: ada.id,
                    master: ada.master.public_key(),
                },
                this: DeviceId::from_signing_key(&this_key.public_key()),
                hpke: HpkeKey::generate(),
                roster: skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap(),
                devices: SpaceDevices {
                    ids: certs.iter().map(|cert| cert.body().device).collect(),
                    certs,
                    revocations: Vec::new(),
                },
                channel,
                bob: DeviceId::from_signing_key(&bob_key.public_key()),
                bob_key,
                server: "skald.example".parse().unwrap(),
                members,
                bob_account: bob,
            }
        }

        /// What a message is checked against, `roster` the space's revision and the channel's
        /// members taken from `group`.
        fn context<'c>(&'c self, roster: &'c Roster, group: Group<'c>) -> Context<'c> {
            Context {
                own: &self.own,
                roster,
                devices: &self.devices,
                server: Some(&self.server),
                group,
            }
        }

        /// The header of a message of the channel to this device.
        fn header(&self) -> Header {
            Header {
                v: NonZeroU64::MIN,
                space: self.roster.body().space,
                channel: self.channel,
                msg_id: MessageId::generate(),
                scheme: Scheme::Hpke,
                recipients: Some(NonEmpty::new(self.this)),
                ttl: None,
                unknown: Unknown::default(),
            }
        }

        /// Content from Bob's person address with `counter` 1 and the A2A message `json`.
        fn content(&self, json: String) -> Content {
            Content {
                from: format!("acme/bob/user@{}", self.server).parse().unwrap(),
                to: None,
                counter: NonZeroU64::new(1),
                hops: 0,
                reply_to: None,
                body: Some(Body::A2a(json)),
                delegation: None,
                grant: None,
                attachments: None,
                unknown: Unknown::default(),
            }
        }

        /// The A2A message of `header` with the text parts `texts`.
        fn a2a(header: &Header, texts: &[&str]) -> String {
            let parts: Vec<serde_json::Value> = texts
                .iter()
                .map(|text| serde_json::json!({ "text": text }))
                .collect();
            serde_json::json!({
                "messageId": header.msg_id.to_string(),
                "role": "ROLE_USER",
                "parts": parts,
            })
            .to_string()
        }

        /// What checking `content` under `header`, signed with `key` and sealed to this device,
        /// gives, with `roster` as the space's revision.
        fn checked_in(
            &self,
            roster: &Roster,
            header: &Header,
            content: &Content,
            key: &SigningKey,
        ) -> Result<Received, String> {
            let payload =
                seal_message(header, content, key, &self.hpke.public_key()).expect("sealed");
            let opened = open_message(header, &payload, self.this, &self.hpke).expect("opens");
            let context = self.context(roster, Group::Held(Some(&self.members)));
            check(opened.sender(), signed(opened), header, &context)
        }

        fn checked(&self, header: &Header, content: &Content) -> Result<Received, String> {
            self.checked_in(&self.roster, header, content, &self.bob_key)
        }
    }

    /// A message from an admitted device of a member of the channel, from its account's person
    /// address in the space on this server, with a counter and an A2A message whose `messageId` is
    /// `msg_id`, gives its text parts, each on a line, `from` and the device's name.
    #[test]
    fn message_that_passes_the_checks_gives_its_text() {
        let world = World::new();
        let header = world.header();
        let content = world.content(World::a2a(&header, &["first", "second"]));

        assert_eq!(
            world.checked(&header, &content),
            Ok(Received {
                from: content.from.clone(),
                to: None,
                device: "laptop".to_owned(),
                text: "first\nsecond".to_owned(),
                counter: NonZeroU64::new(1),
            })
        );
    }

    /// A device of an account the revision does not list, or one its member's `excluded` lists,
    /// fails; so does one whose account has no `client` device in the channel's group as this
    /// device holds it: a group without it, or with it under another key, and no group; and any
    /// device in a channel the revision does not have.
    #[test]
    fn device_without_a_client_device_of_its_account_in_the_group_fails() {
        let world = World::new();
        let header = world.header();
        let content = world.content(World::a2a(&header, &["hi"]));
        let payload =
            seal_message(&header, &content, &world.bob_key, &world.hpke.public_key()).unwrap();
        let check_in = |roster: &Roster, group| {
            let opened = open_message(&header, &payload, world.this, &world.hpke).unwrap();
            check(
                opened.sender(),
                signed(opened),
                &header,
                &world.context(roster, group),
            )
        };
        let mut outside = world.roster.body().clone();
        let mut members = outside.members.clone().into_vec();
        members.truncate(1);
        outside.members = NonEmpty::from_vec(members).unwrap();
        let mut barred = world.roster.body().clone();
        let mut members = barred.members.clone().into_vec();
        members[1].excluded = Some(NonEmpty::new(world.bob));
        barred.members = NonEmpty::from_vec(members).unwrap();
        let in_group = Group::Held(Some(&world.members));

        for body in [outside, barred] {
            let roster = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
            assert_eq!(
                check_in(&roster, in_group),
                Err(
                    "revision 1 of the space acme lists no account with such a device with a \
                     valid certificate that it does not bar"
                        .to_owned()
                )
            );
        }
        let rekeyed = MemberDevice {
            signing: SigningKey::generate().public_key(),
            ..world.members[1]
        };
        let ada_only = &world.members[..1];
        let rekeyed = [world.members[0], rekeyed];
        for group in [
            Group::Held(Some(ada_only)),
            Group::Held(Some(&rekeyed)),
            Group::Held(None),
        ] {
            assert_eq!(
                check_in(&world.roster, group),
                Err(
                    "no client device of the sending device's account is in the channel's group \
                     on this device"
                        .to_owned()
                )
            );
        }
        assert!(check_in(&world.roster, in_group).is_ok());

        let mut elsewhere = world.roster.body().clone();
        elsewhere.channels[0].id = ChannelId::generate();
        let roster = skald_crypto::sign_roster(&elsewhere, &SigningKey::generate()).unwrap();
        assert_eq!(
            check_in(&roster, in_group),
            Err("revision 1 of the space acme has no such channel".to_owned())
        );
    }

    /// Under `mls`, the sending device is in the group: one of a `client` device passes, one of
    /// another kind fails.
    #[test]
    fn sender_in_the_group_passes_as_a_client_device() {
        let world = World::new();
        let header = world.header();
        let content = world.content(World::a2a(&header, &["hi"]));
        let context = world.context(&world.roster, Group::Sender);
        let as_client = check(world.bob, |_| Ok(content.clone()), &header, &context);
        assert!(as_client.is_ok(), "{as_client:?}");

        let bob = &world.bob_account;
        let (service, ..) = bob.device(DeviceKind::Service, &world.bob_key, Some(world.bob));
        let id = service.body().device;
        let mut devices = SpaceDevices {
            ids: world.devices.ids.clone(),
            certs: world.devices.certs.clone(),
            revocations: Vec::new(),
        };
        devices.ids.push(id);
        devices.certs.push(service);
        let context = Context {
            devices: &devices,
            ..world.context(&world.roster, Group::Sender)
        };
        assert!(check(world.bob, |_| Ok(content.clone()), &header, &context).is_ok());
        assert_eq!(
            check(id, |_| Ok(content.clone()), &header, &context),
            Err(
                "no client device of the sending device's account is in the channel's group on \
                 this device"
                    .to_owned()
            )
        );
    }

    /// Content signed by a key other than that of the certificate of the device it names fails.
    #[test]
    fn signature_of_another_key_fails() {
        let world = World::new();
        let header = world.header();
        let content = world.content(World::a2a(&header, &["hi"]));
        let aad = header.to_cbor();
        let plaintext = HpkePlaintext {
            sender: world.bob,
            content: content.clone(),
            signature: SigningKey::generate().sign("skald hpke", &[&aad, &content.to_cbor()]),
        };
        let (enc, ct) = HpkeKey::seal(
            &world.hpke.public_key(),
            b"skald hpke",
            &aad,
            &plaintext.to_cbor(),
        )
        .unwrap();
        let payload = HpkePayload { enc, ct }.to_cbor();
        let opened = open_message(&header, &payload, world.this, &world.hpke).expect("opens");
        let context = world.context(&world.roster, Group::Held(Some(&world.members)));

        assert_eq!(
            check(opened.sender(), signed(opened), &header, &context),
            Err("its signature does not verify with the sending device's certificate".to_owned())
        );
    }

    /// `from` fails unless it is the person or an agent of the sending device's account in the
    /// space, on this server; without the server's name, every one fails.
    #[test]
    fn from_of_another_account_space_or_server_fails() {
        let world = World::new();
        let failed = Err(
            "its from is no address of the sending device's account in the space acme".to_owned(),
        );
        for from in [
            "acme/ada/user@skald.example",
            "other/bob/user@skald.example",
            "acme/bob/user@other.example",
        ] {
            let header = world.header();
            let mut content = world.content(World::a2a(&header, &["hi"]));
            content.from = from.parse().unwrap();
            assert_eq!(world.checked(&header, &content), failed, "{from}");
        }
        let header = world.header();
        let mut content = world.content(World::a2a(&header, &["hi"]));
        content.from = "acme/bob/claude@skald.example".parse().unwrap();
        assert!(world.checked(&header, &content).is_ok(), "an agent of bob");

        let payload = seal_message(&header, &content, &world.bob_key, &world.hpke.public_key())
            .expect("sealed");
        let opened = open_message(&header, &payload, world.this, &world.hpke).unwrap();
        let unnamed = Context {
            server: None,
            ..world.context(&world.roster, Group::Held(Some(&world.members)))
        };
        assert_eq!(
            check(opened.sender(), signed(opened), &header, &unnamed),
            failed
        );
    }

    /// Content without `counter`, without an A2A message, or whose A2A message is no JSON or has
    /// another `messageId` than `msg_id`, fails.
    #[test]
    fn content_without_counter_or_a_matching_a2a_message_fails() {
        let world = World::new();
        let header = world.header();
        // Content with a body of a later version, the field 20, in place of the A2A message.
        let later = |counter| {
            let mut bytes = Content {
                counter,
                body: None,
                ..world.content(String::new())
            }
            .to_cbor();
            // One more field in the map's head, 0xa0 and its count.
            bytes[0] += 1;
            bytes.extend_from_slice(&[0x14, 0x01]);
            Content::from_cbor(&bytes).expect("content of a later version")
        };
        let uncounted = later(None);
        let later = later(NonZeroU64::new(1));
        // A receipt comes from an agent, never from a person, to the sender of its messages.
        let mut receipt = world.content(String::new());
        receipt.from = "acme/bob/claude@skald.example".parse().unwrap();
        receipt.to = Some("acme/ada/user@skald.example".parse().unwrap());
        receipt.body = Some(Body::Receipt(Receipt {
            status: ReceiptStatus::Read,
            messages: NonEmpty::new(MessageId::generate()),
            reason: None,
            unknown: Unknown::default(),
        }));
        let other = Header {
            msg_id: MessageId::generate(),
            ..header.clone()
        };
        for (content, why) in [
            (uncounted, "its content has no counter"),
            (later, "it holds no A2A message"),
            (receipt, "it holds no A2A message"),
            (
                world.content("not json".to_owned()),
                "its A2A message is no JSON",
            ),
            (
                world.content(World::a2a(&other, &["hi"])),
                "its A2A messageId is not its msg_id",
            ),
        ] {
            assert_eq!(
                world.checked(&header, &content),
                Err(why.to_owned()),
                "{why}"
            );
        }
    }

    /// A `delivered` of a message of this device.
    fn delivered() -> DeliveryStatus {
        DeliveryStatus {
            msg_id: MessageId::generate(),
            device: DeviceId::from_signing_key(&[2; 32]),
            status: ServerStatus::Delivered,
            timestamp: 1_800_000_000,
            unknown: Unknown::default(),
        }
    }

    /// Of a catch-up, only the `message` and `status` entries after the cursor are written, each
    /// as its frame.
    #[test]
    fn entries_after_the_cursor_are_written_as_their_frames() {
        let seq = |seq| NonZeroU64::new(seq).unwrap();
        let header = Header {
            v: NonZeroU64::MIN,
            space: SpaceId::generate(),
            channel: ChannelId::generate(),
            msg_id: MessageId::generate(),
            scheme: Scheme::Mls,
            recipients: None,
            ttl: None,
            unknown: Unknown::default(),
        };
        let envelope = RawEnvelope::from_bytes(
            Envelope {
                header,
                payload: vec![1],
            }
            .to_cbor(),
        )
        .unwrap();
        let status = delivered();
        let message = |at| Push::Message {
            seq: seq(at),
            envelope: envelope.clone(),
        };
        let entries = vec![
            message(2),
            message(3),
            Push::SpaceNotice {
                space: SpaceId::generate(),
            },
            Push::Status {
                seq: seq(4),
                status: status.clone(),
            },
        ];

        let written = after(entries, 2);

        let frame = |push| Frame::Push(push).to_cbor();
        assert_eq!(
            written,
            [
                (3, frame(message(3))),
                (
                    4,
                    frame(Push::Status {
                        seq: seq(4),
                        status,
                    })
                )
            ]
        );
    }

    /// A `message` or `status` gives one entry, a `batch` its entries, an empty one the end of
    /// catch-up; any other frame, or one that does not decode, nothing.
    #[test]
    fn frames_give_entries_until_an_empty_batch() {
        let notice = || Push::SpaceNotice {
            space: SpaceId::generate(),
        };
        let push = notice();
        let batch = vec![notice(), notice()];

        assert_eq!(
            got(Some(Frame::Push(push.clone()))),
            Got::Entries(vec![push])
        );
        assert_eq!(got(Some(Frame::Batch(batch.clone()))), Got::Entries(batch));
        assert_eq!(got(Some(Frame::Batch(Vec::new()))), Got::End);
        assert_eq!(got(Some(Frame::Sync { cursor: 1 })), Got::Nothing);
        assert_eq!(got(None), Got::Nothing);
    }

    /// Of a catch-up's entries, the `space-notice` ones give their spaces, in order.
    #[test]
    fn notices_give_their_spaces() {
        let (first, second) = (SpaceId::generate(), SpaceId::generate());
        let entries = [
            Push::SpaceNotice { space: first },
            Push::Status {
                seq: NonZeroU64::MIN,
                status: delivered(),
            },
            Push::SpaceNotice { space: second },
        ];

        assert_eq!(noticed(&entries).collect::<Vec<_>>(), [first, second]);
    }

    /// The warning of missing messages names how many, their counters, the sending device and the
    /// channel.
    #[test]
    fn missing_messages_are_named_by_their_counters() {
        let channel: ChannelId = "01a1dcab-ee8a-7872-9adc-6efd0851e3ee".parse().unwrap();
        let sender: DeviceId = "a90f5ea9-75e5-899a-90ec-3a09f33aebdf".parse().unwrap();

        assert_eq!(
            missing(channel, sender, (2, 2)),
            "the message with the counter 2 from the device a90f5ea9-75e5-899a-90ec-3a09f33aebdf \
             in the channel 01a1dcab-ee8a-7872-9adc-6efd0851e3ee is missing: the server refused, \
             expired or dropped it"
        );
        assert_eq!(
            missing(channel, sender, (4, 6)),
            "3 messages with the counters 4 to 6 from the device \
             a90f5ea9-75e5-899a-90ec-3a09f33aebdf in the channel \
             01a1dcab-ee8a-7872-9adc-6efd0851e3ee are missing: the server refused, expired or \
             dropped them"
        );
    }

    /// An agent's warnings name a message by its reference and a device by its name, escaped,
    /// without IDs.
    #[test]
    fn agent_warnings_name_a_message_by_its_reference_and_a_device_by_its_name() {
        assert_eq!(
            dropped("k3vq", Some("bob\u{7}box"), "its A2A message is no JSON"),
            "dropped #k3vq from bob\\u{7}box: its A2A message is no JSON"
        );
        assert_eq!(
            dropped("k3vq", None, "it does not open"),
            "dropped #k3vq: it does not open"
        );
        assert_eq!(
            missing_before("bob-box", (2, 2), "k3vq"),
            "a message from bob-box is missing before #k3vq"
        );
        assert_eq!(
            missing_before("bob-box", (4, 6), "k3vq"),
            "3 messages from bob-box are missing before #k3vq"
        );
    }
}
