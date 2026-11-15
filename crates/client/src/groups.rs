//! A channel's MLS group on this device (`docs/protocol/membership.md`, "Admitted devices",
//! "Creating a channel", "Commit ordering" and "Group changes").
//!
//! - A device admitted to a channel that has no group on the server creates it: it adds each other
//!   admitted device it gets a KeyPackage for that passes the checks of "KeyPackages", and commits,
//!   also with nothing to add.
//! - It goes through the Welcomes and Commits of its mailbox in order: it joins from a Welcome only
//!   if its signer is admitted, and processes another device's Commit, warning of each device it
//!   adds that is not admitted once it accepted the space's latest revisions.
//! - It commits the Remove of each member that is not admitted, and, when it signed the space's
//!   last accepted revision, the Adds of the admitted devices the group lacks.
//! - Outside a group the server has, it joins by itself from the GroupInfo of the current epoch
//!   once its signer and members are admitted, unless it lists the device and a Welcome comes. A
//!   device whose Welcome it refused or whose Commit it could not process rejoins from a GroupInfo
//!   of a later epoch, removing its old leaf; one a Commit removed while admitted warns and rejoins.
//!   A Commit of an epoch it has passed is a replay, dropped.
//! - Its own Commit goes into the outbox with the state that holds it pending, all at once, before
//!   it is sent, and is sent again with the same bytes until the server answers. After `stored` it
//!   merges the Commit, uploads the new epoch's GroupInfo, then sends the Welcome to the devices it
//!   added, which goes through the outbox too; after a refusal it drops the Commit, and a group
//!   whose first Commit the server refused, as it kept another, waits for that one's Welcome. A
//!   member uploads the GroupInfo when the server has none of its current epoch.
//! - It decrypts each application message of its mailbox in its channel's group in that order,
//!   among the Welcomes and Commits, and keeps what that gave with the entry, sealed under the
//!   state key, until a run shows it; a replay gives nothing to show.
//! - It encrypts a channel message in the channel's group only while every member device is
//!   admitted, no Commit of its own there waits for the server and another device is in it; the
//!   message goes into the outbox with the state, and with its content's `counter`, the device's
//!   next in the channel.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::num::NonZeroU64;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{
    ChannelView, Joining, MemberDevice, MlsContent, MlsDevice, MlsError, MlsFraming, Processed,
    SealedState, StateKey, check_key_package,
};
use skald_proto::{
    Cbor as _, Cert, Channel, ChannelId, Content, DeviceId, Envelope, Frame, Header, MessageId,
    NonEmpty, Push, RawEnvelope, Roster, Scheme, SpaceId, Unknown,
};

use crate::connection::{Method, WebSocket};
use crate::device::Keys;
use crate::devices::text;
use crate::follow::SpaceDevices;
use crate::key_packages;
use crate::send::{self, Answer, Key, Wire};
use crate::spaces::{Follower, Scope, now};
use crate::state::{self, Counted, MlsChanges, MlsOutgoing, Outgoing, State};

/// The problem of a Commit of an epoch that is not the channel's current one.
const EPOCH_MISMATCH: &str = "epoch-mismatch";

/// The spaces a run synced: each one's last accepted revision and its accounts' devices.
pub type Rosters = BTreeMap<SpaceId, (Roster, SpaceDevices)>;

/// What a run sends the device's Commits and Welcomes with: its keys, and its connection once open.
pub struct Link {
    keys: Keys,
    socket: Option<WebSocket>,
}

impl Link {
    pub fn new(follower: &Follower) -> Result<Self, String> {
        Ok(Link {
            keys: follower.keys()?,
            socket: None,
        })
    }
}

/// A Commit or Welcome of the outbox, as it goes out.
struct Out {
    outgoing: MlsOutgoing,
    frame: Vec<u8>,
}

impl Wire for Out {
    fn key(&self) -> Key {
        (self.outgoing.msg_id, None)
    }

    fn frame(&self) -> &[u8] {
        &self.frame
    }

    // The step that acts on the answer takes it out of the outbox.
    fn answered(&self, _: &State) {}
}

/// Runs `step` on the device's MLS state, then saves the state with the `MlsChanges` it gives, all
/// at once.
fn step<T>(
    follower: &Follower,
    keys: &Keys,
    step: impl FnOnce(&mut MlsDevice, &mut MlsChanges) -> Result<T, String>,
) -> Result<T, String> {
    follower.state.mls_change(|sealed| {
        let mut device = load(follower, keys, sealed)?;
        let mut changes = MlsChanges::default();
        let done = step(&mut device, &mut changes)?;
        Ok((Some(device.seal()), changes, done))
    })
}

/// Writes `changes` to the outbox and the inbox, leaving the MLS state as it is.
fn change(follower: &Follower, changes: MlsChanges) -> Result<(), String> {
    follower.state.mls_change(|_| Ok((None, changes, ())))
}

/// What `read` gives of the device's MLS state, which it leaves as it is.
fn read<T>(
    follower: &Follower,
    keys: &Keys,
    read: impl FnOnce(&MlsDevice) -> T,
) -> Result<T, String> {
    follower
        .state
        .mls_step(|sealed| Ok((None, read(&load(follower, keys, sealed)?))))
}

/// The device of `keys` with its MLS state `sealed`, under the state key in its secrets.
fn load(follower: &Follower, keys: &Keys, sealed: &SealedState) -> Result<MlsDevice, String> {
    Ok(load_keyed(follower, keys, sealed)?.0)
}

/// As [`load`], with the state key.
fn load_keyed(
    follower: &Follower,
    keys: &Keys,
    sealed: &SealedState,
) -> Result<(MlsDevice, StateKey), String> {
    let state_key = key_packages::state_key(&follower.secrets, sealed)?;
    let device = MlsDevice::load(keys.signing(), &state_key, sealed)
        .map_err(|err| format!("this device's MLS state does not load: {err}"))?;
    Ok((device, state_key))
}

