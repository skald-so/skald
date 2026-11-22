//! `skald recv`: the device gets its mailbox (`docs/protocol/delivery.md`, "Mailbox"). It first
//! sends what is left in the outbox and the history entries of the messages the server stored
//! ([`crate::history`]), then states its cursor, writes each entry it gets after the cursor to the
//! local state with the new cursor at once, acks, and stops once catch-up has sent every entry. As
//! the person, it then follows the spaces other devices of the account joined ([`crate::joined`]).
//! It then accepts the revisions of each space it follows, and those of the space of each
//! `space-notice` that came meanwhile ("Frames"), following first a noticed space it does not. It
//! then goes through the Welcomes, Commits and `mls` application messages it wrote, decrypting
//! each message in its channel's group, and brings the group of each channel of those spaces in
//! line ([`crate::groups`]), uploading the links of their key chains the server lacks
//! ([`crate::key_chain`]), and repeats the completes and fails of claims that wait, sending their
//! results ([`crate::claims`]). It then shows the other entries it
//! wrote: a `message` only if it passes the checks of `docs/protocol/README.md`, "Envelope",
//! "Scheme `mls`" and "Scheme `hpke`", with the space's latest revisions accepted
//! first, the sending device's certificate valid and the channel's members taken from its group:
//! the sending device under `mls`, the group as this device holds it under `hpke`; from a
//! `service` device, its service agent entry and its grant ("Service grant") in place of the
//! group, the revisions fetched and accepted again once when no entry lists the device; only if
//! [`crate::gaps`] accepts its `counter`, warning first of the counters it makes missing and
//! marking it as following missing ones, or, without `counter`, once per sending device and
//! `msg_id`; a duplicate or a replay without a word; a warning for any other; and each `delivered`
//! and `expired` of the device's messages. As an agent, only those of its folder's space: the rest
//! wait in the local state.
//!
//! A message that passes the checks is kept, with the trust level its sending device's certificate
//! and the space's roster give (`docs/protocol/membership.md`, "Trust levels"), for the person and
//! for the agents it is for (`docs/protocol/delivery.md`, "Device to agent"), among the agents
//! this device had run as in the space when it wrote the entry: with `to`, the one it names;
//! without `to`, each; a message to a person, none; but those whose inbound mode refuses it, which
//! the person's line marks. The person's run shows each kept message once, whichever run checked
//! it. An agent's run shows the messages kept for it and marks each handed
//! over only once its line is written, so a run killed in between shows it again; a message waits
//! for an agent until its envelope's `ttl` ends, at most [`AGENT_WAIT`], after the device wrote it.
//! The device's own channel messages are kept so too, once the server stored them
//! ([`own_message`]).
//!
//! A task result that passes the checks is kept as a message only once the checks of
//! [`crate::results`] pass too; otherwise it is dropped with a warning and counts in gap detection.
//! A message whose content holds a `task` body is dropped.
//!
//! A receipt that passes the checks counts in gap detection and is shown to no one: of each
//! message it lists that this device sent in the receipt's channel, `to` the receipt's `from`, a
//! `read`, or a `refused` with its reason, is kept as a status, once; any other is dropped without
//! a word (`docs/protocol/delivery.md`, "Receipts"). Once an agent's run marks a message to it
//! handed over, the device sends its `read`; once it keeps a message its `to` agent's inbound mode
//! refuses, its `refused` ([`crate::receipts`], [`crate::inbound`]).
//!
//! Each status of the device's messages is kept for the person and, for a message an agent sent,
//! for that agent; a kept reply gives its message `replied` ([`crate::reply`]). The person's run
//! shows each kept status once, and each warning an agent's run held back for it; an agent's run
//! shows those of its messages it has yet to take. A run shows them in the form [`crate::forms`]
//! gives: in full as the person, short as an agent, where a status is shown only if `expired` and a
//! warning in a line of its own without IDs, and as JSON with `--json`.

use std::cell::Cell;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use std::path::Path;
use std::time::Duration;

use skald_crypto::{
    HpkeError, HpkeMessage, MemberDevice, StateKey, check_grant, open_message, trust_level,
};
use skald_proto::{
    AccountId, Address, Body, Cbor as _, Cert, ChannelId, Content, DeviceId, DeviceKind, Envelope,
    Frame, Header, MessageId, Name, Push, Receipt, ReceiptStatus, Roster, RosterBody, ServerName,
    ServerStatus, ServiceAgent, SpaceId, TaskId, TaskResult, TrustLevel, VersionRange,
};

use crate::agents::server_name;
use crate::claims;
use crate::claude_socket;
use crate::connection::printable;
use crate::delivery;
use crate::devices::text;
use crate::follow::{Own, SpaceDevices};
use crate::forms::{self, Form, References};
use crate::gaps::{self, Accepted, Source};
use crate::groups::{self, Link, Opened, Rosters};
use crate::held;
use crate::history;
use crate::hook;
use crate::inbound;
use crate::intake::{self, Intake};
use crate::joined;
use crate::key_chain;
use crate::mailbox::Mailbox;
use crate::message_status;
use crate::output::{self, Stream};
use crate::queues;
use crate::receipts;
use crate::reply;
use crate::results;
use crate::secrets;
use crate::send::{self, Reported};
use crate::spaces::{self, Follower, Scope};
use crate::state::{self, Kept, State, ToRead};
use crate::wraps;

thread_local! {
    /// The space the last run of this thread kept to, if it kept to one: an agent's.
    static SPACE: Cell<Option<SpaceId>> = const { Cell::new(None) };
}

/// The space the last run of this thread kept to, if it kept to one: an agent's.
pub fn last_space() -> Option<SpaceId> {
    SPACE.with(Cell::get)
}

/// How long a message waits for an agent at most: the cloud's mailbox retention
/// (`architecture.md`, "Cloud limits").
pub const AGENT_WAIT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Receives the mailbox of the device of the state in `dir`, its secrets in `storage`, as `agent`
/// if given, whose run holds its warnings for the person ([`holding_warnings`]); waits for each
/// frame for `timeout`; shows as JSON if `json`. Gives whether it showed what `skald recv --wait`
/// waits for: as an agent, a message handed over to it; as the person, anything. For such a run,
/// if `waits`, leaves what takes in what comes next ([`intake::leave`]).
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    agent: Option<&Name>,
    (waits, json): (bool, bool),
) -> Result<bool, String> {
    holding_warnings(dir, agent.is_some(), || {
        let form = Form::of(agent.is_some(), json);
        received(
            dir,
            storage,
            (speaks, timeout),
            agent,
            (waits, form, None),
            true,
        )
    })
}

/// The session an MCP server of `agent` holds of the helper ([`crate::wait`]): [`run`] as the
/// agent's `skald recv --wait`, leaving what takes in what comes next, but handing nothing over,
/// which the agent's `inbox` does.
pub fn hold_run(
    dir: &Path,
    storage: secrets::Storage,
    settings: (VersionRange, Duration),
    agent: &Name,
) -> Result<(), String> {
    holding_warnings(dir, true, || {
        let form = Form::Short;
        received(
            dir,
            storage,
            settings,
            Some(agent),
            (true, form, None),
            false,
        )
        .map(drop)
    })
}

/// The `inbox` of `agent`'s MCP session ([`crate::mcp`]): [`run`] as the agent, in its short form,
/// which writes the lines it hands over in one write and marks them handed over once that write
/// is done, and only if the run did not fail.
pub fn inbox(
    dir: &Path,
    storage: secrets::Storage,
    settings: (VersionRange, Duration),
    agent: &Name,
) -> Result<(), String> {
    holding_warnings(dir, true, || {
        received(
            dir,
            storage,
            settings,
            Some(agent),
            (
                false,
                Form::Short,
                Some(&|follower, ran_as| hand_over_whole(follower, ran_as, Form::Short)),
            ),
            true,
        )
        .map(drop)
    })
}

