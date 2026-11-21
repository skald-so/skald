//! The presence directory: `skald agents` and the MCP tool `agents` list the agents of a space from
//! the presence values of its channels (`docs/protocol/agents.md`, "Presence").
//!
//! - The run accepts the space's latest revisions and goes through the mailbox as `skald send`
//!   does, showing nothing, then gets the values of each channel of the space whose group the
//!   device holds, proving its device.
//! - It opens each value with the key of its epoch: one the device holds, or one it reaches back
//!   from those through the channel's key chain. It drops, warning its person, each value that
//!   fails a reader check: given among another channel's values; of a device without a valid
//!   certificate of kind `client` or `service` admitted to the channel by the space's last accepted
//!   revision; failing [`PresenceKey::open`]; with a `revision` below the highest it accepted for
//!   that channel and device, or equal to it in another value; listing an agent its device does not
//!   send as. A value that fails only check 4 shows its agents `offline`, last seen at `expires`.
//! - It lists each agent once per device, from the device's latest value that passes, labelled
//!   with the device's name and trust level: the person's lines in full, an agent's compact,
//!   [`forms::DATA_WARNING`] before its first line below `own`, `--json` with every field. An
//!   address two devices list, neither as `offline`, is marked as a duplicate.
//! - As an agent, it covers only the space of its folder.
//! - When a device registers an agent whose address another device of its account lists as
//!   running, it warns its person and keeps a line for the agent's next tool result
//!   ([`registered`], [`notices`]). It reads the values as a listing does, but without going
//!   through the mailbox and without warning of those it drops.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use skald_crypto::{PresenceError, PresenceKey, trust_level};
use skald_proto::{
    Address, AgentState, Cbor as _, Cert, ChannelId, DeviceId, Name, Presence, PresenceBody,
    Roster, ServerName, SpaceId, TrustLevel, VersionRange,
};

use crate::agents::{self, server_name};
use crate::connection::printable;
use crate::devices::text;
use crate::folders;
use crate::follow::SpaceDevices;
use crate::forms::{self, Form, bracketed};
use crate::groups;
use crate::history_reading::{reach, rfc3339, sends_as};
use crate::key_chain;
use crate::recv;
use crate::secrets;
use crate::send;
use crate::spaces::{self, Follower, Scope, Synced};
use crate::state::{self, State};

/// `PresenceValues` in the OpenAPI spec.
#[derive(Deserialize)]
struct Values {
    values: Vec<String>,
}

/// One agent as the directory lists it: of a device, from its latest value that passed.
#[derive(Debug, Clone, PartialEq)]
struct Listed {
    agent: Address,
    state: AgentState,
    since: u64,
    note: Option<String>,
    runtime: Option<String>,
    device: DeviceId,
    /// The device's certificate `name`.
    device_name: String,
    level: TrustLevel,
    /// The `expires` of a value past it.
    last_seen: Option<u64>,
    /// Whether another device lists the address and neither as `offline`.
    duplicate: bool,
    /// The `expires` of its value, which picks the device's latest one.
    expires: u64,
}

impl Listed {
    /// Whether its device lists it as running: neither `offline` nor in a value past `expires`.
    fn running(&self) -> bool {
        self.state != AgentState::Offline && self.last_seen.is_none()
    }

    /// Its state as a word: `offline` in a value past `expires`.
    fn state_word(&self) -> &'static str {
        if self.last_seen.is_some() {
            return state_name(AgentState::Offline);
        }
        state_name(self.state)
    }

    /// When it entered its state, or, in a value past `expires`, when it was last seen.
    fn when(&self) -> String {
        match self.last_seen {
            Some(expires) => format!(", last seen {}", rfc3339(expires)),
            None => format!(" since {}", rfc3339(self.since)),
        }
    }

    /// Its line in `form`.
    fn shown(&self, form: Form) -> String {
        match form {
            Form::Full => {
                let mut line = format!("{} {}{}", self.agent, self.state_word(), self.when());
                if self.duplicate {
                    line.push_str(", also running on another device of its account");
                }
                if let Some(runtime) = &self.runtime {
                    line.push_str(&format!(", runtime {runtime}"));
                }
                if let Some(note) = &self.note {
                    line.push_str(&format!(", note: {}", printable(note)));
                }
                format!(
                    "{line}; on the device {} ({}), {}",
                    printable(&self.device_name),
                    self.device,
                    self.level
                )
            }
            Form::Short => {
                let mut rest = format!("{}{}", self.state_word(), self.when());
                if self.duplicate {
                    rest.push_str(" (duplicate)");
                }
                if let Some(runtime) = &self.runtime {
                    rest.push_str(&format!(" ({runtime})"));
                }
                if let Some(note) = &self.note {
                    rest.push_str(&format!(": {}", bracketed(note)));
                }
                format!(
                    "[{}/{} · {} · {}] {rest}",
                    self.agent.owner(),
                    self.agent.agent(),
                    bracketed(&self.device_name),
                    self.level
                )
            }
            Form::Json => forms::object(
                "agent",
                json!({
                    "agent": self.agent.to_string(),
                    "state": self.state_word(),
                    "since": self.since,
                    "note": self.note,
                    "runtime": self.runtime,
                    "device": self.device.to_string(),
                    "device_name": self.device_name,
                    "level": self.level.name(),
                    "duplicate": self.duplicate,
                    "last_seen": self.last_seen,
                }),
            ),
        }
    }
}

