//! Presence uploads (`docs/protocol/agents.md`, "Presence", "Uploads"): the device keeps a value in
//! each channel whose group it is a member of, listing each agent it registered in the channel's
//! space, and `skald --agent <name> set-status` sets an agent's state and note ([`set_status`]).
//!
//! - An agent is `available` while a session of it runs through the helper, an MCP server's or a
//!   `skald recv --wait`, or `busy` once it set itself so; otherwise `offline`. A `skald recv
//!   --wait` that ended counts as running for [`crate::wait::LINGER`] more, as an agent runs the
//!   next once it has handled what it got. `since` is when it entered that state; its note shows
//!   while a session of it runs. A new MCP session of an agent starts it `available`, with no
//!   note.
//! - The helper checks the values after each command it runs, when its sessions change, after it
//!   takes in what came for an MCP session, before a value expires and before it exits, its agents
//!   then `offline`. Out of the helper, a run as an agent checks them once, after its command
//!   ([`keep`]), its agents `offline`: only the helper knows the sessions.
//! - A check uploads in every channel together once an agent's state or note changed there or a
//!   value is near its `expires`; otherwise only in a channel whose epoch changed.
//! - A value is sealed only by a device not revoked, under the channel's current epoch, judged
//!   clean, while no Commit of the device waits there; its `revision` is recorded before the
//!   upload, as a counter is. An upload that fails is tried again by a later check, the helper's
//!   within [`RETRY`] seconds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use skald_crypto::PresenceKey;
use skald_proto::{
    Address, AgentState, AgentStatus, Cbor as _, ChannelId, DeviceKind, NOTE_MAX, Name,
    PresenceBody, SpaceId, Unknown, VersionRange,
};

use crate::devices::text;
use crate::spaces::Follower;
use crate::state::{self, State, Uploaded};
use crate::{agents, folders, follow, groups, presence, revocations, secrets, service};

/// The setting of how far after an upload its value expires, in seconds; a helper takes it from
/// the command that starts it.
pub const EXPIRES_SETTING: &str = "SKALD_PRESENCE_EXPIRES";

/// How far after an upload its value expires by default, and at most.
pub const EXPIRES: u64 = 900;

/// How long the helper waits before it tries a failed upload again, in seconds.
pub const RETRY: u64 = 30;

/// Whether this process is the helper, which checks the values itself.
static HELPER: AtomicBool = AtomicBool::new(false);

/// The settings of the last operation the helper ran, which its own checks use.
static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);

/// What a check takes of a run's settings.
#[derive(Clone, Copy)]
pub struct Settings {
    pub storage: secrets::Storage,
    pub speaks: VersionRange,
    pub timeout: Duration,
}

/// An agent with a running session in a space.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Running {
    pub space: SpaceId,
    pub agent: String,
}

/// The agents whose sessions run, and when the last session of each other agent that ran through
/// the helper ended, in Unix seconds: the `since` of its `offline`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub running: BTreeSet<Running>,
    pub ended: BTreeMap<Running, u64>,
}

impl Settings {
    /// What a check takes of `operation`'s settings.
    pub fn of(operation: &crate::Operation) -> Self {
        Settings {
            storage: operation.key_storage,
            speaks: operation.speaks,
            timeout: operation.timeout,
        }
    }
}

/// The state an agent sets itself to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Settable {
    Busy,
    Available,
}

/// Notes that this process is the helper.
pub fn set_helper() {
    HELPER.store(true, Ordering::SeqCst);
}

/// Whether this process is the helper.
pub fn in_helper() -> bool {
    HELPER.load(Ordering::SeqCst)
}

/// Keeps `settings` for the helper's own checks.
pub fn remember(settings: Settings) {
    *SETTINGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(settings);
}