/// A run of `agent`'s hook ([`crate::hook`]) on `event` of `session`, with Claude Code's cap `cap`
/// of `Stop` continuations in a row: in a folder in no space, nothing; on `Stop`, first what the
/// session file of a registered session confirms ([`claude_socket::confirm`]); at the cap,
/// nothing, leaving every message waiting; otherwise [`run`] as the agent, which hands over what
/// [`hook::compose`] gives, in one write, and only if the run did not fail.
pub fn hook(
    dir: &Path,
    storage: secrets::Storage,
    settings: (VersionRange, Duration),
    agent: &Name,
    (event, cap, session): (hook::Event, u64, &hook::Session),
) -> Result<(), String> {
    holding_warnings(dir, true, || {
        let Ok(space) = crate::folders::working_space(dir) else {
            return Ok(());
        };
        let state = State::open(dir).map_err(text)?;
        state.add_agent(space.id, agent.as_str()).map_err(text)?;
        if let (hook::Event::Stop { .. }, Some(session)) = (event, &session.id) {
            claude_socket::confirm(&state, session).map_err(text)?;
        }
        let stops = match event {
            hook::Event::Stop { continued: true } => {
                state.count_stops(space.id, agent.as_str(), None)
            }
            _ => state.count_stops(space.id, agent.as_str(), Some(0)),
        }
        .map_err(text)?;
        if matches!(event, hook::Event::Stop { .. }) && stops >= cap {
            return Ok(());
        }
        drop(state);
        received(
            dir,
            storage,
            settings,
            Some(agent),
            (
                false,
                Form::Short,
                Some(&|follower, ran_as| hand_over_hook(follower, ran_as, event)),
            ),
            true,
        )
        .map(drop)
    })
}

/// A run of the helper for the Claude Code sessions of `agent` it serves
/// ([`crate::claude_socket`]): [`run`] as the agent, which writes what waits for it into the
/// socket of its last registered session that still runs, and only if the run did not fail;
/// leaves what takes in what comes next. Gives whether the run did not fail; [`due`] then gives
/// when a run next pushes what `batch` holds back.
pub fn to_sessions(
    dir: &Path,
    storage: secrets::Storage,
    settings: (VersionRange, Duration),
    agent: &Name,
) -> Result<bool, String> {
    DUE.with(|due| due.set(None));
    holding_warnings(dir, true, || {
        received(
            dir,
            storage,
            settings,
            Some(agent),
            (true, Form::Short, Some(&hand_over_socket)),
            true,
        )
    })
}

/// Shows the message `message` names, by `#<ref>`, `<ref>` or its `msg_id`, whole, in `form`, as
/// `agent`'s run, by the state in `dir`, its secrets in `storage`: one handed over to the agent in
/// its folder's space, while the device keeps what a reply to it needs ([`crate::reply`]). A short
/// line of a message below `own` follows [`forms::DATA_WARNING`].
pub fn reread(
    dir: &Path,
    storage: secrets::Storage,
    agent: &Name,
    message: &str,
    form: Form,
) -> Result<(), String> {
    holding_warnings(dir, true, || {
        let shown = printable(message.trim());
        let named = reply::given(message)?;
        let (_, reader) = reply::reader(dir, Some(agent))?;
        let target = reply::kept_target(dir, &named, reader).map_err(|why| match why {
            reply::Unnamed::Unknown => format!(
                "{shown} is no message handed over to this agent in its folder's space, or this \
                 device no longer keeps it"
            ),
            reply::Unnamed::Shared => reply::shared(&shown),
            reply::Unnamed::Failed(err) => err,
        })?;
        let sealed = (target.line).ok_or_else(|| format!("this device kept no line of {shown}"))?;
        let (state, _, _, _, secrets) = crate::devices::signed_in_with_secrets(dir, storage)?;
        let this = (state.device().map_err(text)?)
            .ok_or("this machine has no device yet: sign in with skald sign-in")?
            .id;
        let key = crate::key_packages::local_state_key(&state, &secrets)?;
        let opened = (key.open(this, &kept_item(target.seq), &sealed))
            .ok_or("what this device kept of the message does not open")?;
        let references = References::new(state.kept_ids().map_err(text)?);
        let (line, level) = kept_line(&opened, form, &references);
        let line = match form == Form::Short && level < TrustLevel::Own {
            true => format!("{}\n{line}\n", forms::DATA_WARNING),
            false => format!("{line}\n"),
        };
        output::write(Stream::Out, line.as_bytes())
            .map_err(|err| format!("cannot write the message to the output: {err}"))
    })
}

/// Runs `run`, a run as an agent if `agent`; such a run holds back each of its warnings and keeps
/// it in the state in `dir` for the person's run, or, if it cannot, shows it.
pub fn holding_warnings<R>(dir: &Path, agent: bool, run: impl FnOnce() -> R) -> R {
    if !agent {
        return run();
    }
    let (ran, held) = output::holding_warnings(run);
    keep(dir, &held);
    ran
}

/// Keeps `held`, warnings an agent's run held back, in the state in `dir` for the person's run,
/// or, if it cannot, shows them.
pub fn keep(dir: &Path, held: &[String]) {
    if !held.is_empty() {
        keep_held(
            State::open(dir).and_then(|state| state.keep_warnings(held)),
            held,
        );
    }
}

/// Shows `held`, warnings an agent's run held back, unless `kept`, keeping them for the person's
/// run, succeeded.
pub fn keep_held(kept: Result<(), state::Error>, held: &[String]) {
    if let Err(err) = kept {
        for warning in held {
            eprintln!("{warning}");
        }
        eprintln!("skald: warning: the local state keeps no warning for the person: {err}");
    }
}

/// How a run that fails hands nothing over, such as [`inbox`]'s, hands over to an agent in a
/// space: gives how many messages it handed over.
type Whole<'w> = &'w dyn Fn(&Follower, (SpaceId, &Name)) -> Result<usize, String>;

/// [`run`], showing in `form`; handing over with `whole`, if given, as [`inbox`] does; handing
/// nothing over to the agent unless `hands`.
fn received(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    agent: Option<&Name>,
    (waits, form, whole): (bool, Form, Option<Whole>),
    hands: bool,
) -> Result<bool, String> {
    let written = output::written();
    let scope = Scope::of_run(dir, agent.is_some())?;
    SPACE.with(|space| {
        space.set(match scope {
            Scope::Only(only) => Some(only),
            Scope::All => None,
        });
    });
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
    let reported = match (agent, form) {
        (Some(_), _) => Reported::Quiet,
        (None, Form::Json) => Reported::Json(None),
        (None, _) => Reported::Full,
    };
    if let Err(err) = send::report(&follower.state, &queued, &delivered, timeout, reported) {
        eprintln!("skald: warning: {err}");
    }
    // The entries go out on the outbox's connection, which then gets the mailbox.
    let flushed = std::mem::replace(&mut link.mailbox, delivered.mailbox.take());
    history::upload(&mut follower, &mut link, scope);
    queues::upload_received(&mut follower, scope);
    let mailbox = std::mem::replace(&mut link.mailbox, flushed);
    let mut synced = BTreeMap::new();
    let received = receive(&mut follower, mailbox, (timeout, scope), &mut synced);
    let aligned = align(&mut follower, &mut link, scope, &mut synced);
    // Completes and fails that wait, and their results, go out before what the run shows.
    claims::finish_waiting(&mut follower, scope, &synced);
    let mut checks = Checks::new(&follower, synced);
    let shown = match checks.as_mut() {
        Ok(checks) => show(&mut follower, (scope, ran_as.is_some()), checks),
        Err(err) => Err(err.clone()),
    };
    let handed = match (ran_as, whole) {
        (Some(_), _) if !hands => Ok(0),
        // A failed run hands nothing over.
        (Some((space, agent)), Some(whole)) => match (&received, &aligned, &shown) {
            (Ok(()), Ok(()), Ok(())) => whole(&follower, (space, agent)),
            _ => Ok(0),
        },
        (Some((space, agent)), None) => hand_over(&follower, (space, agent), form),
        (None, _) => show_to_person(&follower, form),
    };
    if let Ok(checks) = &checks {
        receipts::send(&mut follower, &checks.synced, scope);
    }
    // What waits for the person's decision is no news a wait ends for.
    let person_shown = output::written() > written;
    if ran_as.is_none()
        && !follower.stateless()
        && let Err(err) = held::say_waiting(&follower.state, form)
    {
        eprintln!("skald: warning: {err}");
    }
    if follower.stateless()
        && received.is_ok()
        && aligned.is_ok()
        && shown.is_ok()
        && handed.is_ok()
    {
        ack_shown(&mut follower)?;
    }
    if waits && let Ok(mut checks) = checks {
        // A failure leaves it to the first message.
        let _ = checks.ask_server(&mut follower);
        // Its connection goes back to be kept.
        link.mailbox = None;
        intake::leave(Intake::new(follower, checks, link));
    }
    received?;
    aligned?;
    shown?;
    let handed = handed?;
    Ok(match ran_as {
        Some(_) => handed > 0,
        None => person_shown,
    })
}