/// `state` as a word.
fn state_name(state: AgentState) -> &'static str {
    match state {
        AgentState::Available => "available",
        AgentState::Busy => "busy",
        AgentState::Waiting => "waiting",
        AgentState::Offline => "offline",
    }
}

/// `skald agents`: lists the agents of the space named `space`, or of the working directory's,
/// by the state in `dir`, its secrets in `storage`; as an agent if `agent`, only of its folder's
/// space, holding its warnings for the person; as JSON if `json`.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    space: Option<&str>,
    agent: bool,
    json: bool,
) -> Result<(), String> {
    recv::holding_warnings(dir, agent, || {
        let name = match space {
            Some(name) => name.to_owned(),
            None => folders::working_space(dir)?.name,
        };
        let scope = agents::scope(dir, &name, agent)?;
        let synced = spaces::synced(dir, storage, speaks, timeout, &name, scope)?;
        let listed = directory(synced, scope, Reading::Listing)?;
        show(&listed, Form::of(agent, json), &name);
        Ok(())
    })
}

/// Shows `listed`, the agents of the space `space`, in `form`.
fn show(listed: &[Listed], form: Form, space: &str) {
    if listed.is_empty() {
        match form {
            Form::Full => println!("No agent of the space {space} has a presence value."),
            Form::Short => println!("No agent of this space has a presence value."),
            Form::Json => {}
        }
        return;
    }
    let mut warned = false;
    for one in listed {
        if form == Form::Short && !warned && one.level < TrustLevel::Own {
            warned = true;
            println!("{}", forms::DATA_WARNING);
        }
        println!("{}", one.shown(form));
    }
}

/// How a run reads the directory.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reading {
    /// A listing: it goes through the mailbox first and warns of each value it drops.
    Listing,
    /// A registration's check: it does neither.
    Check,
}

/// The agents the presence values of the channels of `synced`'s space list, within `scope`, read
/// as `reading` says, sorted: `own` first, then by address and device name.
fn directory(synced: Synced, scope: Scope, reading: Reading) -> Result<Vec<Listed>, String> {
    let Synced {
        mut follower,
        space,
        roster,
        devices,
    } = synced;
    let (roster, devices) = match reading {
        Reading::Listing => {
            send::through_mailbox(&mut follower, scope, space.id, (roster, devices))?
        }
        Reading::Check => (roster, devices),
    };
    let warn = |warning: String| {
        if reading == Reading::Listing {
            eprintln!("skald: warning: {warning}");
        }
    };
    let server = server_name(&mut follower.connection)?;
    let keys = follower.keys()?;
    let channels: Vec<ChannelId> = roster.body().channels.iter().map(|it| it.id).collect();
    let held: Vec<ChannelId> = groups::read(&follower, &keys, |device| {
        (channels.iter().copied())
            .filter(|&channel| device.epoch(channel).is_ok())
            .collect()
    })?;
    let mut given = Vec::new();
    for channel in held {
        match values(&mut follower, (space.id, channel), &warn) {
            Ok(values) => given.push((channel, values)),
            Err(err) => warn(format!(
                "the presence values of the channel {channel} are not read: {err}"
            )),
        }
    }
    let presence_keys = opening_keys(&mut follower, space.id, &given, &warn)?;
    let reader = Reader {
        follower: &follower,
        roster: &roster,
        devices: &devices,
        server: server.as_ref(),
        keys: &presence_keys,
        now: state::unix_now(),
    };
    let mut listed: Vec<Listed> = Vec::new();
    for (channel, values) in &given {
        for bytes in values {
            match reader.read(*channel, bytes) {
                Ok(agents) => listed.extend(agents),
                Err(dropped) => warn(dropped),
            }
        }
    }
    Ok(sorted(latest(listed)))
}