/// The member devices of `channel`'s group as the device holds it; none if it holds no group.
pub fn members(
    follower: &Follower,
    keys: &Keys,
    channel: ChannelId,
) -> Result<Option<Vec<MemberDevice>>, String> {
    read(follower, keys, |device| device.members(channel).ok())
}

/// The member devices of the group of each of `channels` the device holds.
pub fn held(
    follower: &Follower,
    keys: &Keys,
    channels: &[Channel],
) -> Result<BTreeMap<ChannelId, Vec<MemberDevice>>, String> {
    read(follower, keys, |device| {
        channels
            .iter()
            .filter_map(|channel| Some((channel.id, device.members(channel.id).ok()?)))
            .collect()
    })
}

/// The devices admitted to `channel` in `roster`, the space's last accepted revision, with
/// `devices`, its accounts' devices: this one, if it is, and the others, each with its valid
/// certificate.
pub fn admitted(
    follower: &Follower,
    roster: &Roster,
    devices: &SpaceDevices,
    channel: ChannelId,
) -> BTreeMap<DeviceId, Cert> {
    let mut admitted: BTreeMap<DeviceId, Cert> =
        send::admitted(&follower.own, follower.this, roster, devices, channel)
            .into_iter()
            .map(|(device, cert)| (device, cert.clone()))
            .collect();
    let body = roster.body();
    let own = body.member(follower.own.account);
    let listed = body
        .channels
        .iter()
        .find(|listed| listed.id == channel)
        .is_some_and(|listed| {
            listed
                .members
                .iter()
                .any(|entry| entry.account == follower.own.account)
        });
    let excluded = own
        .and_then(|member| member.excluded.as_ref())
        .is_some_and(|excluded| excluded.contains(&follower.this));
    if own.is_some() && listed && !excluded {
        admitted.insert(follower.this, follower.cert.clone());
    }
    admitted
}

/// Whether `member`, a leaf of a group, is one of `admitted`, with its certificate's signing key.
fn admits(admitted: &BTreeMap<DeviceId, Cert>, member: &MemberDevice) -> bool {
    admitted
        .get(&member.device)
        .is_some_and(|cert| cert.body().signing == member.signing)
}

/// Sends the device's own Commits and Welcomes that wait in the outbox within `scope`, in order,
/// and acts on each answer; stops at the first the server has not answered in time.
pub fn flush(follower: &mut Follower, link: &mut Link, scope: Scope) -> Result<(), String> {
    let waiting = follower.state.mls_outbox().map_err(text)?;
    for outgoing in waiting
        .into_iter()
        .filter(|outgoing| !scope.excludes(outgoing.space))
    {
        if !send_out(follower, link, outgoing)? {
            break;
        }
    }
    Ok(())
}

/// Sends `outgoing`, a Commit or Welcome of the outbox, and acts on the answer; gives whether the
/// server answered. Before a Welcome, the device uploads the GroupInfo if the server has none of
/// its current epoch, as the Welcome's group may be new.
fn send_out(
    follower: &mut Follower,
    link: &mut Link,
    outgoing: MlsOutgoing,
) -> Result<bool, String> {
    let envelope = RawEnvelope::from_bytes(outgoing.envelope.clone())
        .map_err(|err| format!("the local state is malformed: an envelope in the outbox: {err}"))?;
    let payload = &envelope.envelope().payload;
    let welcome = MlsFraming::read(payload) == Some(MlsFraming::Welcome);
    let external = MlsFraming::is_external_commit(payload);
    let (space, channel) = (outgoing.space, outgoing.channel);
    if welcome && let Err(err) = keep_group_info(follower, link, space, channel) {
        eprintln!("skald: warning: {err}");
    }
    let out = Out {
        frame: Frame::Send {
            envelope,
            report: false,
        }
        .to_cbor(),
        outgoing,
    };
    let mut delivered = send::deliver(
        follower,
        std::slice::from_ref(&out),
        follower.timeout,
        link.socket.take(),
    );
    link.socket = delivered.socket.take();
    let what = if welcome { "Welcome" } else { "Commit" };
    let msg_id = out.outgoing.msg_id;
    let Some(answer) = delivered.answer(&out.key()).cloned() else {
        let why = delivered
            .failed()
            .map_or_else(String::new, |err| format!(" ({err})"));
        eprintln!(
            "skald: warning: the server has not answered this device's {what} in the channel \
             {channel}{why}; the outbox keeps it, and the next run sends it again"
        );
        return Ok(false);
    };
    if welcome {
        let answered = vec![msg_id];
        change(
            follower,
            MlsChanges {
                answered,
                ..MlsChanges::default()
            },
        )?;
        if let Answer::Refused(problem) = answer {
            eprintln!(
                "skald: warning: the server refused this device's Welcome in the channel \
                 {channel}: {problem}; the devices it adds join by themselves"
            );
        }
        return Ok(true);
    }
    match answer {
        Answer::Stored => committed(follower, link, (space, channel), msg_id),
        Answer::Refused(problem) => {
            step(follower, &link.keys, |device, changes| {
                changes.answered.push(msg_id);
                // A device's external Commit is applied at once: it drops the group it joined.
                let refused = if external {
                    device.external_commit_refused(channel)
                } else {
                    device.commit_refused(channel)
                };
                match refused {
                    Ok(()) | Err(MlsError::NoCommitPending(_) | MlsError::NoGroup(_)) => Ok(()),
                    Err(err) => Err(failed(channel, err)),
                }
            })?;
            if problem != EPOCH_MISMATCH {
                eprintln!(
                    "skald: warning: the server refused this device's Commit in the channel \
                     {channel}: {problem}"
                );
            }
            Ok(true)
        }
    }
}