/// Acks what a stateless run wrote, once it has shown it ([`crate::mailbox`]).
fn ack_shown(follower: &mut Follower) -> Result<(), String> {
    let keys = follower.keys()?;
    let mut mailbox = follower.connect(&keys)?;
    mailbox.ack(follower, &keys)
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
    if scope == Scope::All {
        joined::follow(follower);
    }
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
        if scope == Scope::All && joined::names_unfollowed(follower, &due) {
            joined::follow(follower);
        }
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
    catch_up_noting(follower, mailbox, timeout, &mut BTreeSet::new())
}

/// As [`catch_up`], adding to `noticed` the space of each `space-notice` catch-up gives.
pub fn catch_up_noting(
    follower: &mut Follower,
    mailbox: Option<Mailbox>,
    timeout: Duration,
    noticed: &mut BTreeSet<SpaceId>,
) -> Result<Mailbox, String> {
    let keys = follower.keys()?;
    let mailbox = match mailbox {
        Some(mailbox) => mailbox,
        None => follower.connect(&keys)?,
    };
    mailbox.catch_up(follower, &keys, timeout, |follower, entries, cursor| {
        noticed.extend(self::noticed(&entries));
        let written = after(entries, cursor);
        if written.is_empty() {
            return Ok(cursor);
        }
        follower.state.write_entries(&written).map_err(text)
    })
}

/// Syncs into `synced` each space of `due` within `scope` that the device follows, and empties
/// `due`. A space that fails to sync is warned of, and synced again at its first message.
pub fn sync_due(
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
                Push::SpaceNotice { .. } | Push::LinkNotice | Push::Unknown(_) => return None,
            };
            (seq > cursor).then(|| (seq, Frame::Push(push).to_cbor()))
        })
        .collect()
}

/// What a run checks messages with, besides the state and the spaces it follows: those it synced,
/// each one's last accepted revision and its accounts' devices, the server's name, once it asked
/// for it, and the key that opens what decrypting the mailbox's application messages gave, once it
/// needed it.
pub struct Checks {
    keys: crate::device::Keys,
    pub synced: Rosters,
    server: Option<Option<ServerName>>,
    inbox_key: Option<StateKey>,
    /// Each space and device a message from which made the run fetch the space's revisions again,
    /// as no entry listed the device: once each.
    refetched: BTreeSet<(SpaceId, DeviceId)>,
}

impl Checks {
    /// The checks of `follower`'s run, with the spaces it synced, `synced`.
    pub fn new(follower: &Follower, synced: Rosters) -> Result<Self, String> {
        Ok(Checks {
            keys: follower.keys()?,
            synced,
            server: None,
            inbox_key: None,
            refetched: BTreeSet::new(),
        })
    }

    /// Asks `follower`'s server for its name, once, so a later message is checked without asking.
    pub fn ask_server(&mut self, follower: &mut Follower) -> Result<(), String> {
        if self.server.is_none() {
            self.server = Some(server_name(&mut follower.connection)?);
        }
        Ok(())
    }
}

/// Goes through each entry the state holds within `scope`, in `seq` order, and takes it out,
/// keeping each message that passes the checks for the person and the agents it is for, and each
/// status for the person and the agent that sent its message; checks messages against the spaces
/// as `checks` holds them, syncing any other at its first message. Warns of each message it drops
/// and of the messages missing before one; as an agent, if `agent`, also in a short line.
pub fn show(
    follower: &mut Follower,
    (scope, agent): (Scope, bool),
    checks: &mut Checks,
) -> Result<(), String> {
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
                    decrypted(follower, checks, (seq, envelope), opened)?
                } else {
                    verdict(follower, checks, (seq, envelope))?
                };
                let header = &envelope.header;
                match verdict {
                    Verdict::Show(sender, message, counted) => {
                        let gap = counted.as_ref().and_then(|(_, accepted)| accepted.gap);
                        if let Some(gap) = gap {
                            eprintln!("skald: warning: {}", missing(header.channel, sender, gap));
                            // A receipt has no line an agent's warning could name.
                            if agent && message.receipt.is_none() {
                                let reference = reference(follower, header.msg_id)?;
                                eprintln!(
                                    "skald: {}",
                                    missing_before(&message.device, gap, &reference)
                                );
                            }
                        }
                        let counters = (counted.as_ref())
                            .map(|(source, accepted)| (source, &accepted.counters));
                        if let Some(receipt) = &message.receipt {
                            let reads =
                                reads(follower, checks, (header, sender), &message.from, receipt)?;
                            let (held, reads) = &reads;
                            (follower.state)
                                .take_receipt(
                                    seq,
                                    (counters, (sender, header.msg_id)),
                                    (*held, reads),
                                )
                                .map_err(text)?;
                            continue;
                        }
                        // A result its checks drop still counts in gap detection.
                        if let Some(result) = &message.result {
                            let ours = run_agent(follower, checks, header.space);
                            let addresses = (&message.from, message.to.as_ref(), ours.as_ref());
                            let judged =
                                results::judge(follower, (header, sender), addresses, result)?;
                            if let Err(why) = judged {
                                say_dropped(follower, checks, (header, Some(sender)), &why, agent)?;
                                (follower.state)
                                    .take_receipt(
                                        seq,
                                        (counters, (sender, header.msg_id)),
                                        (false, &[]),
                                    )
                                    .map_err(text)?;
                                continue;
                            }
                        }
                        let kept = kept(follower, checks, (seq, header, sender), &message, gap)?;
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
                        say_dropped(follower, checks, (header, sender), &why, agent)?;
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
                    device: Some(status.device.to_string()),
                    device_name: space.and_then(|space| device_name(checks, space, status.device)),
                    problem: None,
                    reason: None,
                    timestamp: status.timestamp,
                };
                let json = serde_json::to_string(&kept).expect("a status encodes");
                let agent = sent.as_ref().and_then(|sent| sent.agent.as_deref());
                (follower.state)
                    .keep_status(seq, (status.msg_id, space, agent), &json)
                    .map_err(text)?;
            }
            // This client wrote only entries it reads.
            _ => follower.state.remove_entry(seq).map_err(text)?,
        }
    }
    Ok(())
}

/// Warns that the message with `header` from the device `sender`, if known, is dropped for `why`;
/// as an agent, if `agent`, also in a short line.
fn say_dropped(
    follower: &Follower,
    checks: &Checks,
    (header, sender): (&Header, Option<DeviceId>),
    why: &str,
    agent: bool,
) -> Result<(), String> {
    let from = sender.map_or_else(String::new, |sender| format!(" from the device {sender}"));
    eprintln!(
        "skald: warning: dropped the message {} in the channel {}{from}: {why}; it is not shown",
        header.msg_id, header.channel
    );
    if agent {
        let reference = reference(follower, header.msg_id)?;
        let name = sender.and_then(|sender| device_name(checks, header.space, sender));
        eprintln!("skald: {}", dropped(&reference, name.as_deref(), why));
    }
    Ok(())
}

/// The address of the agent of a stateless run of a service agent in `space`, as the space's last
/// accepted revision in `checks` lists its device; none for any other run.
fn run_agent(follower: &Follower, checks: &Checks, space: SpaceId) -> Option<Address> {
    let server = checks.server.as_ref().and_then(Option::as_ref)?;
    let (roster, _) = checks.synced.get(&space)?;
    (follower.stateless())
        .then(|| claims::service_address(roster, follower.this, server))
        .flatten()
}