/// The path of the presence values of `channel` in `space`.
pub fn path(space: SpaceId, channel: ChannelId) -> String {
    format!("/spaces/{space}/channels/{channel}/presence")
}

/// The values the server gives of the channel `at`, each as its bytes, to the device of
/// `follower`; those that are no base64url are left out, given to `warn`.
fn values(
    follower: &mut Follower,
    (space, channel): (SpaceId, ChannelId),
    warn: &dyn Fn(String),
) -> Result<Vec<Vec<u8>>, String> {
    let proof = follower.proof(&follower.keys()?)?;
    let body = (follower.connection)
        .get_proved(&path(space, channel), (&follower.token, &proof))
        .map_err(text)?;
    let Values { values } = serde_json::from_slice(&body)
        .map_err(|_| "unexpected answer from the server: a channel's presence values".to_owned())?;
    Ok((values.iter())
        .filter_map(|value| match URL_SAFE_NO_PAD.decode(value) {
            Ok(bytes) => Some(bytes),
            Err(_) => {
                warn(format!(
                    "dropped a presence value of the channel {channel}: it is no base64url"
                ));
                None
            }
        })
        .collect())
}

/// The presence key of each channel and epoch of the values of `given` that the device holds the
/// history key of, or reaches through the channel's key chain, which it gets only for a channel
/// with a value of an epoch it holds no key of; a key chain not read is given to `warn`.
fn opening_keys(
    follower: &mut Follower,
    space: SpaceId,
    given: &[(ChannelId, Vec<Vec<u8>>)],
    warn: &dyn Fn(String),
) -> Result<BTreeMap<(ChannelId, u64), PresenceKey>, String> {
    let wanted: BTreeSet<(ChannelId, u64)> = (given.iter())
        .flat_map(|(channel, values)| {
            (values.iter())
                .filter_map(|bytes| Presence::from_cbor(bytes).ok())
                .map(move |value| (*channel, value.epoch))
        })
        .collect();
    let keys = follower.keys()?;
    let mut found = groups::read(follower, &keys, |device| {
        (wanted.iter())
            .filter_map(|&(channel, epoch)| {
                let key = device.history_key(channel, epoch)?;
                Some(((channel, epoch), key.presence_key()))
            })
            .collect::<BTreeMap<_, _>>()
    })?;
    let behind: BTreeSet<ChannelId> = (wanted.iter())
        .filter(|at| !found.contains_key(at))
        .map(|(channel, _)| *channel)
        .collect();
    for channel in behind {
        let links = match key_chain::links(follower, (space, channel)) {
            Ok(links) => links,
            Err(err) => {
                warn(format!(
                    "the key chain of the channel {channel} is not read: {err}"
                ));
                continue;
            }
        };
        let reached = groups::read(follower, &keys, |device| reach(device, channel, &links))?;
        for (epoch, key) in reached {
            if wanted.contains(&(channel, epoch)) {
                found
                    .entry((channel, epoch))
                    .or_insert_with(|| key.presence_key());
            }
        }
    }
    Ok(found)
}

/// What the device reads values against: its state and certificate, the space's last accepted
/// revision and its accounts' devices, the server's name, the presence keys it holds, and now.
struct Reader<'r> {
    follower: &'r Follower,
    roster: &'r Roster,
    devices: &'r SpaceDevices,
    server: Option<&'r ServerName>,
    keys: &'r BTreeMap<(ChannelId, u64), PresenceKey>,
    now: u64,
}