/// The server stored the device's Commit `msg_id` in `channel`: the device merges it, uploads the
/// new epoch's GroupInfo and sends the Commit's Welcome, if any, to the devices it added.
fn committed(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    msg_id: MessageId,
) -> Result<bool, String> {
    let version = follower.connection.version();
    let welcome = step(follower, &link.keys, |device, changes| {
        changes.answered.push(msg_id);
        let devices = |device: &MlsDevice| -> Result<Vec<DeviceId>, MlsError> {
            let members = device.members(channel)?;
            Ok(members.iter().map(|member| member.device).collect())
        };
        // A group the device dropped meanwhile has nothing to merge.
        let Ok(before) = devices(device) else {
            return Ok(None);
        };
        // The device is in the group of the channel's current epoch: an external Commit of its own
        // was applied at once, another one merges now.
        changes.rejoined.push(channel);
        let welcome = match device.commit_stored(channel) {
            Ok(welcome) => welcome,
            Err(MlsError::NoCommitPending(_)) => return Ok(None),
            Err(err) => return Err(failed(channel, err)),
        };
        let added = (devices(device)
            .map_err(|err| failed(channel, err))?
            .into_iter())
        .filter(|member| !before.contains(member))
        .collect();
        let queued = welcome
            .zip(NonEmpty::from_vec(added))
            .map(|(welcome, added)| outgoing(version, (space, channel), &welcome, Some(added)));
        changes.queued.extend(queued.clone());
        Ok(queued)
    })?;
    if let Err(err) = upload_group_info(follower, link, space, channel) {
        eprintln!("skald: warning: {err}");
    }
    match welcome {
        Some(welcome) => send_out(follower, link, welcome),
        None => Ok(true),
    }
}

/// A new envelope of `channel` in `space`, of protocol version `version`, with `payload`, an MLS
/// message, to `recipients`, none for the channel, as it goes into the outbox.
fn outgoing(
    version: std::num::NonZeroU64,
    (space, channel): (SpaceId, ChannelId),
    payload: &[u8],
    recipients: Option<NonEmpty<DeviceId>>,
) -> MlsOutgoing {
    let msg_id = MessageId::generate();
    let header = Header {
        v: version,
        space,
        channel,
        msg_id,
        scheme: Scheme::Mls,
        recipients,
        ttl: None,
        unknown: Unknown::default(),
    };
    MlsOutgoing {
        msg_id,
        space,
        channel,
        envelope: Envelope {
            header,
            payload: payload.to_vec(),
        }
        .to_cbor(),
    }
}

/// Why an MLS step in `channel` failed, by `err`.
fn failed(channel: ChannelId, err: impl Display) -> String {
    format!("the group of the channel {channel} fails: {err}")
}

/// Commits in `channel` of `space` what `make` makes in its group, writing the changes it gives
/// with it, and sends it.
fn commit(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    make: impl FnOnce(&mut MlsDevice, &mut MlsChanges) -> Result<Vec<u8>, MlsError>,
) -> Result<(), String> {
    let version = follower.connection.version();
    let queued = step(follower, &link.keys, |device, changes| {
        let commit = make(device, changes).map_err(|err| failed(channel, err))?;
        let queued = outgoing(version, (space, channel), &commit, None);
        changes.queued.push(queued.clone());
        Ok(queued)
    })?;
    send_out(follower, link, queued).map(drop)
}

/// `ChannelGroup` in the API spec.
#[derive(Deserialize)]
struct ChannelGroup {
    group: bool,
    group_info: Option<String>,
}

/// The path of `channel`'s GroupInfo in `space`.
fn group_info_path(space: SpaceId, channel: ChannelId) -> String {
    format!("/spaces/{space}/channels/{channel}/group-info")
}

/// Whether `channel` of `space` has a group on the server, and its GroupInfo of the current epoch,
/// if the server has one.
fn group_on_server(
    follower: &mut Follower,
    space: SpaceId,
    channel: ChannelId,
) -> Result<(bool, Option<Vec<u8>>), String> {
    let body = follower
        .connection
        .get_as(&group_info_path(space, channel), &follower.token)
        .map_err(text)?;
    let unexpected = || "unexpected answer from the server: a channel's group".to_owned();
    let given: ChannelGroup = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    let group_info = (given.group_info)
        .map(|text| URL_SAFE_NO_PAD.decode(text).map_err(|_| unexpected()))
        .transpose()?;
    Ok((given.group, group_info))
}

/// Whether `channel` of `space` has a group on the server, and the epoch of its GroupInfo, if the
/// server has one.
fn on_server(
    follower: &mut Follower,
    space: SpaceId,
    channel: ChannelId,
) -> Result<(bool, Option<u64>), String> {
    let (group, group_info) = group_on_server(follower, space, channel)?;
    let epoch = group_info.and_then(|bytes| {
        ChannelView::observe(channel, &bytes)
            .ok()
            .map(|view| view.epoch())
    });
    Ok((group, epoch))
}

/// Uploads the GroupInfo of `channel`'s group in the device's current epoch.
fn upload_group_info(
    follower: &mut Follower,
    link: &Link,
    space: SpaceId,
    channel: ChannelId,
) -> Result<(), String> {
    let group_info = read(follower, &link.keys, |device| device.group_info(channel))?
        .map_err(|err| failed(channel, err))?;
    let request =
        serde_json::json!({ "group_info": URL_SAFE_NO_PAD.encode(group_info) }).to_string();
    follower
        .connection
        .send_json_as(
            Method::Put,
            &group_info_path(space, channel),
            &follower.token,
            request.as_bytes(),
        )
        .map(drop)
        .map_err(|err| {
            format!(
                "the GroupInfo of the channel {channel} is not uploaded: {}",
                text(err)
            )
        })
}

/// Uploads the GroupInfo of `channel`'s group, which the device holds, if the server has none of
/// its current epoch.
fn keep_group_info(
    follower: &mut Follower,
    link: &Link,
    space: SpaceId,
    channel: ChannelId,
) -> Result<(), String> {
    let Ok(epoch) = read(follower, &link.keys, |device| device.epoch(channel))? else {
        return Ok(());
    };
    let (_, given) = on_server(follower, space, channel)
        .map_err(|err| format!("the GroupInfo of the channel {channel} is not checked: {err}"))?;
    if given.is_some_and(|given| given >= epoch) {
        return Ok(());
    }
    upload_group_info(follower, link, space, channel)
}