/// The `read`, `refused` or `held` statuses, as JSON by `msg_id`, that `receipt`, from `from` in
/// the envelope with `header` from the device `sender`, gives of messages this device sent, and
/// whether they are `held`: of a message it sent in the receipt's channel whose `to` is `from`,
/// `refused` with the receipt's reason, if any. It ignores any other, as every device but its
/// messages' sender ignores a receipt (`docs/protocol/delivery.md`, "Receipts").
fn reads(
    follower: &Follower,
    checks: &Checks,
    (header, sender): (&Header, DeviceId),
    from: &Address,
    receipt: &Receipt,
) -> Result<(bool, Vec<(MessageId, String)>), String> {
    let from = from.to_string();
    let mut reads = Vec::new();
    for &msg_id in receipt.messages.iter() {
        let Some(sent) = follower.state.sent(msg_id).map_err(text)? else {
            continue;
        };
        if sent.channel != header.channel || sent.addressee.as_deref() != Some(from.as_str()) {
            continue;
        }
        let name = device_name(checks, sent.space, sender);
        let at = (sent.space, header.channel);
        let json = match receipt.status {
            ReceiptStatus::Refused => {
                let reason = receipt.reason.as_deref();
                forms::Status::refused_by_device(msg_id, at, (sender, name), reason)
            }
            ReceiptStatus::Held => {
                forms::Status::from_device(forms::HELD, msg_id, at, (sender, name))
            }
            ReceiptStatus::Read => {
                forms::Status::from_device(forms::READ, msg_id, at, (sender, name))
            }
        };
        reads.push((msg_id, json));
    }
    Ok((receipt.status == ReceiptStatus::Held, reads))
}

/// How the message of the entry `seq` with `header` from the device `sender`, which passed the
/// checks, is kept: its line, sealed, and the agents it is for whose inbound mode takes it, with
/// those that refuse it marked in its line, and those that hold it for the person's decision;
/// `gap`, the counters it makes missing, if any. A message whose `to` agent refuses it owes its
/// `refused` at once, and one whose `to` agent holds it its `held` ([`inbound`]).
fn kept(
    follower: &Follower,
    checks: &mut Checks,
    (seq, header, sender): (u64, &Header, DeviceId),
    message: &Received,
    gap: Option<(u64, u64)>,
) -> Result<Kept, String> {
    let mut agents = agents_of(follower, checks, (seq, header), message.to.as_ref())?;
    let (refusing, held) =
        inbound::refusing_and_holding(&follower.state, header.space, &agents, &message.from)?;
    // With `to`, the agents are none or the one it names.
    let read = match (&message.to, agents.first()) {
        (Some(to), Some(agent)) => Some(ToRead {
            channel: header.channel,
            sender,
            author: message.from.to_string(),
            addressee: to.to_string(),
            agent: agent.clone(),
            reason: refusing.contains(agent).then(|| inbound::REASON.to_owned()),
            held: held.contains(agent),
        }),
        _ => None,
    };
    agents.retain(|agent| !refusing.contains(agent) && !held.contains(agent));
    let record = forms::Message {
        msg_id: header.msg_id.to_string(),
        space: header.space.to_string(),
        channel: header.channel.to_string(),
        from: message.from.to_string(),
        device: sender.to_string(),
        device_name: message.device.clone(),
        level: message.level,
        text: message.text.clone(),
        after_missing: gap.is_some(),
        reply_to: message.reply_to.map(|reply_to| reply_to.to_string()),
        refused: refusing,
        result: message.result.as_ref().map(results::result_of),
    };
    let record = serde_json::to_vec(&record).expect("a message encodes");
    let at = (header.space, header.channel);
    let reply = (at, sender, &message.from, message.device.as_str());
    let replied = reply::replied(&follower.state, reply, message.reply_to)?;
    let key = match &checks.inbox_key {
        Some(key) => key,
        None => checks.inbox_key.insert(groups::inbox_key(follower)?),
    };
    Ok(Kept {
        space: header.space,
        channel: header.channel,
        msg_id: header.msg_id,
        line: key.seal(follower.this, &kept_item(seq), &record),
        target: reply::seal_target(
            key,
            follower.this,
            seq,
            (&message.from, message.hops),
            message_status::origin(header.msg_id, (sender, &message.device), message.level),
        ),
        wait: agent_wait(header.ttl),
        hops: message.hops,
        agents,
        held,
        read,
        replied,
    })
}