impl Reader<'_> {
    /// The agents of `bytes`, a value given among those of `channel`, once it passes the reader
    /// checks; why it is dropped otherwise, naming its device. Records its revision.
    fn read(&self, channel: ChannelId, bytes: &[u8]) -> Result<Vec<Listed>, String> {
        let value = Presence::from_cbor(bytes).map_err(|err| {
            format!("dropped a presence value of the channel {channel}: it does not decode: {err}")
        })?;
        let device = value.device;
        let dropped = |why: String| {
            format!(
                "dropped the presence value of the device {} in the channel {channel}: {why}",
                self.device_name(device)
            )
        };
        let (cert, body) = self.check(channel, &value, bytes).map_err(dropped)?;
        let expired = body.as_ref().err().map(|body| body.expires);
        let body = body.unwrap_or_else(|body| body);
        let level = trust_level(self.follower.own.account, &cert, self.roster.body())
            .ok_or_else(|| dropped("its device has no trust level in this space".to_owned()))?;
        let device_name = cert.body().name.clone();
        Ok((body.agents.into_iter())
            .map(|status| Listed {
                agent: status.agent,
                state: status.state,
                since: status.since,
                note: status.note,
                runtime: status.runtime.map(|runtime| runtime.to_string()),
                device,
                device_name: device_name.clone(),
                level,
                last_seen: expired,
                duplicate: false,
                expires: body.expires,
            })
            .collect())
    }

    /// The certificate of `value`'s device and its body, once it passes the reader checks: `Ok`
    /// before its `expires`, `Err` after; why it fails otherwise. Records its revision.
    fn check(
        &self,
        channel: ChannelId,
        value: &Presence,
        bytes: &[u8],
    ) -> Result<(Cert, Result<PresenceBody, PresenceBody>), String> {
        if value.channel != channel {
            return Err("it is given among the values of another channel".to_owned());
        }
        let mut admitted = groups::admitted(self.follower, self.roster, self.devices, channel);
        let cert = (admitted.remove(&value.device)).ok_or_else(|| {
            "its device has no valid certificate admitted to the channel by the space's last \
             revision this device accepted"
                .to_owned()
        })?;
        let key = (self.keys.get(&(channel, value.epoch)))
            .ok_or_else(|| format!("this device holds no key of its epoch {}", value.epoch))?;
        let body = match key.open(value, &cert.body().signing, self.now) {
            Ok(body) => Ok(body),
            Err(PresenceError::Expired { body }) => Err(*body),
            Err(err) => return Err(err.to_string()),
        };
        let checked = body.as_ref().unwrap_or_else(|body| body);
        if let Some(status) = (checked.agents.iter())
            .find(|status| !sends_as(self.roster, &cert, &status.agent, self.server))
        {
            return Err(format!(
                "it lists {}, an address its device does not send as",
                status.agent
            ));
        }
        self.accept(channel, value.device, checked.revision, bytes)?;
        Ok((cert, body))
    }

    /// Check 3: records `revision`, of the value `bytes` of `device` in `channel`, unless it is
    /// below the highest the device accepted for them, or equal to it in another value.
    fn accept(
        &self,
        channel: ChannelId,
        device: DeviceId,
        revision: u64,
        bytes: &[u8],
    ) -> Result<(), String> {
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        let state = &self.follower.state;
        match state.presence_revision(channel, device).map_err(text)? {
            Some((highest, accepted))
                if revision < highest || (revision == highest && accepted != hash) =>
            {
                Err(format!(
                    "its revision {revision} is not above {highest}, the highest this device \
                     accepted of that device in the channel"
                ))
            }
            Some((highest, _)) if revision == highest => Ok(()),
            _ => state
                .accept_presence((channel, device), revision, &hash)
                .map_err(text),
        }
    }

    /// `device` as a warning names it: its certificate's `name`, if the device knows it, and its
    /// ID.
    fn device_name(&self, device: DeviceId) -> String {
        match (self.devices.certs.iter()).find(|cert| cert.body().device == device) {
            Some(cert) => format!("{} ({device})", printable(&cert.body().name)),
            None => device.to_string(),
        }
    }
}

/// Of `listed`, each agent once per device, from the device's latest value: one before its
/// `expires` over one past it, then the one with the latest `expires`; each address two devices
/// list running marked.
fn latest(listed: Vec<Listed>) -> Vec<Listed> {
    let mut by_device: BTreeMap<(String, DeviceId), Listed> = BTreeMap::new();
    for one in listed {
        let key = (one.agent.to_string(), one.device);
        let newer = |kept: &Listed| {
            (one.last_seen.is_none(), one.expires) > (kept.last_seen.is_none(), kept.expires)
        };
        match by_device.get(&key) {
            Some(kept) if !newer(kept) => {}
            _ => {
                by_device.insert(key, one);
            }
        }
    }
    let mut running: BTreeMap<String, usize> = BTreeMap::new();
    for one in by_device.values().filter(|one| one.running()) {
        *running.entry(one.agent.to_string()).or_default() += 1;
    }
    (by_device.into_values())
        .map(|mut one| {
            one.duplicate = one.running() && running.get(&one.agent.to_string()) > Some(&1);
            one
        })
        .collect()
}