/// `KeyPackageRequest` and `HandedOutKeyPackages` in the API spec.
#[derive(Deserialize)]
struct Handed {
    key_packages: Vec<HandedOut>,
}

#[derive(Deserialize)]
struct HandedOut {
    device: String,
    key_package: String,
}

/// A KeyPackage the server hands out for each of `wanted`, of those it has one for, each with the
/// valid certificate the caller gives; those that fail the checks of "KeyPackages" against their
/// device's certificate are warned of and left out.
fn key_packages(
    follower: &mut Follower,
    wanted: &BTreeMap<DeviceId, Cert>,
) -> Result<Vec<Vec<u8>>, String> {
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let devices: Vec<String> = wanted.keys().map(ToString::to_string).collect();
    let request = serde_json::json!({ "devices": devices }).to_string();
    let body = follower
        .connection
        .send_json_as(
            Method::Post,
            "/key-packages",
            &follower.token,
            request.as_bytes(),
        )
        .map_err(text)?;
    let unexpected = || "unexpected answer from the server: KeyPackages".to_owned();
    let handed: Handed = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    let mut taken = BTreeMap::new();
    for entry in handed.key_packages {
        let device: DeviceId = entry.device.parse().map_err(|_| unexpected())?;
        let Some(cert) = wanted.get(&device) else {
            continue;
        };
        let package = URL_SAFE_NO_PAD
            .decode(&entry.key_package)
            .map_err(|_| unexpected())?;
        match check_key_package(&package, cert, now()) {
            Ok(_) => {
                taken.entry(device).or_insert(package);
            }
            Err(err) => eprintln!(
                "skald: warning: the server handed out a KeyPackage of the device {device} that \
                 fails its checks: {err}; this device does not add it"
            ),
        }
    }
    Ok(taken.into_values().collect())
}

/// Brings in line the group of each channel of `roster`, the last accepted revision of the space
/// `space`, with `devices`, its accounts' devices, that this device is admitted to; only `only`, if
/// given. A channel whose Commit of this device waits for the server is left as it is. Warns of a
/// group it fails to bring in line.
pub fn align(
    follower: &mut Follower,
    link: &mut Link,
    space: SpaceId,
    (roster, devices): (&Roster, &SpaceDevices),
    only: Option<ChannelId>,
) -> Result<(), String> {
    let body = roster.body();
    let waiting = follower.state.mls_outbox().map_err(text)?;
    let rejoining = follower.state.rejoining().map_err(text)?;
    let welcomes = welcomes_waiting(follower)?;
    let held = held(follower, &link.keys, &body.channels)?;
    for channel in &body.channels {
        let id = channel.id;
        let mut admitted = admitted(follower, roster, devices, id);
        if only.is_some_and(|only| only != id)
            || waiting.iter().any(|outgoing| outgoing.channel == id)
            || admitted.remove(&follower.this).is_none()
        {
            continue;
        }
        let at = (space, id);
        let aligned = match held.get(&id) {
            // A Welcome in the mailbox comes first.
            None if welcomes.contains(&id) => Ok(()),
            None => outside(follower, link, at, &admitted, rejoining.contains(&id)),
            Some(_) if rejoining.contains(&id) => behind(follower, link, at, &admitted),
            Some(members) => keep_in_line(follower, link, at, (roster, &admitted), members),
        };
        if let Err(err) = aligned {
            eprintln!(
                "skald: warning: the group of the channel {id} in the space {} is not brought in \
                 line: {err}",
                body.name.as_str()
            );
        }
    }
    Ok(())
}

/// The channels of the Welcomes in the mailbox that no run has taken in yet.
fn welcomes_waiting(follower: &Follower) -> Result<Vec<ChannelId>, String> {
    let inbox = follower.state.inbox().map_err(text)?;
    Ok((inbox.iter())
        .filter_map(|entry| match classify(&entry.frame)? {
            (envelope, Kind::Welcome) => Some(envelope.header.channel),
            _ => None,
        })
        .collect())
}

/// Brings this device, admitted to `channel` of `space` but outside its group, into it, with
/// `others`, the other admitted devices: creates the group if the server has none; otherwise joins
/// by itself from the GroupInfo of the current epoch, if the server has one, unless it lists this
/// device, which another device added and whose Welcome comes, or `rejoining`.
fn outside(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    others: &BTreeMap<DeviceId, Cert>,
    rejoining: bool,
) -> Result<(), String> {
    let (group, group_info) = group_on_server(follower, space, channel)?;
    if !group {
        return create(follower, link, (space, channel), others);
    }
    let Some(group_info) = group_info else {
        return Ok(());
    };
    let view = observe(channel, &group_info)?;
    if !rejoining
        && view
            .members()
            .iter()
            .any(|member| member.device == follower.this)
    {
        return Ok(());
    }
    join_by_itself(
        follower,
        link,
        (space, channel),
        (&view, &group_info),
        others,
    )
}

/// Rejoins `channel` of `space`, whose group this device holds but whose Commit it could not
/// process or whose Welcome it refused, from the GroupInfo of the current epoch, if the server has
/// one of an epoch past the device's; with `others`, the other admitted devices.
fn behind(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    others: &BTreeMap<DeviceId, Cert>,
) -> Result<(), String> {
    let epoch = read(follower, &link.keys, |device| device.epoch(channel))?
        .map_err(|err| failed(channel, err))?;
    let Some(group_info) = group_on_server(follower, space, channel)?.1 else {
        return Ok(());
    };
    let view = observe(channel, &group_info)?;
    if view.epoch() > epoch {
        return join_by_itself(
            follower,
            link,
            (space, channel),
            (&view, &group_info),
            others,
        );
    }
    // The device is in the current epoch: what it could not process was not the channel's.
    change(
        follower,
        MlsChanges {
            rejoined: vec![channel],
            ..MlsChanges::default()
        },
    )
}