/// How the device of `follower` keeps its own message `content` of `msg_id` in `channel` of
/// `space`, from `agent` if given, by `seq`, once the server stores it, sealed with `key`, as a
/// recipient device keeps it (`docs/protocol/delivery.md`, "Device to agent"): at the level `own`,
/// for its person unless the person sent it and for the agents of this device it is for but its
/// sender and those whose inbound mode refuses it, marked in its line, or holds it, for the
/// person's decision, with the `replied` it gives a message the device sent and its own `read`, or
/// `refused` if its `to` agent refuses it, or `held` and then `read` or `refused` if that agent
/// holds it. Gives it, and whether anything on the device gets it: the person, or an agent the
/// device has run as in the space that takes or holds it.
pub fn own_message(
    follower: &Follower,
    key: &StateKey,
    ((space, channel, msg_id), seq): ((SpaceId, ChannelId, MessageId), u64),
    (content, agent): (&Content, Option<&str>),
) -> Result<(state::OwnMessage, bool), String> {
    let name = (follower.state.device().map_err(text)?)
        .map(|device| device.name)
        .unwrap_or_default();
    let from = &content.from;
    // Only an agent's address of this account in the space on this server, as `from` is, names an
    // agent of this device.
    let to = content.to.as_ref().map(|to| {
        let ours = !to.is_person()
            && to.space() == from.space()
            && to.owner() == from.owner()
            && to.server() == from.server();
        ours.then(|| to.agent().to_string())
    });
    // Every agent it has run as in the space that it is for: none ran as one after `seq`, past the
    // cursor.
    let mut agents = (follower.state.agents(space, seq)).map_err(text)?;
    agents.retain(|it| {
        Some(it.as_str()) != agent && (to.as_ref()).is_none_or(|to| to.as_deref() == Some(it))
    });
    let (refusing, holding) = inbound::refusing_and_holding(&follower.state, space, &agents, from)?;
    let gets = agent.is_some() || agents.iter().any(|it| !refusing.contains(it));
    let device = (follower.this, Some(name.clone()));
    let (read, refused) = match &to {
        Some(Some(to)) if refusing.contains(to) => {
            let reason = Some(inbound::REASON);
            let refused =
                forms::Status::refused_by_device(msg_id, (space, channel), device.clone(), reason);
            (None, Some(refused))
        }
        Some(Some(_)) => {
            let read =
                forms::Status::from_device(forms::READ, msg_id, (space, channel), device.clone());
            (Some(read), None)
        }
        _ => (None, None),
    };
    let held = match &to {
        Some(Some(to)) if holding.contains(to) => Some(forms::Status::from_device(
            forms::HELD,
            msg_id,
            (space, channel),
            device,
        )),
        _ => None,
    };
    // The device's own result's text is its response's.
    let result = (content.task_result())
        .transpose()
        .map_err(|err| format!("its task result breaks the protocol: {err}"))?;
    let text = match &result {
        Some(result) => message_text(&result.response, msg_id)?,
        None => a2a_text(content, msg_id)?,
    };
    let record = forms::Message {
        msg_id: msg_id.to_string(),
        space: space.to_string(),
        channel: channel.to_string(),
        from: from.to_string(),
        device: follower.this.to_string(),
        device_name: name.clone(),
        level: TrustLevel::Own,
        text,
        after_missing: false,
        reply_to: content.reply_to.map(|reply_to| reply_to.to_string()),
        refused: refusing.clone(),
        result: result.as_ref().map(results::result_of),
    };
    let record = serde_json::to_vec(&record).expect("a message encodes");
    let reply = ((space, channel), follower.this, from, name.as_str());
    let replied = reply::replied(&follower.state, reply, content.reply_to)?;
    let own = state::OwnMessage {
        seq,
        kept: Kept {
            space,
            channel,
            msg_id,
            line: key.seal(follower.this, &kept_item(seq), &record),
            target: reply::seal_target(
                key,
                follower.this,
                seq,
                (from, content.hops),
                message_status::origin(msg_id, (follower.this, &name), TrustLevel::Own),
            ),
            wait: agent_wait(None),
            hops: content.hops,
            agents: Vec::new(),
            held: Vec::new(),
            replied,
            read: None,
        },
        person: agent.is_some(),
        sender: agent.map(str::to_owned),
        to,
        read,
        refusing,
        refused,
        holding,
        held,
    };
    Ok((own, gets))
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
pub fn kept_item(seq: u64) -> Vec<u8> {
    [b"kept:".as_slice(), &seq.to_be_bytes()].concat()
}

/// Shows each warning an agent's run kept for the person, each status and then each kept message
/// the person's run has yet to show, in `form`, in order, and marks each shown; gives how many
/// lines it showed.
pub fn show_to_person(follower: &Follower, form: Form) -> Result<usize, String> {
    let state = &follower.state;
    let warnings = state.warnings().map_err(text)?;
    let mut shown = warnings.len();
    for (position, warning) in warnings {
        output::write(Stream::Err, format!("{warning}\n").as_bytes())
            .map_err(|err| format!("cannot write a warning to the output: {err}"))?;
        state.warning_shown(position).map_err(text)?;
    }
    let references = References::new(state.kept_ids().map_err(text)?);
    let statuses = state.statuses_for_person().map_err(text)?;
    shown += shown_statuses(
        statuses,
        (form, &references),
        (&mut written("a status"), &mut |seq| {
            state.status_shown_to_person(seq)
        }),
    )?;
    let kept = state.for_person().map_err(text)?;
    Ok(shown
        + shown_kept(
            follower,
            kept,
            (form, &references),
            (&mut written("a message"), &mut |seq| {
                state.shown_to_person(seq, follower.service.is_some())
            }),
        )?)
}

/// Hands each kept status of a message `agent` sent in `space`, then each kept message that waits
/// for it there, over to it, in `form`, in `seq` order: shows it, then marks it handed over. A
/// message whose wait ended is dropped first. Gives how many messages it handed over.
pub fn hand_over(
    follower: &Follower,
    (space, agent): (SpaceId, &Name),
    form: Form,
) -> Result<usize, String> {
    let state = &follower.state;
    handing_over(
        follower,
        (space, agent),
        form,
        |what, line| written(what)(line),
        (&mut |seq| state.status_taken_by_agent(seq), &mut |seq| {
            state.handed_over(seq, agent.as_str())
        }),
    )
}

/// As [`hand_over`], gathering the lines and writing them in one write, after which it marks
/// each status and message handed over; a failed write marks none.
fn hand_over_whole(
    follower: &Follower,
    (space, agent): (SpaceId, &Name),
    form: Form,
) -> Result<usize, String> {
    let state = &follower.state;
    let mut lines = String::new();
    let (mut statuses, mut messages) = (Vec::new(), Vec::new());
    let shown = handing_over(
        follower,
        (space, agent),
        form,
        |_, line| {
            lines.push_str(&line);
            Ok(())
        },
        (
            &mut |seq| {
                statuses.push(seq);
                Ok(())
            },
            &mut |seq| {
                messages.push(seq);
                Ok(())
            },
        ),
    )?;
    if !lines.is_empty() {
        output::write(Stream::Out, lines.as_bytes())
            .map_err(|err| format!("cannot write the messages to the output: {err}"))?;
    }
    for seq in statuses {
        state.status_taken_by_agent(seq).map_err(text)?;
    }
    for seq in messages {
        state.handed_over(seq, agent.as_str()).map_err(text)?;
    }
    Ok(shown)
}

/// Hands over to `agent` in `space` what its hook gives on `event` ([`hook::compose`]) of the
/// kept statuses of its messages and the kept messages that wait for it that a push path pushes
/// now ([`pushed`]), in one write, after which it marks each status it gave taken, each message it
/// gave handed over and each signal it gave given, and counts one wake for each channel whose
/// messages or signal it gave; a failed write marks none. A message that does not open is warned
/// of and handed over. Gives how many messages it handed over.
fn hand_over_hook(
    follower: &Follower,
    (space, agent): (SpaceId, &Name),
    event: hook::Event,
) -> Result<usize, String> {
    let state = &follower.state;
    let references = References::new(state.kept_ids().map_err(text)?);
    let mut waiting = Vec::new();
    for (position, json) in (state.statuses_for_agent(space, agent.as_str())).map_err(text)? {
        let line = status_line(&json, Form::Short, &references);
        waiting.push(hook::Waiting::Status { position, line });
    }
    let kept = (state.for_agent(space, agent.as_str(), state::unix_now())).map_err(text)?;
    let mut unopened = Vec::new();
    let mut channels = BTreeMap::new();
    if !kept.is_empty() {
        let signalled = state.signalled(agent.as_str()).map_err(text)?;
        let pushing = pushed(follower, space, kept)?;
        for _ in &pushing.unopened {
            eprintln!(
                "skald: warning: a message this device kept does not open, so it is not shown"
            );
        }
        unopened = pushing.unopened;
        for message in pushing.messages {
            let (line, _) = kept_line(&message.opened, Form::Short, &references);
            channels.insert(message.seq, message.channel.clone());
            let reference = message.msg_id.map(|msg_id| references.of(msg_id));
            waiting.push(match (message.goes, reference) {
                (Goes::Own, Some(reference)) => hook::Waiting::Own {
                    seq: message.seq,
                    line,
                    reference,
                },
                (Goes::Inline, Some(reference)) => hook::Waiting::Inline {
                    seq: message.seq,
                    line,
                    reference,
                },
                _ => hook::Waiting::Below {
                    seq: message.seq,
                    signalled: signalled.contains(&message.seq),
                },
            });
        }
    }
    let given = hook::compose(event, &waiting);
    if let Some(given) = &given {
        output::write(Stream::Out, given.output.as_bytes())
            .map_err(|err| format!("cannot write the messages to the output: {err}"))?;
        for position in &given.statuses {
            state.status_taken_by_agent(*position).map_err(text)?;
        }
        for seq in &given.signalled {
            state.signal_given(*seq, agent.as_str()).map_err(text)?;
        }
        let woken: BTreeSet<String> = (given.handed.iter().chain(&given.signalled))
            .filter_map(|seq| channels.get(seq).cloned().flatten())
            .collect();
        state.count_wakes(&woken).map_err(text)?;
    }
    let handed = given.map(|given| given.handed).unwrap_or_default();
    for seq in handed.iter().chain(&unopened) {
        state.handed_over(*seq, agent.as_str()).map_err(text)?;
    }
    Ok(handed.len())
}

thread_local! {
    /// When the first window of `batch` that holds back a message for the last socket run of this
    /// thread ends, in Unix milliseconds, if one does.
    static DUE: Cell<Option<u64>> = const { Cell::new(None) };
}

/// When the first window of `batch` that held back a message for the last run of
/// [`to_sessions`] on this thread ends, in Unix milliseconds, if one did: a run then pushes it.
pub fn due() -> Option<u64> {
    DUE.with(Cell::get)
}

/// How a kept message goes into a session by a push path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Goes {
    /// As its line, labelled as a request from the person's own agent: `own`.
    Own,
    /// As its line, labelled as data from its sender, after the warning line: `inline`.
    Inline,
    /// Only as the signal.
    Signal,
}

/// A kept message a push path pushes now, opened.
struct Pushed {
    seq: u64,
    opened: zeroize::Zeroizing<Vec<u8>>,
    channel: Option<String>,
    msg_id: Option<MessageId>,
    goes: Goes,
}

/// What a push path takes of the kept messages that wait for an agent.
struct Pushing {
    /// Those it pushes now, by `seq`.
    messages: Vec<Pushed>,
    /// Those that do not open, by `seq`.
    unopened: Vec<u64>,
    /// When the first window of `batch` that holds one back ends, in Unix milliseconds.
    due: Option<u64>,
}