/// The settings the helper keeps for its own checks, if it ran an operation.
pub fn remembered() -> Option<Settings> {
    *SETTINGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// How far after an upload its value expires, in seconds: the setting, from 1 to [`EXPIRES`].
pub fn expires_in() -> u64 {
    std::env::var(EXPIRES_SETTING)
        .ok()
        .and_then(|given| given.parse::<u64>().ok())
        .map_or(EXPIRES, |seconds| seconds.clamp(1, EXPIRES))
}

/// `skald --agent <agent> set-status`: sets `agent`'s state in the working directory's space to
/// `state`, with `note`, by the state in `dir`; fails, changing nothing, for a note over
/// [`NOTE_MAX`] bytes or a folder in no space.
pub fn set_status(
    dir: &Path,
    agent: &Name,
    state: Settable,
    note: Option<&str>,
) -> Result<(), String> {
    let note = note.filter(|note| !note.is_empty());
    note_fits(note)?;
    let space = folders::working_space(dir)
        .map_err(|why| format!("this agent sees no space: {why}; nothing changed"))?;
    let held = State::open(dir).map_err(text)?;
    held.add_agent(space.id, agent.as_str()).map_err(text)?;
    held.set_agent_status(space.id, agent.as_str(), state == Settable::Busy, note)
        .map_err(text)?;
    println!(
        "Status: {}",
        match state {
            Settable::Busy => "busy",
            Settable::Available => "available",
        }
    );
    Ok(())
}

/// Fails, saying the limit, for a note over [`NOTE_MAX`] bytes.
pub fn note_fits(note: Option<&str>) -> Result<(), String> {
    match note {
        Some(note) if note.len() > NOTE_MAX => Err(format!(
            "the note is {} bytes in UTF-8, over the limit of {NOTE_MAX}; nothing changed",
            note.len()
        )),
        _ => Ok(()),
    }
}

/// Checks the device's values by the state in `dir`, with the sessions of `live`, uploading what
/// is due ([`self`]); gives when the next check is due, in Unix seconds, if one is. What fails
/// leaves its upload to a later check.
pub fn keep(dir: &Path, settings: Settings, live: &Live) -> Option<u64> {
    if service::stateless() || !dir.join(state::FILE).is_file() {
        return None;
    }
    if !matches!(service::from_env(), Ok(None)) {
        return None;
    }
    let now = state::unix_now();
    match checked(dir, settings, live, now) {
        Ok(due) => due,
        Err(_) => Some(now + RETRY),
    }
}

/// What the helper last checked the values with: its sessions' agents, and when the next check is
/// due, in Unix seconds.
#[derive(Default)]
pub struct Kept {
    running: Option<BTreeSet<Running>>,
    due: Option<u64>,
}

impl Kept {
    /// Checks the values by the state in `dir` with the sessions of `live` ([`keep`]) if `anyway`,
    /// if the agents that run changed or once the next check is due, with the settings the helper
    /// keeps; with none yet, leaves it to the next.
    pub fn check(&mut self, dir: &Path, live: &Live, anyway: bool) {
        let due = self.due.is_some_and(|due| state::unix_now() >= due);
        if !anyway && !due && self.running.as_ref() == Some(&live.running) {
            return;
        }
        let Some(settings) = remembered() else {
            return;
        };
        self.due = keep(dir, settings, live);
        self.running = Some(live.running.clone());
    }
}

/// An agent's status in a value, before its address is known.
#[derive(Clone, Debug, PartialEq, Serialize)]
struct Shown {
    agent: String,
    state: u8,
    since: u64,
    note: Option<String>,
    runtime: Option<String>,
}

/// A channel the device may upload a value in: its space, its current epoch and that epoch's
/// presence key.
struct Open {
    space: SpaceId,
    channel: ChannelId,
    epoch: u64,
    key: PresenceKey,
}

/// [`keep`] at `now`.
fn checked(dir: &Path, settings: Settings, live: &Live, now: u64) -> Result<Option<u64>, String> {
    let Some(mut follower) =
        Follower::unchecked(dir, settings.storage, settings.speaks, settings.timeout)?
    else {
        return Ok(None);
    };
    if follower.cert.body().kind != DeviceKind::Client {
        return Ok(None);
    }
    let span = expires_in();
    let early = (span / 3).max(1);
    let mut spaces = BTreeMap::new();
    let mut channels = Vec::new();
    for space in follower.state.spaces().map_err(text)? {
        let roster = follow::decode(&space.accepted, &space.name)?;
        let shown = shown(&follower.state, space.id, live, now)?;
        channels.extend(
            roster
                .body()
                .channels
                .iter()
                .map(|channel| (space.id, channel.id)),
        );
        spaces.insert(space.id, (roster, shown));
    }
    let keys = follower.keys()?;
    let waiting: BTreeSet<ChannelId> = (follower.state.mls_outbox().map_err(text)?)
        .iter()
        .map(|outgoing| outgoing.channel)
        .collect();
    let held = groups::read(&follower, &keys, |device| {
        (channels.iter())
            .filter_map(|&(space, channel)| {
                let epoch = device.epoch(channel).ok()?;
                let key = device.history_key(channel, epoch)?.presence_key();
                Some(Open {
                    space,
                    channel,
                    epoch,
                    key,
                })
            })
            .collect::<Vec<_>>()
    })?;
    // The channels a value may be uploaded in, each with what the server holds of the device's.
    let mut open = Vec::new();
    for channel in held {
        let clean = follower.state.judgments(channel.channel).map_err(text)?;
        if clean.get(&channel.epoch) != Some(&true) || waiting.contains(&channel.channel) {
            continue;
        }
        let last = (follower
            .state
            .channel_presence(channel.channel)
            .map_err(text)?)
        .and_then(|presence| presence.uploaded);
        open.push((channel, last));
    }
    let listed = |space: SpaceId| -> String {
        let shown = spaces.get(&space).map(|(_, shown)| shown.as_slice());
        serde_json::to_string(&shown.unwrap_or_default()).expect("statuses as JSON")
    };
    let all = open.iter().any(|(channel, last)| match last {
        Some(last) => last.agents != listed(channel.space) || now + early >= last.expires,
        None => true,
    });
    let mut due = None;
    let mut sealing = Vec::new();
    for (channel, last) in open {
        match last {
            Some(last) if !all && last.epoch == channel.epoch => {
                due = earliest(due, last.expires - early.min(last.expires));
            }
            _ => sealing.push(channel),
        }
    }
    if sealing.is_empty() {
        return Ok(due);
    }
    // A revoked device signs and sends nothing, and checks no more.
    let signer = (&follower.own.master, &follower.cert);
    if revocations::revoked(
        &mut follower.connection,
        &follower.token,
        &follower.state,
        signer,
    )? {
        return Ok(None);
    }
    // Only agents need it, for their addresses.
    let named = (sealing.iter()).any(|channel| !spaces[&channel.space].1.is_empty());
    let server = match named {
        true => agents::server_name(&mut follower.connection)?,
        false => None,
    };
    let mut failed = false;
    for channel in sealing {
        let (roster, shown) = &spaces[&channel.space];
        let body = roster.body();
        let handle = body
            .member(follower.own.account)
            .map(|member| member.handle.clone());
        let statuses = match (&server, handle) {
            (_, _) if shown.is_empty() => Vec::new(),
            (Some(server), Some(handle)) => statuses(shown, (&body.name, &handle, server)),
            // Its agents have no address.
            _ => {
                failed = true;
                continue;
            }
        };
        let uploaded = Uploaded {
            epoch: channel.epoch,
            expires: now + span,
            agents: listed(channel.space),
        };
        match upload(&mut follower, &keys, &channel, (&uploaded, statuses)) {
            Ok(()) => {
                follower
                    .state
                    .presence_uploaded(channel.channel, &uploaded)
                    .map_err(text)?;
                due = earliest(due, uploaded.expires - early.min(uploaded.expires));
            }
            Err(_) => failed = true,
        }
    }
    if failed {
        due = earliest(due, now + RETRY);
    }
    Ok(due)
}

/// The earlier of `due` and `at`.
fn earliest(due: Option<u64>, at: u64) -> Option<u64> {
    Some(due.map_or(at, |due| due.min(at)))
}

/// The status of each agent of the device in `space`, by name, as the sessions of `live` make it
/// at `now`, recording each one's new state with its `since`: `now`, or for an `offline`, when its
/// last session ended, if `live` has it.
fn shown(held: &State, space: SpaceId, live: &Live, now: u64) -> Result<Vec<Shown>, String> {
    let mut shown = Vec::new();
    for record in held.agent_statuses(space).map_err(text)? {
        let this = Running {
            space,
            agent: record.name.clone(),
        };
        let runs = live.running.contains(&this);
        let state = match (runs, record.busy) {
            (false, _) => AgentState::Offline,
            (true, true) => AgentState::Busy,
            (true, false) => AgentState::Available,
        };
        let code = state.code();
        let since = match record.shown {
            Some((shown, since)) if shown == code => since,
            last => {
                let ended = (live.ended.get(&this).copied())
                    .filter(|ended| !runs && *ended <= now)
                    .filter(|ended| last.is_none_or(|(_, since)| *ended >= since));
                let since = ended.unwrap_or(now);
                held.agent_shown(space, &record.name, (code, since))
                    .map_err(text)?;
                since
            }
        };
        shown.push(Shown {
            agent: record.name,
            state: code,
            since,
            note: record.note.filter(|_| runs),
            runtime: record.runtime,
        });
    }
    Ok(shown)
}

/// The statuses of `shown` in a value, each agent's address in the space `space` of the member
/// `handle` on `server`; an agent whose name is not valid is left out.
fn statuses(
    shown: &[Shown],
    (space, handle, server): (&Name, &Name, &skald_proto::ServerName),
) -> Vec<AgentStatus> {
    (shown.iter())
        .filter_map(|shown| {
            let agent = shown.agent.parse().ok()?;
            Some(AgentStatus {
                agent: Address::new(space.clone(), handle.clone(), agent, server.clone()),
                state: AgentState::of_code(u64::from(shown.state)).unwrap_or(AgentState::Offline),
                since: shown.since,
                note: shown.note.clone(),
                runtime: (shown.runtime.as_deref()).and_then(|runtime| runtime.parse().ok()),
                unknown: Unknown::default(),
            })
        })
        .collect()
}

/// Uploads the device's value of `uploaded`, listing `agents`, in `channel`, under the next
/// revision there, which it records first.
fn upload(
    follower: &mut Follower,
    keys: &crate::device::Keys,
    channel: &Open,
    (uploaded, agents): (&Uploaded, Vec<AgentStatus>),
) -> Result<(), String> {
    let revision = (follower.state)
        .next_presence_revision(channel.channel)
        .map_err(text)?;
    let body = PresenceBody {
        device: follower.this,
        channel: channel.channel,
        epoch: uploaded.epoch,
        revision,
        expires: uploaded.expires,
        agents,
        serves: Vec::new(),
        unknown: Unknown::default(),
    };
    let value = (channel.key)
        .seal(&body, keys.signing())
        .map_err(|err| err.to_string())?;
    let proof = follower.proof(keys)?;
    let path = presence::path(channel.space, channel.channel);
    let token = follower.token.clone();
    (follower.connection)
        .put_cbor_proved(&path, (&token, &proof), &value.to_cbor())
        .map_err(text)?;
    Ok(())
}

/// The runtime of an MCP session whose client gives itself the name `client`, made a `name` as an
/// agent's default name makes it ([`crate::mcp::default_name`]); none if nothing is left of it.
pub fn runtime(client: &str) -> Option<String> {
    let part = crate::mcp::name_of(client);
    part.parse::<Name>().ok().map(|name| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client's name is made a name as an agent's default name makes it.
    #[test]
    fn runtime_is_the_client_s_name_made_a_name() {
        assert_eq!(runtime("Codex CLI").as_deref(), Some("codex-cli"));
        assert_eq!(runtime("claude-code").as_deref(), Some("claude-code"));
        assert_eq!(runtime("!!"), None);
    }
}