/// `listed` with `own` first, then by address and device name.
fn sorted(mut listed: Vec<Listed>) -> Vec<Listed> {
    listed.sort_by(|one, other| {
        (other.level.cmp(&one.level))
            .then_with(|| one.agent.to_string().cmp(&other.agent.to_string()))
            .then_with(|| one.device_name.cmp(&other.device_name))
    });
    listed
}

/// The registration of `agent` in `space` on the device of the state in `dir`, its secrets in
/// `storage`: if another device of its account lists the agent's address as running, warns its
/// person and keeps a line for the agent's next tool result. Says nothing if the device cannot
/// tell, such as offline.
pub fn registered(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    agent: &Name,
    space: &state::Space,
) {
    recv::holding_warnings(dir, true, || {
        let scope = Scope::Only(space.id);
        let Ok(mut synced) = spaces::synced(dir, storage, speaks, timeout, &space.name, scope)
        else {
            return;
        };
        let (this, account) = (synced.follower.this, synced.follower.own.account);
        let Some(handle) =
            (synced.roster.body().member(account)).map(|member| member.handle.clone())
        else {
            return;
        };
        let Ok(Some(server)) = server_name(&mut synced.follower.connection) else {
            return;
        };
        let address = Address::new(
            synced.roster.body().name.clone(),
            handle,
            agent.clone(),
            server,
        );
        let Ok(listed) = directory(synced, scope, Reading::Check) else {
            return;
        };
        let elsewhere: Vec<&Listed> = (listed.iter())
            .filter(|one| one.agent == address && one.device != this && one.running())
            .collect();
        if elsewhere.is_empty() {
            return;
        }
        let names: Vec<String> = (elsewhere.iter())
            .map(|one| format!("the device {}", bracketed(&one.device_name)))
            .collect();
        let short = format!("{}/{}", address.owner(), address.agent());
        let names = names.join(", ");
        eprintln!(
            "skald: warning: {short} is registered on this device while {names} lists it running; \
             messages to {address} reach both"
        );
        let line =
            format!("skald: {short} also runs on {names}: messages to its address reach both");
        if let Err(err) =
            State::open(dir).and_then(|state| state.keep_notice(space.id, agent.as_str(), &line))
        {
            eprintln!("skald: warning: the local state keeps no line for the agent {agent}: {err}");
        }
    });
}

/// Says, on the error output, which goes into the tool's result, each line kept for the next
/// tool result of `agent` in the working directory's space of the state in `dir`.
pub fn notices(dir: &Path, agent: &Name) {
    let Ok(space) = folders::working_space(dir) else {
        return;
    };
    let lines = State::open(dir).and_then(|state| state.take_notices(space.id, agent.as_str()));
    for line in lines.unwrap_or_default() {
        eprintln!("{line}");
    }
}

/// The `agents` tool of an agent's MCP session ([`crate::mcp`]): the agent's `skald agents`.
pub fn tool(
    dir: &Path,
    storage: secrets::Storage,
    settings: (VersionRange, Duration),
) -> Result<(), String> {
    run(dir, storage, settings, None, true, false)
}

#[cfg(test)]
mod tests {
    use skald_crypto::SigningKey;

    use super::*;

    fn device() -> DeviceId {
        DeviceId::from_signing_key(&SigningKey::generate().public_key())
    }

    /// Bob's agent `name` on `device`, in `state`, from a value that expires at `expires`, past it
    /// if `past`.
    fn listed(
        name: &str,
        device: DeviceId,
        state: AgentState,
        (expires, past): (u64, bool),
    ) -> Listed {
        Listed {
            agent: format!("acme/bob/{name}@skald.example").parse().unwrap(),
            state,
            since: 1_800_000_000,
            note: None,
            runtime: None,
            device,
            device_name: "bob-laptop".to_owned(),
            level: TrustLevel::Peer,
            last_seen: past.then_some(expires),
            duplicate: false,
            expires,
        }
    }