/// Of `kept`, kept messages in `space` by `seq` with their sealed lines, those a push path pushes
/// now, as the person's delivery settings say ([`delivery::Settings::plan`]), each with how it
/// goes; and those that do not open.
fn pushed(
    follower: &Follower,
    space: SpaceId,
    kept: Vec<(u64, Vec<u8>)>,
) -> Result<Pushing, String> {
    let state = &follower.state;
    let key = groups::inbox_key(follower)?;
    let settings = delivery::Settings::load(state)?;
    let came = state.kept_came(space).map_err(text)?;
    let (mut opened, mut unopened) = (Vec::new(), Vec::new());
    for (seq, sealed) in kept {
        let Some(open) = key.open(follower.this, &kept_item(seq), &sealed) else {
            unopened.push(seq);
            continue;
        };
        let record = serde_json::from_slice::<forms::Message>(&open).ok();
        let msg_id = (record.as_ref()).and_then(|record| record.msg_id.parse::<MessageId>().ok());
        let level = (record.as_ref()).map_or(TrustLevel::Peer, |record| record.level);
        let waiting = delivery::Waiting {
            seq,
            channel: record.map(|record| record.channel),
            level,
            came: came.get(&seq).copied().unwrap_or(0),
        };
        opened.push((waiting, open, msg_id));
    }
    let waiting: Vec<delivery::Waiting> =
        opened.iter().map(|(waiting, ..)| waiting.clone()).collect();
    let plan = settings.plan(&waiting, state::unix_millis());
    let messages = (opened.into_iter())
        .filter(|(waiting, ..)| plan.goes.contains(&waiting.seq))
        .map(|(waiting, opened, msg_id)| {
            let goes = match settings.as_text(waiting.channel.as_deref(), waiting.level) {
                true if waiting.level >= TrustLevel::Own => Goes::Own,
                true => Goes::Inline,
                false => Goes::Signal,
            };
            Pushed {
                seq: waiting.seq,
                opened,
                channel: waiting.channel,
                msg_id,
                goes,
            }
        })
        .collect();
    Ok(Pushing {
        messages,
        unopened,
        due: plan.due,
    })
}

/// Writes into the socket of the last registered Claude Code session of `agent` in `space` that
/// still runs ([`claude_socket::runs`]; a newer one that does not is no longer registered),
/// with [`claude_socket::write`], each kept message that waits for it that no socket took and a
/// push path pushes now ([`pushed`]): one at `own`, or under `inline`, as its line, the latter
/// after the warning line, within the limit ([`hook::fitted`]); the others whose signal did not
/// go, as one signal ([`hook::SIGNAL`]). Each is recorded as the socket's before the write, and as
/// before it if the write fails, which ends the registration; a write counts one wake for each
/// channel whose messages or signal it carries, unless the session says none of its lines reached
/// the model ([`State::socket_wakes`]). Hands nothing over: the session file confirms what
/// the socket took ([`claude_socket`]). Gives 0.
fn hand_over_socket(
    follower: &Follower,
    (space, agent): (SpaceId, &Name),
) -> Result<usize, String> {
    let state = &follower.state;
    let sessions = state.claude_sessions().map_err(text)?;
    let mut theirs = (sessions.iter().rev())
        .filter(|session| session.space == space && session.agent == agent.as_str());
    let session = loop {
        let Some(session) = theirs.next() else {
            return Ok(0);
        };
        if claude_socket::runs(&session.socket) {
            break session;
        }
        state.end_claude(&session.session).map_err(text)?;
    };
    let kept = (state.for_agent(space, agent.as_str(), state::unix_now())).map_err(text)?;
    let written: BTreeSet<u64> = state
        .written(agent.as_str())
        .map_err(text)?
        .into_iter()
        .collect();
    let signalled: BTreeSet<u64> = (state.signalled(agent.as_str()).map_err(text)?)
        .into_iter()
        .collect();
    let kept: Vec<_> = (kept.into_iter())
        .filter(|(seq, _)| !written.contains(seq) && !signalled.contains(seq))
        .collect();
    if kept.is_empty() {
        return Ok(0);
    }
    let references = References::new(state.kept_ids().map_err(text)?);
    // One that does not open waits for the hooks, which warn of it.
    let pushing = pushed(follower, space, kept)?;
    DUE.with(|due| due.set(pushing.due));
    let (mut lines, mut taken, mut below) = (Vec::new(), Vec::new(), Vec::new());
    let mut channels = BTreeSet::new();
    for message in pushing.messages {
        let (line, _) = kept_line(&message.opened, Form::Short, &references);
        match (message.goes, message.msg_id) {
            (Goes::Own | Goes::Inline, Some(msg_id)) => {
                let line = match message.goes {
                    Goes::Inline => format!("{}\n{line}", forms::DATA_WARNING),
                    _ => line,
                };
                lines.push((
                    msg_id.to_string(),
                    hook::fitted(&line, &references.of(msg_id)),
                ));
                taken.push(message.seq);
            }
            _ => below.push(message.seq),
        }
        channels.extend(message.channel);
    }
    let signal = MessageId::generate().to_string();
    if !below.is_empty() {
        lines.push((signal.clone(), hook::SIGNAL.to_owned()));
    }
    if lines.is_empty() {
        return Ok(0);
    }
    let session_agent = (&session.session[..], agent.as_str());
    (state.socket_takes(session_agent, &taken, (&below, &signal))).map_err(text)?;
    if claude_socket::write(session, &lines).is_err() {
        let seqs: Vec<u64> = taken.into_iter().chain(below).collect();
        state.socket_failed(&seqs, agent.as_str()).map_err(text)?;
        state.end_claude(&session.session).map_err(text)?;
        return Ok(0);
    }
    let ids: Vec<String> = lines.into_iter().map(|(id, _)| id).collect();
    (state.socket_wakes(&session.session, &ids, &channels)).map_err(text)?;
    Ok(0)
}