/// `channel`'s GroupInfo `group_info`, the server's, as seen from outside the group.
fn observe(channel: ChannelId, group_info: &[u8]) -> Result<ChannelView, String> {
    ChannelView::observe_joinable(channel, group_info).map_err(|err| {
        format!("the server gave a GroupInfo of the channel {channel} that does not check: {err}")
    })
}

/// Of `signer` and `members`, a GroupInfo's, those that are not of `others`, the other admitted
/// devices, nor this device, `this`, with its own key `signing`.
fn not_admitted(
    (this, signing): (DeviceId, &[u8; 32]),
    others: &BTreeMap<DeviceId, Cert>,
    (signer, members): (&MemberDevice, &[MemberDevice]),
) -> BTreeSet<DeviceId> {
    (members.iter().chain([signer]))
        .filter(|member| {
            let own = member.device == this && member.signing == *signing;
            !own && !admits(others, member)
        })
        .map(|member| member.device)
        .collect()
}

/// Joins `channel` of `space` by itself from `group_info`, seen as `view`, if its signer and every
/// member it lists are admitted, with `others`, the other admitted devices, once the device accepted
/// the space's latest revisions if one is not; warns naming those that are not. The external
/// Commit goes through the outbox as the device's other Commits do.
fn join_by_itself(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    (view, group_info): (&ChannelView, &[u8]),
    others: &BTreeMap<DeviceId, Cert>,
) -> Result<(), String> {
    let own = (follower.this, &link.keys.signing().public_key());
    let listed = (&view.signer(), view.members());
    let mut admitted = others.clone();
    if !not_admitted(own, &admitted, listed).is_empty() {
        // Before it holds them not admitted, the device accepts the space's latest revisions.
        let synced = follower.state.spaces().map_err(text)?;
        if let Some(held) = synced.iter().find(|held| held.id == space) {
            let (roster, devices) = follower.sync(held)?;
            admitted = self::admitted(follower, &roster, &devices, channel);
            admitted.remove(&follower.this);
        }
    }
    let strangers = not_admitted(own, &admitted, listed);
    if !strangers.is_empty() {
        let strangers: Vec<String> = strangers.iter().map(ToString::to_string).collect();
        return Err(format!(
            "this device does not join the channel's group by itself: the server's GroupInfo of \
             the channel {channel} lists the devices {}, which are not admitted to the channel",
            strangers.join(", ")
        ));
    }
    // Gap detection counts the channel's `mls` messages from their baseline again: those after the
    // cursor.
    let cursor = follower.state.cursor().map_err(text)?;
    commit(follower, link, (space, channel), |device, changes| {
        changes.joined.push((channel, cursor + 1));
        device.join_by_itself(channel, group_info, now(), |joining| {
            let listed = (&joining.signer, joining.members.as_slice());
            joining.channel == channel && not_admitted(own, &admitted, listed).is_empty()
        })
    })
}

/// Creates the group of `channel` in `space`, which has none on the server, with each of `others`,
/// the other admitted devices, it gets a valid KeyPackage for.
fn create(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    others: &BTreeMap<DeviceId, Cert>,
) -> Result<(), String> {
    let packages = key_packages(follower, others)?;
    let certs: Vec<Cert> = others.values().cloned().collect();
    commit(follower, link, (space, channel), |device, _| {
        device.create_group(channel, now())?;
        device.add_devices(channel, &packages, &certs, now())
    })
}

/// Commits the Remove of each of `members`, the group of `channel`, that is not admitted; with
/// none, and if this device signed `roster`, the Adds of the devices of `others`, the other
/// admitted devices, that the group lacks.
fn keep_in_line(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    (roster, others): (&Roster, &BTreeMap<DeviceId, Cert>),
    members: &[MemberDevice],
) -> Result<(), String> {
    let this = follower.this;
    let strangers: Vec<DeviceId> = members
        .iter()
        .filter(|member| member.device != this && !admits(others, member))
        .map(|member| member.device)
        .collect();
    if !strangers.is_empty() {
        return commit(follower, link, (space, channel), |device, _| {
            device.remove_devices(channel, &strangers, b"", now())
        });
    }
    if roster.body().signer != this {
        return Ok(());
    }
    let lacking: BTreeMap<DeviceId, Cert> = others
        .iter()
        .filter(|(device, _)| !members.iter().any(|member| member.device == **device))
        .map(|(device, cert)| (*device, cert.clone()))
        .collect();
    let packages = key_packages(follower, &lacking)?;
    if packages.is_empty() {
        return Ok(());
    }
    let certs: Vec<Cert> = lacking.into_values().collect();
    commit(follower, link, (space, channel), |device, _| {
        device.add_devices(channel, &packages, &certs, now())
    })
}

/// Of `added`, the devices a Commit in `channel` added, those not admitted to it in the space as
/// `synced` holds it.
fn strangers(
    follower: &Follower,
    (roster, devices): &(Roster, SpaceDevices),
    channel: ChannelId,
    added: &[MemberDevice],
) -> Vec<DeviceId> {
    let admitted = admitted(follower, roster, devices, channel);
    (added.iter())
        .filter(|member| !admits(&admitted, member))
        .map(|member| member.device)
        .collect()
}

/// A Welcome, Commit or application message in the mailbox.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Welcome,
    Commit,
    Application,
}

/// The envelope of `entry`, a mailbox entry as its frame, if it holds a Welcome, a Commit or an
/// application message under `mls`.
fn classify(entry: &[u8]) -> Option<(Envelope, Kind)> {
    let Ok(Frame::Push(Push::Message { envelope, .. })) = Frame::from_cbor(entry) else {
        return None;
    };
    let envelope = envelope.envelope().clone();
    if envelope.header.scheme != Scheme::Mls {
        return None;
    }
    let kind = match MlsFraming::read(&envelope.payload)? {
        MlsFraming::Welcome => Kind::Welcome,
        MlsFraming::Message {
            public: true,
            content: MlsContent::Commit,
            ..
        } => Kind::Commit,
        MlsFraming::Message {
            content: MlsContent::Application,
            ..
        } => Kind::Application,
        _ => return None,
    };
    Some((envelope, kind))
}