    /// Each agent is listed once per device, from a value before its `expires` over one past it,
    /// then from the one with the latest `expires`.
    #[test]
    fn each_agent_is_listed_once_per_device_from_its_latest_value() {
        let (laptop, desktop) = (device(), device());
        let got = latest(vec![
            listed("api", laptop, AgentState::Busy, (1_800_000_900, false)),
            listed("api", laptop, AgentState::Available, (1_800_001_800, true)),
            listed("api", laptop, AgentState::Waiting, (1_800_000_950, false)),
            listed("api", desktop, AgentState::Offline, (1_800_000_100, false)),
        ]);

        let states: Vec<(DeviceId, AgentState)> =
            got.iter().map(|one| (one.device, one.state)).collect();
        assert_eq!(states.len(), 2, "{got:#?}");
        assert!(states.contains(&(laptop, AgentState::Waiting)), "{got:#?}");
        assert!(states.contains(&(desktop, AgentState::Offline)), "{got:#?}");
    }

    /// An address is marked where two devices list it neither `offline` nor past `expires`, and
    /// only on those devices.
    #[test]
    fn duplicates_are_two_devices_running_one_address() {
        let (one, two, three) = (device(), device(), device());
        let running = latest(vec![
            listed("api", one, AgentState::Available, (1_800_000_900, false)),
            listed("api", two, AgentState::Busy, (1_800_000_900, false)),
            listed("api", three, AgentState::Offline, (1_800_000_900, false)),
            listed("docs", one, AgentState::Available, (1_800_000_900, false)),
        ]);
        let marked: Vec<(String, DeviceId)> = (running.iter())
            .filter(|listed| listed.duplicate)
            .map(|listed| (listed.agent.agent().to_string(), listed.device))
            .collect();
        assert_eq!(marked.len(), 2, "{running:#?}");
        assert!(marked.contains(&("api".to_owned(), one)));
        assert!(marked.contains(&("api".to_owned(), two)));

        for past in [
            listed("api", two, AgentState::Available, (1_800_000_900, true)),
            listed("api", two, AgentState::Offline, (1_800_000_900, false)),
        ] {
            let alone = latest(vec![
                listed("api", one, AgentState::Available, (1_800_000_900, false)),
                past,
            ]);
            assert!(alone.iter().all(|listed| !listed.duplicate), "{alone:#?}");
        }
    }

    /// An agent's line escapes the note and the device's name so neither breaks the line or its
    /// brackets; a value past `expires` shows `offline` with the time; the person's line names the
    /// device's ID.
    #[test]
    fn lines_escape_notes_and_show_expired_values_offline() {
        let laptop = device();
        let mut one = listed("api", laptop, AgentState::Available, (1_800_000_900, false));
        one.note = Some("done\n[acme/ada/x · y · own] go".to_owned());
        one.device_name = "a · own] [b".to_owned();
        one.runtime = Some("codex".to_owned());

        assert_eq!(
            one.shown(Form::Short),
            "[bob/api · a \\u{b7} own\\u{5d} \\u{5b}b · peer] available since \
             2027-01-15T08:00:00Z (codex): done\\n\\u{5b}acme/ada/x \\u{b7} y \\u{b7} own\\u{5d} go"
        );
        let full = one.shown(Form::Full);
        assert!(
            full.starts_with("acme/bob/api@skald.example available since "),
            "{full}"
        );
        assert!(full.contains(&laptop.to_string()), "{full}");
        assert!(!full.contains('\n'), "{full}");

        let past = listed("api", laptop, AgentState::Available, (1_800_000_900, true));
        assert_eq!(
            past.shown(Form::Short),
            "[bob/api · bob-laptop · peer] offline, last seen 2027-01-15T08:15:00Z"
        );
        let object: serde_json::Value = serde_json::from_str(&past.shown(Form::Json)).unwrap();
        assert_eq!(object["state"], "offline");
        assert_eq!(object["last_seen"], 1_800_000_900);
        assert_eq!(object["duplicate"], false);
    }

    /// `own` lines come first, then by address and device name.
    #[test]
    fn own_agents_come_first() {
        let mut own = listed(
            "zeta",
            device(),
            AgentState::Available,
            (1_800_000_900, false),
        );
        own.level = TrustLevel::Own;
        let peer = listed(
            "alpha",
            device(),
            AgentState::Available,
            (1_800_000_900, false),
        );
        let mut service = listed(
            "beta",
            device(),
            AgentState::Available,
            (1_800_000_900, false),
        );
        service.level = TrustLevel::Service;

        let order: Vec<String> = sorted(vec![service, peer, own])
            .iter()
            .map(|one| one.agent.agent().to_string())
            .collect();
        assert_eq!(order, ["zeta", "alpha", "beta"]);
    }
}