/// Puts each kept status of a message `agent` sent in `space`, then each kept message that waits
/// for it there, in `form`, in `seq` order, with `put`, which takes what it is, such as a status,
/// and its line; then marks it with `status` or `message`. Gives how many messages it put.
fn handing_over(
    follower: &Follower,
    (space, agent): (SpaceId, &Name),
    form: Form,
    mut put: impl FnMut(&'static str, String) -> Result<(), String>,
    (status, message): (&mut Mark, &mut Mark),
) -> Result<usize, String> {
    let state = &follower.state;
    let references = References::new(state.kept_ids().map_err(text)?);
    let statuses = (state.statuses_for_agent(space, agent.as_str())).map_err(text)?;
    shown_statuses(
        statuses,
        (form, &references),
        (&mut |line| put("a status", line), status),
    )?;
    let kept = (state.for_agent(space, agent.as_str(), state::unix_now())).map_err(text)?;
    shown_kept(
        follower,
        kept,
        (form, &references),
        (&mut |line| put("a message", line), message),
    )
}

/// Writes each line it is given, with its end, to the output: `what`, such as a message.
fn written(what: &'static str) -> impl FnMut(String) -> Result<(), String> {
    move |line| {
        output::write(Stream::Out, line.as_bytes())
            .map_err(|err| format!("cannot write {what} to the output: {err}"))
    }
}

/// What puts each line shown, with its end.
type Put<'p> = dyn FnMut(String) -> Result<(), String> + 'p;

/// What marks a status or message shown, by its `seq`.
type Mark<'m> = dyn FnMut(u64) -> Result<(), state::Error> + 'm;

/// Puts each of `kept`, kept statuses by `seq` as JSON, in `form` with `references`, with `put`,
/// unless the form leaves it out, then marks its `seq` with `mark`; one that does
/// not decode is warned of and marked. Gives how many it put.
fn shown_statuses(
    kept: Vec<(u64, String)>,
    (form, references): (Form, &References),
    (put, mark): (&mut Put, &mut Mark),
) -> Result<usize, String> {
    let mut shown = 0;
    for (seq, json) in kept {
        if let Some(line) = status_line(&json, form, references) {
            put(format!("{line}\n"))?;
            shown += 1;
        }
        mark(seq).map_err(text)?;
    }
    Ok(shown)
}

/// The line of a kept status, `json`, in `form` with `references`, unless the form leaves it out;
/// one that does not decode is warned of and has none.
fn status_line(json: &str, form: Form, references: &References) -> Option<String> {
    let status = serde_json::from_str::<forms::Status>(json).ok();
    let msg_id = (status.as_ref()).and_then(|status| status.msg_id.parse().ok());
    match (status, msg_id) {
        (Some(status), Some(msg_id)) => status.shown(form, &references.of(msg_id)),
        _ => {
            eprintln!(
                "skald: warning: a status this device kept does not decode, so it is not shown"
            );
            None
        }
    }
}

/// Puts each of `kept`, kept messages by `seq` with their sealed lines, in `form` with
/// `references`, with `put`, then marks its `seq` with `mark`; one that does not open
/// is warned of and marked. A line an older client kept is put as it is. In the short form,
/// [`forms::DATA_WARNING`] comes before the first line below `own`. Gives how many it put.
fn shown_kept(
    follower: &Follower,
    kept: Vec<(u64, Vec<u8>)>,
    (form, references): (Form, &References),
    (put, mark): (&mut Put, &mut Mark),
) -> Result<usize, String> {
    if kept.is_empty() {
        return Ok(0);
    }
    let key = groups::inbox_key(follower)?;
    let mut shown = 0;
    let mut warned = false;
    for (seq, sealed) in kept {
        match key.open(follower.this, &kept_item(seq), &sealed) {
            Some(opened) => {
                let (line, level) = kept_line(&opened, form, references);
                let warns = form == Form::Short && !warned && level < TrustLevel::Own;
                let line = if warns {
                    warned = true;
                    format!("{}\n{line}", forms::DATA_WARNING)
                } else {
                    line
                };
                put(format!("{line}\n"))?;
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

/// The line of a kept message, `opened` its record, in `form` with `references`, and its sending
/// device's trust level; one an older client kept is its line as it is, at the level `peer`.
fn kept_line(opened: &[u8], form: Form, references: &References) -> (String, TrustLevel) {
    let record = serde_json::from_slice::<forms::Message>(opened).ok();
    let msg_id = (record.as_ref()).and_then(|record| record.msg_id.parse::<MessageId>().ok());
    match (record, msg_id) {
        (Some(record), Some(msg_id)) => {
            let answered = (record.reply_to.as_deref())
                .and_then(|reply_to| reply_to.parse::<MessageId>().ok())
                .map(|reply_to| references.of(reply_to));
            let shown_id = match &record.result {
                Some(result) => (result.task.parse::<TaskId>())
                    .map_or(msg_id, |task| MessageId::from_bytes(*task.as_bytes())),
                None => msg_id,
            };
            let line = record.shown(form, &references.of(shown_id), answered.as_deref());
            (line, record.level)
        }
        _ => (
            String::from_utf8_lossy(opened).into_owned(),
            TrustLevel::Peer,
        ),
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
    let held = match groups::members(follower, &checks.keys, header.channel)? {
        // A `service` device outside the channel's group takes its members from the latest
        // GroupInfo, as a recipient outside the group does ("Envelope").
        None if follower.service.is_some() => group_info_members(follower, header)?,
        held => held,
    };
    judge(
        follower,
        checks,
        (seq, header, sender, Group::Held(held.as_deref())),
        signed(opened),
    )
}

/// The member devices of the latest GroupInfo the server gives of the channel of `header`, once its
/// signature verifies with a member leaf's key; none without one that does. Fails if the server does
/// not give it, so the run acks nothing it has not judged.
fn group_info_members(
    follower: &mut Follower,
    header: &Header,
) -> Result<Option<Vec<MemberDevice>>, String> {
    let (space, channel) = (header.space, header.channel);
    let (_, group_info) = groups::group_on_server(follower, space, channel)?;
    Ok(group_info
        .and_then(|bytes| groups::observe(channel, &bytes).ok())
        .map(|view| view.members().to_vec()))
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
    let followed = follower.state.spaces().map_err(text)?;
    let Some(space) = followed.iter().find(|space| space.id == header.space) else {
        return Ok(Verdict::Drop(
            Some(sender),
            format!("its space {} is none this device follows", header.space),
        ));
    };
    if let Entry::Vacant(vacant) = checks.synced.entry(space.id) {
        vacant.insert(follower.sync(space)?);
    } else if !listed(&follower.own, &checks.synced[&space.id], sender)
        && checks.refetched.insert((space.id, sender))
    {
        // Before it refuses a device no entry lists, as "Admitted devices" says: so a new service
        // agent's first messages are taken.
        match follower.sync(space) {
            Ok(done) => {
                checks.synced.insert(space.id, done);
            }
            Err(err) => eprintln!("skald: warning: {err}"),
        }
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
    // Content without `counter`, or in a stateless run, which keeps no counters: once per sending
    // device and `msg_id`.
    let counter = message.counter.filter(|_| !follower.stateless());
    let Some(counter) = counter else {
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
    /// The sending device's trust level.
    level: TrustLevel,
    text: String,
    counter: Option<NonZeroU64>,
    /// Its content's `hops` and `reply_to`.
    hops: u64,
    reply_to: Option<MessageId>,
    /// Its receipt, if it is one: it holds no text.
    receipt: Option<Receipt>,
    /// Its task result, if it is one: its text is the response's.
    result: Option<TaskResult>,
}

/// The valid certificate of `device` of an account of the space in `context`'s revision that its
/// member's `excluded` does not list, checked against the master key the device pinned for its own
/// account and the revision's for any other.
fn valid_cert<'c>(context: &Context<'c>, device: DeviceId) -> Option<&'c Cert> {
    member_cert(context.own, (context.roster, context.devices), device)
}

/// [`valid_cert`] of `device` in `roster`, with the space's devices `devices` and the device's own
/// account and pinned master key `own`.
pub fn member_cert<'c>(
    own: &Own,
    (roster, devices): (&'c Roster, &'c SpaceDevices),
    device: DeviceId,
) -> Option<&'c Cert> {
    roster.body().members.iter().find_map(|member| {
        let certs = devices.of(roster, own, member);
        certs.into_iter().find(|cert| cert.body().device == device)
    })
}

/// Whether `synced`, a space's last accepted revision and its devices, lists `device`: with a
/// valid certificate on a member's account, and, for a `service` device, in a service agent entry.
fn listed(own: &Own, (roster, devices): &(Roster, SpaceDevices), device: DeviceId) -> bool {
    member_cert(own, (roster, devices), device).is_some_and(|cert| {
        cert.body().kind != DeviceKind::Service || service_entry(roster, device).is_some()
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
    let level = trust_level(context.own.account, cert, body).ok_or_else(|| {
        format!("its sending device has no trust level in revision {revision} of the space {name}")
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
    if cert.body().kind == DeviceKind::Service {
        check_service(sender, &content, header, context)?;
    } else {
        if content.grant.is_some() {
            return Err(
                "its content carries a grant, which only a service device's may".to_owned(),
            );
        }
        if !of_account(&content.from, body, cert.body().account, context.server) {
            return Err(format!(
                "its from is no address of the sending device's account in the space {name}"
            ));
        }
        if !in_group(context, cert) {
            return Err(
                "no client device of the sending device's account is in the channel's group on \
                 this device"
                    .to_owned(),
            );
        }
        if content.counter.is_none() {
            return Err("its content has no counter".to_owned());
        }
    }
    if content.task().is_some() {
        return Err("its content holds a task body, which only a task may".to_owned());
    }
    // A receipt holds no A2A message: check 4 is the A2A message's, a result's its response's.
    let result = (content.task_result())
        .transpose()
        .map_err(|err| format!("its task result breaks the protocol: {err}"))?;
    let text = match (&content.body, &result) {
        (Some(Body::Receipt(_)), None) => String::new(),
        (_, Some(result)) => message_text(&result.response, header.msg_id)?,
        (_, None) => a2a_text(&content, header.msg_id)?,
    };
    let receipt = match content.body {
        Some(Body::Receipt(receipt)) => Some(receipt),
        _ => None,
    };
    Ok(Received {
        from: content.from,
        to: content.to,
        device: cert.body().name.clone(),
        level,
        text,
        counter: content.counter,
        hops: content.hops,
        reply_to: content.reply_to,
        receipt,
        result,
    })
}

/// Checks 2 and 3 of "Envelope" and the `counter` of `content` from `sender`, a `service` device
/// with a valid certificate on a member's account, in an envelope with `header`, against `context`:
/// `from` is the device's entry in the revision, its grant passes the checks of "Service grant",
/// and content without `counter` comes only under `hpke`.
fn check_service(
    sender: DeviceId,
    content: &Content,
    header: &Header,
    context: &Context,
) -> Result<(), String> {
    let body = context.roster.body();
    let (name, revision) = (body.name.as_str(), body.revision);
    let entry = service_entry(context.roster, sender).ok_or_else(|| {
        format!("revision {revision} of the space {name} lists the service device as no agent")
    })?;
    let listed = context.server.map(|server| {
        Address::new(
            body.name.clone(),
            entry.owner.clone(),
            entry.agent.clone(),
            server.clone(),
        )
    });
    if listed.as_ref() != Some(&content.from) {
        return Err(format!(
            "its from is not the service device's agent in revision {revision} of the space {name}"
        ));
    }
    let grant = (content.grant.as_ref())
        .ok_or_else(|| "its content from a service device carries no grant".to_owned())?;
    let own = (context.own.account, &context.own.master);
    check_grant(
        grant,
        (sender, header.channel),
        context.roster,
        &context.devices.certs,
        &context.devices.revocations,
        own,
        state::unix_now(),
    )
    .map_err(|err| err.to_string())?;
    if content.counter.is_none() && matches!(context.group, Group::Sender) {
        return Err(
            "its content has no counter, which only content under hpke may lack".to_owned(),
        );
    }
    Ok(())
}

/// The service agent entry of `device` in `roster`, if it lists one.
pub fn service_entry(roster: &Roster, device: DeviceId) -> Option<&ServiceAgent> {
    (roster.body().service_agents.iter()).find(|agent| agent.device == device)
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
    message_text(json, msg_id)
}

/// The text of `json`, an A2A message in JSON, its text parts each on a line, once its `messageId`
/// is `msg_id` in text form.
pub fn message_text(json: &str, msg_id: MessageId) -> Result<String, String> {
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
        Content, DeliveryStatus, GRANT_LABEL, Grant, GrantBody, HpkePayload, HpkePlaintext,
        MessageId, NonEmpty, RawEnvelope, Receipt, ReceiptStatus, Scheme, Unknown,
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
        /// The signing key of this device, an admin's.
        ada_key: SigningKey,
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
                    ..SpaceDevices::default()
                },
                channel,
                bob: DeviceId::from_signing_key(&bob_key.public_key()),
                bob_key,
                server: "skald.example".parse().unwrap(),
                members,
                bob_account: bob,
                ada_key: this_key,
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

        /// The space's devices with one more of Bob's, of `kind`, issued by his device; and its ID.
        fn with_bobs(&self, kind: DeviceKind) -> (SpaceDevices, DeviceId) {
            let (cert, ..) = (self.bob_account).device(kind, &self.bob_key, Some(self.bob));
            let id = cert.body().device;
            let mut devices = SpaceDevices {
                ids: self.devices.ids.clone(),
                certs: self.devices.certs.clone(),
                revocations: Vec::new(),
                records: Vec::new(),
            };
            devices.ids.push(id);
            devices.certs.push(cert);
            (devices, id)
        }

        fn checked(&self, header: &Header, content: &Content) -> Result<Received, String> {
            self.checked_in(&self.roster, header, content, &self.bob_key)
        }
    }

    /// A message from an admitted device of a member of the channel, from its account's person
    /// address in the space on this server, with a counter and an A2A message whose `messageId` is
    /// `msg_id`, gives its text parts, each on a line, `from`, the device's name and its level,
    /// `peer` for a device of another account.
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
                level: TrustLevel::Peer,
                text: "first\nsecond".to_owned(),
                counter: NonZeroU64::new(1),
                hops: content.hops,
                reply_to: None,
                receipt: None,
                result: None,
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

        let (devices, id) = world.with_bobs(DeviceKind::Browser);
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

    /// Content from Bob's `service` device, which the revision lists as `ci/deploy-bot`, with a
    /// grant this device signs, an admin's: without `counter` it passes under `hpke` and fails under
    /// `mls`, where it passes with one.
    #[test]
    fn service_content_without_counter_passes_only_under_hpke() {
        let world = World::new();
        let (devices, id) = world.with_bobs(DeviceKind::Service);
        let mut body = world.roster.body().clone();
        body.service_agents.push(ServiceAgent {
            device: id,
            owner: "ci".parse().unwrap(),
            agent: "deploy-bot".parse().unwrap(),
            unknown: Unknown::default(),
        });
        let roster = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
        let grant = GrantBody {
            service: id,
            channels: NonEmpty::new(world.channel),
            signer: world.this,
            expires: state::unix_now() + 3600,
            unknown: Unknown::default(),
        };
        let signature = world.ada_key.sign(GRANT_LABEL, &[&grant.to_cbor()]);
        let header = world.header();
        let content = Content {
            from: "acme/ci/deploy-bot@skald.example".parse().unwrap(),
            counter: None,
            grant: Some(Grant {
                body: grant,
                signature,
            }),
            ..world.content(World::a2a(&header, &["deployed"]))
        };
        let counted = Content {
            counter: NonZeroU64::new(1),
            ..content.clone()
        };
        let context = |group| Context {
            devices: &devices,
            ..world.context(&roster, group)
        };
        let checked =
            |content: &Content, group| check(id, |_| Ok(content.clone()), &header, &context(group));

        assert_eq!(
            checked(&content, Group::Sender),
            Err("its content has no counter, which only content under hpke may lack".to_owned())
        );
        assert!(checked(&counted, Group::Sender).is_ok());
        assert!(checked(&content, Group::Held(None)).is_ok());
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

    /// Content without `counter`, without an A2A message or a receipt, or whose A2A message is no
    /// JSON or has another `messageId` than `msg_id`, fails. A receipt passes without text.
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
        let Some(Body::Receipt(read)) = receipt.body.clone() else {
            unreachable!("a receipt");
        };
        assert_eq!(
            world.checked(&header, &receipt),
            Ok(Received {
                from: receipt.from.clone(),
                to: receipt.to.clone(),
                device: "laptop".to_owned(),
                level: TrustLevel::Peer,
                text: String::new(),
                counter: NonZeroU64::new(1),
                hops: receipt.hops,
                reply_to: None,
                receipt: Some(read),
                result: None,
            })
        );
        for (content, why) in [
            (uncounted, "its content has no counter"),
            (later, "it holds no A2A message"),
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

    /// A task result passes with its response's text; one whose response has another `messageId`
    /// than `msg_id` fails.
    #[test]
    fn task_result_gives_its_response_s_text() {
        let world = World::new();
        let header = world.header();
        let result = TaskResult {
            task: TaskId::generate(),
            claim_id: skald_proto::ClaimId::generate(),
            generation: 2,
            outcome: skald_proto::Outcome::Done,
            response: World::a2a(&header, &["deployed", "build 412"]),
            unknown: Unknown::default(),
        };
        let from = "acme/bob/claude@skald.example".parse().unwrap();
        let to: Address = "acme/ada/codex@skald.example".parse().unwrap();
        let counter = NonZeroU64::MIN;
        let content = result.content((from, to.clone()), counter, 3);

        assert_eq!(
            world.checked(&header, &content),
            Ok(Received {
                from: content.from.clone(),
                to: Some(to.clone()),
                device: "laptop".to_owned(),
                level: TrustLevel::Peer,
                text: "deployed\nbuild 412".to_owned(),
                counter: Some(counter),
                hops: 3,
                reply_to: None,
                receipt: None,
                result: Some(result.clone()),
            })
        );
        let other = Header {
            msg_id: MessageId::generate(),
            ..header.clone()
        };
        assert_eq!(
            world.checked(&other, &content),
            Err("its A2A messageId is not its msg_id".to_owned())
        );
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