/// Whether `entry`, a mailbox entry as its frame, holds a Welcome or a Commit.
pub fn is_handshake(entry: &[u8]) -> bool {
    classify(entry).is_some_and(|(_, kind)| kind != Kind::Application)
}

/// Whether `entry`, a mailbox entry as its frame, holds an application message under `mls`.
pub fn is_application(entry: &[u8]) -> bool {
    classify(entry).is_some_and(|(_, kind)| kind == Kind::Application)
}

/// Goes through the Welcomes, Commits and application messages of the mailbox entries the state
/// holds within `scope`, in `seq` order: takes each Welcome and Commit out with the state it
/// leaves, checking them against the spaces as `rosters` holds them, syncing any other at its first
/// entry, and decrypts each application message. One of a space the device does not follow or that
/// fails to sync, or of a channel whose Commit of this device waits for the server, stays for a
/// later run.
pub fn take_in(
    follower: &mut Follower,
    link: &mut Link,
    scope: Scope,
    rosters: &mut Rosters,
) -> Result<(), String> {
    let followed = follower.state.spaces().map_err(text)?;
    let waiting = follower.state.mls_outbox().map_err(text)?;
    for entry in follower.state.inbox().map_err(text)? {
        if entry.opened.is_some() {
            continue;
        }
        let seq = entry.seq;
        let Some((envelope, kind)) = classify(&entry.frame) else {
            continue;
        };
        let header = &envelope.header;
        if scope.excludes(header.space)
            || waiting
                .iter()
                .any(|outgoing| outgoing.channel == header.channel)
        {
            continue;
        }
        // One of a space the device does not follow yet waits until it does.
        let Some(space) = followed.iter().find(|space| space.id == header.space) else {
            continue;
        };
        if kind == Kind::Application {
            open(follower, link, seq, &envelope)?;
            continue;
        }
        if let Entry::Vacant(vacant) = rosters.entry(space.id) {
            match follower.sync(space) {
                Ok(done) => {
                    vacant.insert(done);
                }
                Err(err) => {
                    eprintln!("skald: warning: {err}");
                    continue;
                }
            }
        }
        let taking = Taking {
            seq,
            space,
            envelope: &envelope,
        };
        if kind == Kind::Welcome {
            welcomed(follower, link, &taking, rosters)?;
        } else {
            processed(follower, link, &taking, rosters)?;
        }
    }
    Ok(())
}

/// A Welcome or Commit the device takes in: its entry's `seq`, its space and its envelope.
#[derive(Clone, Copy)]
struct Taking<'t> {
    seq: u64,
    space: &'t state::Space,
    envelope: &'t Envelope,
}

/// Joins a channel's group from the Welcome `taking` holds, if its signer is admitted to the
/// channel, and warns of each member it lists that is not; warns of a Welcome it refuses. Before it
/// holds the signer or a member not admitted, the device accepts the space's latest revisions, and
/// joins with them.
fn welcomed(
    follower: &mut Follower,
    link: &mut Link,
    taking: &Taking,
    rosters: &mut Rosters,
) -> Result<(), String> {
    let Taking {
        seq,
        space,
        envelope,
    } = *taking;
    let channel = envelope.header.channel;
    let mut admitted = admitted_in(follower, rosters, (space.id, channel));
    let mut synced = false;
    let (joined, seen) = match join(follower, link, (seq, envelope), &admitted, false)? {
        Some(done) => done,
        None => {
            synced = true;
            sync_again(follower, space, rosters);
            admitted = admitted_in(follower, rosters, (space.id, channel));
            join(follower, link, (seq, envelope), &admitted, true)?
                .expect("a last attempt is written")
        }
    };
    let strangers = |admitted: &BTreeMap<DeviceId, Cert>| -> Vec<DeviceId> {
        let members = seen.iter().flat_map(|joining| &joining.members);
        (members.filter(|member| !admits(admitted, member)))
            .map(|member| member.device)
            .collect()
    };
    if joined.is_ok() && !synced && !strangers(&admitted).is_empty() {
        sync_again(follower, space, rosters);
        admitted = admitted_in(follower, rosters, (space.id, channel));
    }
    let name = &space.name;
    match (joined, &seen) {
        (Ok(_), _) => {
            for device in strangers(&admitted) {
                eprintln!(
                    "skald: warning: the group of the channel {channel} in the space {name} that \
                     this device joined holds the device {device}, which is not admitted to the \
                     channel; this device removes it"
                );
            }
        }
        (Err(MlsError::Refused), Some(joining)) => eprintln!(
            "skald: warning: this device refused a Welcome to the channel {channel} in the space \
             {name}: its signer, the device {}, is not admitted to the channel",
            joining.signer.device
        ),
        (Err(MlsError::GroupExists(_)), _) => {}
        (Err(err), _) => eprintln!(
            "skald: warning: this device cannot join the channel {channel} in the space {name} \
             from a Welcome: {err}"
        ),
    }
    Ok(())
}

/// The devices admitted to `channel` of `space` as `rosters` holds it.
fn admitted_in(
    follower: &Follower,
    rosters: &Rosters,
    (space, channel): (SpaceId, ChannelId),
) -> BTreeMap<DeviceId, Cert> {
    let (roster, devices) = &rosters[&space];
    admitted(follower, roster, devices, channel)
}

/// What joining from a Welcome gave, and the group the Welcome shows, if it reached the check.
type Joined = (Result<ChannelId, MlsError>, Option<Joining>);

