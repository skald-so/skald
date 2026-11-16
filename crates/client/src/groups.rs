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
//!   last accepted revision, the Adds of the admitted devices the group lacks. A Commit with the
//!   Remove of a revoked device carries the revocations by which this device holds it revoked; of
//!   those another device's Commit carries, it keeps the ones that count, and warns of the devices
//!   they revoke, before it judges the new epoch.
//! - Its own Commit goes into the outbox with the state that holds it pending, all at once, before
//!   it is sent, and is sent again with the same bytes until the server answers. After `stored` it
//!   merges the Commit, uploads the new epoch's GroupInfo, then sends the Welcome to the devices it
//!   added, which goes through the outbox too; after a refusal it drops the Commit, and a group
//!   whose first Commit the server refused, as it kept another, waits for that one's Welcome. A
//!   member uploads the GroupInfo when the server has none of its current epoch.
//! - It decrypts each application message of its mailbox in its channel's group in that order,
//!   among the Welcomes and Commits, and keeps what that gave with the entry, sealed under the
//!   state key, until a run shows it; a replay gives nothing to show. Once an entry of a channel
//!   stays for a later run, so does each later one there; a message that comes before the device
//!   joins the channel's group from its Welcome waits for it, unless the channel does not admit
//!   the device.
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
use crate::follow::{self, SpaceDevices};
use crate::key_packages;
use crate::revocations;
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
    let welcome = MlsFraming::read(&envelope.envelope().payload) == Some(MlsFraming::Welcome);
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
                match device.commit_refused(channel) {
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

/// Commits in `channel` of `space` what `make` makes in its group, and sends it.
fn commit(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    make: impl FnOnce(&mut MlsDevice) -> Result<Vec<u8>, MlsError>,
) -> Result<(), String> {
    let version = follower.connection.version();
    let queued = step(follower, &link.keys, |device, changes| {
        let commit = make(device).map_err(|err| failed(channel, err))?;
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

/// Whether `channel` of `space` has a group on the server, and the epoch of its GroupInfo, if the
/// server has one.
fn on_server(
    follower: &mut Follower,
    space: SpaceId,
    channel: ChannelId,
) -> Result<(bool, Option<u64>), String> {
    let body = follower
        .connection
        .get_as(&group_info_path(space, channel), &follower.token)
        .map_err(text)?;
    let unexpected = || "unexpected answer from the server: a channel's group".to_owned();
    let given: ChannelGroup = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    let epoch = match given.group_info {
        Some(text) => {
            let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| unexpected())?;
            ChannelView::observe(channel, &bytes)
                .ok()
                .map(|view| view.epoch())
        }
        None => None,
    };
    Ok((given.group, epoch))
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

/// Brings in line the groups of the channels of each space the run accepted the revisions of
/// ([`Follower::accepted`]), as [`align`] does; warns of what fails.
pub fn align_accepted(follower: &mut Follower) {
    match Link::new(follower) {
        Ok(mut link) => align_with(follower, &mut link),
        Err(err) => eprintln!("skald: warning: {err}"),
    }
}

/// As [`align_accepted`], sending over `link`.
pub fn align_with(follower: &mut Follower, link: &mut Link) {
    let accepted: Vec<(SpaceId, (Roster, SpaceDevices))> = (follower.accepted.iter())
        .map(|(space, synced)| (*space, synced.clone()))
        .collect();
    for (space, (roster, devices)) in accepted {
        if let Err(err) = align(follower, link, space, (&roster, &devices)) {
            eprintln!("skald: warning: {err}");
        }
    }
}

/// Brings in line the group of each channel of `roster`, the last accepted revision of the space
/// `space`, with `devices`, its accounts' devices, that this device is admitted to. Of a group it
/// holds, it uploads the GroupInfo first if the server has none of its epoch. A channel whose
/// Commit of this device waits for the server is left as it is. Warns of a group it fails to bring
/// in line.
fn align(
    follower: &mut Follower,
    link: &mut Link,
    space: SpaceId,
    (roster, devices): (&Roster, &SpaceDevices),
) -> Result<(), String> {
    let body = roster.body();
    let waiting = follower.state.mls_outbox().map_err(text)?;
    let held = held(follower, &link.keys, &body.channels)?;
    for channel in &body.channels {
        let id = channel.id;
        let mut admitted = admitted(follower, roster, devices, id);
        if waiting.iter().any(|outgoing| outgoing.channel == id)
            || admitted.remove(&follower.this).is_none()
        {
            continue;
        }
        let aligned = match held.get(&id) {
            None => create(follower, link, (space, id), &admitted),
            Some(members) => keep_group_info(follower, link, space, id).and_then(|()| {
                keep_in_line(
                    follower,
                    link,
                    (space, id),
                    (roster, devices, &admitted),
                    members,
                )
            }),
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

/// Creates the group of `channel` in `space`, unless the server has one, whose Welcome the device
/// waits for, with each of `others`, the other admitted devices, it gets a valid KeyPackage for.
fn create(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    others: &BTreeMap<DeviceId, Cert>,
) -> Result<(), String> {
    if on_server(follower, space, channel)?.0 {
        return Ok(());
    }
    let packages = key_packages(follower, others)?;
    let certs: Vec<Cert> = others.values().cloned().collect();
    commit(follower, link, (space, channel), |device| {
        device.create_group(channel, now())?;
        device.add_devices(channel, &packages, &certs, now())
    })
}

/// Commits the Remove of each of `members`, the group of `channel`, that is not admitted, carrying
/// the revocations of `devices` by which those revoked are; with none, and if this device signed
/// `roster`, the Adds of the devices of `others`, the other admitted devices, that the group lacks.
fn keep_in_line(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    (roster, devices, others): (&Roster, &SpaceDevices, &BTreeMap<DeviceId, Cert>),
    members: &[MemberDevice],
) -> Result<(), String> {
    let this = follower.this;
    let strangers: Vec<DeviceId> = members
        .iter()
        .filter(|member| member.device != this && !admits(others, member))
        .map(|member| member.device)
        .collect();
    if !strangers.is_empty() {
        let own = &follower.own;
        let carried = revocations::carried(
            devices,
            |account| follow::master_of(roster, own, account),
            &strangers,
        );
        return commit(follower, link, (space, channel), |device| {
            device.remove_devices(channel, &strangers, &carried, now())
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
    commit(follower, link, (space, channel), |device| {
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
/// later run, and so does each later one of its channel. An application message of a channel whose
/// group the device does not hold stays until the device joins the group, then decrypts at once;
/// one of a channel that does not admit the device fails.
pub fn take_in(
    follower: &mut Follower,
    link: &mut Link,
    scope: Scope,
    rosters: &mut Rosters,
) -> Result<(), String> {
    let followed = follower.state.spaces().map_err(text)?;
    let waiting = follower.state.mls_outbox().map_err(text)?;
    // The channels whose entries stay for a later run from here on, to keep the mailbox order.
    let mut later: BTreeSet<ChannelId> =
        (waiting.iter()).map(|outgoing| outgoing.channel).collect();
    // The application messages of each channel that wait for the device to join its group.
    let mut unjoined: BTreeMap<ChannelId, Vec<(u64, Envelope)>> = BTreeMap::new();
    for entry in follower.state.inbox().map_err(text)? {
        if entry.opened.is_some() {
            continue;
        }
        let seq = entry.seq;
        let Some((envelope, kind)) = classify(&entry.frame) else {
            continue;
        };
        let channel = envelope.header.channel;
        if scope.excludes(envelope.header.space) || later.contains(&channel) {
            continue;
        }
        // One of a space the device does not follow yet waits until it does.
        let Some(space) = followed
            .iter()
            .find(|space| space.id == envelope.header.space)
        else {
            continue;
        };
        if kind == Kind::Application && unjoined.contains_key(&channel) {
            unjoined.entry(channel).or_default().push((seq, envelope));
            continue;
        }
        if kind == Kind::Application && open(follower, link, seq, &envelope)? {
            continue;
        }
        if let Entry::Vacant(vacant) = rosters.entry(space.id) {
            match follower.sync(space) {
                Ok(done) => {
                    vacant.insert(done);
                }
                Err(err) => {
                    eprintln!("skald: warning: {err}");
                    later.insert(channel);
                    continue;
                }
            }
        }
        if kind == Kind::Application {
            let (roster, devices) = &rosters[&space.id];
            if admitted(follower, roster, devices, channel).contains_key(&follower.this) {
                unjoined.entry(channel).or_default().push((seq, envelope));
            } else {
                let why = "this device is in no group of the channel, which does not admit it";
                keep(follower, seq, &Opened::Failed(why.to_owned()))?;
            }
            continue;
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
        if unjoined.contains_key(&channel) && members(follower, &link.keys, channel)?.is_some() {
            for (seq, envelope) in unjoined.remove(&channel).unwrap_or_default() {
                open(follower, link, seq, &envelope)?;
            }
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
        let taken = MlsChanges {
            taken: vec![seq],
            ..MlsChanges::default()
        };
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

/// Processes another device's Commit that `taking` holds. It keeps the revocations the Commit
/// carries that count, with the epoch, and warns of the devices they revoke. Of each device it adds
/// that is not admitted, the device warns once it accepted the space's latest revisions; it then
/// uploads the GroupInfo if the server has none of the new epoch.
fn processed(
    follower: &mut Follower,
    link: &mut Link,
    taking: &Taking,
    rosters: &mut Rosters,
) -> Result<(), String> {
    let (seq, space, envelope) = (taking.seq, taking.space, taking.envelope);
    let channel = envelope.header.channel;
    let mut seen = None;
    let mut learned = Vec::new();
    let (roster, devices) = &rosters[&space.id];
    let own = &follower.own;
    let master = |account| follow::master_of(roster, own, account);
    let result = step(follower, &link.keys, |device, changes| {
        changes.taken.push(seq);
        if device.epoch(channel).is_err() {
            // A Commit of a group the device is not in.
            return Ok(None);
        }
        let processed = device.process_commit(channel, &envelope.payload, now(), |change| {
            learned = revocations::learned(&change.authenticated_data, devices, master);
            seen = Some(change.clone());
            true
        });
        if processed.is_ok() {
            changes.revocations = learned.iter().map(skald_proto::Cbor::to_cbor).collect();
        }
        Ok(Some(processed))
    })?;
    let name = &space.name;
    let change = match (result, seen) {
        (Some(Ok(Processed::Entered { .. })), Some(change)) => change,
        (Some(Err(err)), _) => {
            eprintln!(
                "skald: warning: this device cannot process a Commit in the channel {channel} in \
                 the space {name}: {err}"
            );
            return Ok(());
        }
        _ => return Ok(()),
    };
    if !learned.is_empty()
        && let Some((roster, devices)) = rosters.get_mut(&space.id)
    {
        devices.revocations.extend(learned);
        for warning in follow::device_changes(&follower.state, roster, devices, &follower.own)? {
            eprintln!("skald: warning: {warning}");
        }
    }
    let mut added = strangers(follower, &rosters[&space.id], channel, &change.added);
    if !added.is_empty() {
        // Before it holds them not admitted, the device accepts the space's latest revisions.
        sync_again(follower, space, rosters);
        added = strangers(follower, &rosters[&space.id], channel, &change.added);
    }
    for device in added {
        eprintln!(
            "skald: warning: a Commit of the device {} in the channel {channel} in the space \
             {name} added the device {device}, which is not admitted to the channel; this device \
             removes it",
            change.sender
        );
    }
    if let Err(err) = keep_group_info(follower, link, space.id, channel) {
        eprintln!("skald: warning: {err}");
    }
    Ok(())
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
/// keeps what that gave with the entry, with the state it leaves; gives whether it did: it writes
/// nothing if the device holds no group of the channel.
fn open(follower: &Follower, link: &Link, seq: u64, envelope: &Envelope) -> Result<bool, String> {
    let channel = envelope.header.channel;
    follower.state.mls_change(|sealed| {
        let (mut device, state_key) = load_keyed(follower, &link.keys, sealed)?;
        let (opened, saved) = match device.decrypt(channel, &envelope.payload) {
            Ok(read) => (
                Opened::Read(read.sender, read.plaintext),
                Some(device.seal()),
            ),
            Err(MlsError::NoGroup(_)) => return Ok((None, MlsChanges::default(), false)),
            Err(MlsError::Replay) => (Opened::Replay, None),
            Err(err) => (
                Opened::Failed(format!("it does not decrypt in the channel's group: {err}")),
                None,
            ),
        };
        Ok((saved, kept(follower, &state_key, seq, &opened), true))
    })
}

/// Keeps `opened` with the entry `seq` without decrypting it, leaving the MLS state as it is.
fn keep(follower: &Follower, seq: u64, opened: &Opened) -> Result<(), String> {
    follower.state.mls_change(|sealed| {
        let state_key = key_packages::state_key(&follower.secrets, sealed)?;
        Ok((None, kept(follower, &state_key, seq, opened), ()))
    })
}

/// What keeps `opened` with the entry `seq`, sealed under `state_key`.
fn kept(follower: &Follower, state_key: &StateKey, seq: u64, opened: &Opened) -> MlsChanges {
    let sealed = state_key.seal(follower.this, &opened_item(seq), &opened.to_bytes());
    MlsChanges {
        opened: vec![(seq, sealed)],
        ..MlsChanges::default()
    }
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