/// Joins from the Welcome in `envelope`, the entry `seq`, if its signer is one of `admitted`, and
/// takes the entry out; gives what joining gave and the group the Welcome shows. Unless `last`, a
/// Welcome refused for its signer writes nothing and gives `None`, so it can be joined again.
fn join(
    follower: &Follower,
    link: &Link,
    (seq, envelope): (u64, &Envelope),
    admitted: &BTreeMap<DeviceId, Cert>,
    last: bool,
) -> Result<Option<Joined>, String> {
    let channel = envelope.header.channel;
    let mut seen: Option<Joining> = None;
    let joined = follower.state.mls_change(|sealed| {
        let mut device = load(follower, &link.keys, sealed)?;
        let joined = device.join(&envelope.payload, now(), |joining| {
            seen = Some(joining.clone());
            joining.channel == channel && admits(admitted, &joining.signer)
        });
        if !last && matches!(joined, Err(MlsError::Refused)) {
            return Ok((None, MlsChanges::default(), None));
        }
        // Gap detection counts the channel's `mls` messages from their baseline again.
        let mut taken = MlsChanges {
            taken: vec![seq],
            joined: joined.iter().map(|&channel| (channel, seq)).collect(),
            ..MlsChanges::default()
        };
        match joined {
            Ok(_) => taken.rejoined.push(channel),
            // A refused Welcome leaves a leaf in the group that the device cannot use: it joins by
            // itself even though the GroupInfo lists it.
            Err(MlsError::GroupExists(_)) => {}
            Err(_) => taken.rejoin.push(channel),
        }
        Ok((Some(device.seal()), taken, Some(joined)))
    })?;
    Ok(joined.map(|joined| (joined, seen)))
}

/// Accepts the latest revisions of `space` into `rosters`; warns if it cannot.
fn sync_again(follower: &mut Follower, space: &state::Space, rosters: &mut Rosters) {
    match follower.sync(space) {
        Ok(done) => {
            rosters.insert(space.id, done);
        }
        Err(err) => eprintln!("skald: warning: {err}"),
    }
}

/// Processes another device's Commit that `taking` holds. Of each device it adds that is not
/// admitted, the device warns once it accepted the space's latest revisions; it then uploads the
/// GroupInfo if the server has none of the new epoch.
fn processed(
    follower: &mut Follower,
    link: &mut Link,
    taking: &Taking,
    rosters: &mut Rosters,
) -> Result<(), String> {
    let (seq, space, envelope) = (taking.seq, taking.space, taking.envelope);
    let channel = envelope.header.channel;
    let of_epoch = match MlsFraming::read(&envelope.payload) {
        Some(MlsFraming::Message { epoch, .. }) => epoch,
        _ => 0,
    };
    let mut seen = None;
    let result = step(follower, &link.keys, |device, changes| {
        changes.taken.push(seq);
        let Ok(epoch) = device.epoch(channel) else {
            // A Commit of a group the device is not in.
            return Ok(None);
        };
        // A Commit of an epoch the device has passed is a replay ("Rejoining").
        if of_epoch < epoch {
            return Ok(None);
        }
        let processed = device.process_commit(channel, &envelope.payload, now(), |change| {
            seen = Some(change.clone());
            true
        });
        if processed.is_err() {
            changes.rejoin.push(channel);
        }
        Ok(Some(processed))
    })?;
    let name = &space.name;
    let change = match (result, seen) {
        (Some(Ok(Processed::Entered { .. })), Some(change)) => change,
        (Some(Ok(Processed::Removed)), _) => {
            removed(follower, space, channel, rosters);
            return Ok(());
        }
        (Some(Err(err)), _) => {
            eprintln!(
                "skald: warning: this device cannot process a Commit in the channel {channel} in \
                 the space {name}: {err}; it rejoins the channel's group"
            );
            return Ok(());
        }
        _ => return Ok(()),
    };
    let mut added = strangers(follower, &rosters[&space.id], channel, &change.added);
    if !added.is_empty() {
        // Before it holds them not admitted, the device accepts the space's latest revisions.
        sync_again(follower, space, rosters);
        added = strangers(follower, &rosters[&space.id], channel, &change.added);
    }
    for device in added {
        if device == change.sender {
            eprintln!(
                "skald: warning: the device {device} joined the channel {channel} in the space \
                 {name} by itself, but it is not admitted to the channel; this device removes it"
            );
        } else {
            eprintln!(
                "skald: warning: a Commit of the device {} in the channel {channel} in the space \
                 {name} added the device {device}, which is not admitted to the channel; this \
                 device removes it",
                change.sender
            );
        }
    }
    if let Err(err) = keep_group_info(follower, link, space.id, channel) {
        eprintln!("skald: warning: {err}");
    }
    Ok(())
}

/// A Commit removed this device from `channel`'s group: it warns if the device is admitted to the
/// channel once it accepted the space's latest revisions, as it then rejoins ("Rejoining").
fn removed(
    follower: &mut Follower,
    space: &state::Space,
    channel: ChannelId,
    rosters: &mut Rosters,
) {
    sync_again(follower, space, rosters);
    if admitted_in(follower, rosters, (space.id, channel)).contains_key(&follower.this) {
        eprintln!(
            "skald: warning: a Commit removed this device from the channel {channel} in the space \
             {}, which admits it; this device rejoins the channel's group",
            space.name
        );
    }
}

/// What decrypting an application message of the mailbox in its channel's group gave, kept with
/// its entry, sealed under the state key, until a run shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The sending device, a member of the group, and the bytes of the content.
    Read(DeviceId, Vec<u8>),
    /// The device had used its key already: a replay.
    Replay,
    /// It did not decrypt, for this reason.
    Failed(String),
}

impl Opened {
    fn to_bytes(&self) -> Vec<u8> {
        match self {
            Opened::Read(sender, content) => [&[0][..], sender.as_bytes(), content].concat(),
            Opened::Replay => vec![1],
            Opened::Failed(why) => [&[2][..], why.as_bytes()].concat(),
        }
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (tag, rest) = bytes.split_first()?;
        match tag {
            0 => {
                let (sender, content) = rest.split_at_checked(16)?;
                let sender = DeviceId::from_bytes(sender.try_into().ok()?);
                Some(Opened::Read(sender, content.to_vec()))
            }
            1 if rest.is_empty() => Some(Opened::Replay),
            2 => Some(Opened::Failed(String::from_utf8(rest.to_vec()).ok()?)),
            _ => None,
        }
    }
}

/// The name the state key seals what decrypting the entry `seq` gave under.
fn opened_item(seq: u64) -> Vec<u8> {
    [b"inbox:".as_slice(), &seq.to_be_bytes()].concat()
}

/// Decrypts the application message in `envelope`, the entry `seq`, in its channel's group, and
/// keeps what that gave with the entry, with the state it leaves.
fn open(follower: &Follower, link: &Link, seq: u64, envelope: &Envelope) -> Result<(), String> {
    let channel = envelope.header.channel;
    follower.state.mls_change(|sealed| {
        let (mut device, state_key) = load_keyed(follower, &link.keys, sealed)?;
        let (opened, saved) = match device.decrypt(channel, &envelope.payload) {
            Ok(read) => (
                Opened::Read(read.sender, read.plaintext),
                Some(device.seal()),
            ),
            Err(MlsError::Replay) => (Opened::Replay, None),
            Err(err) => (
                Opened::Failed(format!("it does not decrypt in the channel's group: {err}")),
                None,
            ),
        };
        let kept = state_key.seal(follower.this, &opened_item(seq), &opened.to_bytes());
        let changes = MlsChanges {
            opened: vec![(seq, kept)],
            ..MlsChanges::default()
        };
        Ok((saved, changes, ()))
    })
}

/// The key that opens what decrypting the entries of the mailbox gave.
pub fn inbox_key(follower: &Follower) -> Result<StateKey, String> {
    follower.state.mls_step(|sealed| {
        let key = key_packages::state_key(&follower.secrets, sealed)?;
        Ok((None, key))
    })
}

/// What decrypting `entry` gave, opened with `key`, once the device decrypted it.
pub fn opened(follower: &Follower, key: &StateKey, entry: &state::Entry) -> Option<Opened> {
    let sealed = entry.opened.as_ref()?;
    let opened = key
        .open(follower.this, &opened_item(entry.seq), sealed)
        .and_then(|bytes| Opened::from_bytes(&bytes));
    Some(opened.unwrap_or_else(|| {
        Opened::Failed("what this device kept of its decryption does not open".to_owned())
    }))
}

/// Encrypts the content `content` gives for a new message ID and the device's next counter in
/// `channel` of `roster`, the space's last accepted revision with `devices`, its accounts'
/// devices, in the channel's group, and puts the `mls` envelope in the outbox with the state it
/// leaves; gives its message ID. Sends nothing, and says why, while a member device of the group
/// is not admitted, while a Commit of this device in the channel waits for the server, or when no
/// other device is in the group.
pub fn encrypt(
    follower: &Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    channel: ChannelId,
    content: impl FnOnce(MessageId, NonZeroU64) -> Content,
) -> Result<MessageId, String> {
    let keys = follower.keys()?;
    let admitted = admitted(follower, roster, devices, channel);
    let waiting = (follower.state.mls_outbox().map_err(text)?)
        .iter()
        .any(|outgoing| outgoing.channel == channel);
    let (space, version) = (roster.body().space, follower.connection.version());
    follower.state.mls_change(|sealed| {
        let mut device = load(follower, &keys, sealed)?;
        let members = device.members(channel).map_err(|err| match err {
            MlsError::NoGroup(_) => {
                format!("this device is in no group of the channel {channel} yet; nothing is sent")
            }
            err => failed(channel, err),
        })?;
        let strangers: Vec<String> = (members.iter())
            .filter(|member| !admits(&admitted, member))
            .map(|member| member.device.to_string())
            .collect();
        if !strangers.is_empty() {
            return Err(format!(
                "the group of the channel {channel} holds the devices {}, which are not admitted \
                 to the channel; nothing is sent there until the server stores this device's \
                 Commit that removes them",
                strangers.join(", ")
            ));
        }
        if waiting {
            return Err(format!(
                "this device's Commit in the channel {channel} waits for the server's answer; \
                 nothing is sent there until the server stores it"
            ));
        }
        if members.iter().all(|member| member.device == follower.this) {
            return Err(format!(
                "no device can receive the message: the group of the channel {channel} holds no \
                 other device; nothing is sent"
            ));
        }
        let last = follower.state.channel_counter(channel).map_err(text)?;
        let counter = last
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or_else(|| format!("the counter in the channel {channel} has no next"))?;
        let msg_id = MessageId::generate();
        let plaintext = content(msg_id, counter).to_cbor();
        let payload = device
            .encrypt(channel, &plaintext)
            .map_err(|err| failed(channel, err))?;
        let header = Header {
            v: version,
            space,
            channel,
            msg_id,
            scheme: Scheme::Mls,
            recipients: None,
            ttl: None,
            unknown: Unknown::default(),
        };
        let message = Counted {
            outgoing: Outgoing {
                msg_id,
                recipient: None,
                space,
                envelope: Envelope { header, payload }.to_cbor(),
            },
            channel,
            counter: counter.get(),
        };
        let changes = MlsChanges {
            messages: vec![message],
            ..MlsChanges::default()
        };
        Ok((Some(device.seal()), changes, msg_id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What decrypting gave comes back as it was kept; bytes of no such form give nothing.
    #[test]
    fn opened_is_kept_as_it_was() {
        let sender = DeviceId::from_signing_key(&[4; 32]);
        for opened in [
            Opened::Read(sender, b"content".to_vec()),
            Opened::Read(sender, Vec::new()),
            Opened::Replay,
            Opened::Failed("it does not decrypt".to_owned()),
        ] {
            assert_eq!(Opened::from_bytes(&opened.to_bytes()), Some(opened));
        }
        for bytes in [&[][..], &[0, 1, 2], &[1, 0], &[2, 0xff], &[3]] {
            assert_eq!(Opened::from_bytes(bytes), None, "{bytes:?}");
        }
    }
}
