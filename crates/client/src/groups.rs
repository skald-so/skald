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
//! - Its own Commit goes into the outbox with the state that holds it pending, all at once, before
//!   it is sent, and is sent again with the same bytes until the server answers. After `stored` it
//!   merges the Commit, uploads the new epoch's GroupInfo, then sends the Welcome to the devices it
//!   added, which goes through the outbox too; after a refusal it drops the Commit, and a group
//!   whose first Commit the server refused, as it kept another, waits for that one's Welcome. A
//!   member uploads the GroupInfo when the server has none of its current epoch.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt::Display;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{
    ChannelView, Joining, MemberDevice, MlsContent, MlsDevice, MlsError, MlsFraming, Processed,
    SealedState, check_key_package,
};
use skald_proto::{
    Cbor as _, Cert, Channel, ChannelId, DeviceId, Envelope, Frame, Header, MessageId, NonEmpty,
    Push, RawEnvelope, Roster, Scheme, SpaceId, Unknown,
};

use crate::connection::{Method, WebSocket};
use crate::device::Keys;
use crate::devices::text;
use crate::follow::SpaceDevices;
use crate::key_packages;
use crate::send::{self, Answer, Key, Wire};
use crate::spaces::{Follower, Scope, now};
use crate::state::{self, MlsChanges, MlsOutgoing, State};

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
    let state_key = key_packages::state_key(&follower.secrets, sealed)?;
    MlsDevice::load(keys.signing(), &state_key, sealed)
        .map_err(|err| format!("this device's MLS state does not load: {err}"))
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
        let aligned = match held.get(&id) {
            None => create(follower, link, (space, id), &admitted),
            Some(members) => {
                keep_in_line(follower, link, (space, id), (roster, &admitted), members)
            }
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
        return commit(follower, link, (space, channel), |device| {
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

/// A Welcome or Commit in the mailbox.
enum Handshake {
    Welcome,
    Commit,
}

/// The envelope of `entry`, a mailbox entry as its frame, if it holds a Welcome or a Commit.
fn handshake(entry: &[u8]) -> Option<(Envelope, Handshake)> {
    let Ok(Frame::Push(Push::Message { envelope, .. })) = Frame::from_cbor(entry) else {
        return None;
    };
    let envelope = envelope.envelope().clone();
    if envelope.header.scheme != Scheme::Mls {
        return None;
    }
    let kind = match MlsFraming::read(&envelope.payload)? {
        MlsFraming::Welcome => Handshake::Welcome,
        MlsFraming::Message {
            public: true,
            content: MlsContent::Commit,
            ..
        } => Handshake::Commit,
        _ => return None,
    };
    Some((envelope, kind))
}

/// Whether `entry`, a mailbox entry as its frame, holds a Welcome or a Commit.
pub fn is_handshake(entry: &[u8]) -> bool {
    handshake(entry).is_some()
}

/// Goes through the Welcomes and Commits of the mailbox entries the state holds within `scope`, in
/// `seq` order, and takes each one out with the state it leaves, checking them against the spaces
/// as `rosters` holds them, syncing any other at its first entry. One of a space the device does not
/// follow or that fails to sync, or of a channel whose Commit of this device waits for the server,
/// stays for a later run.
pub fn take_in(
    follower: &mut Follower,
    link: &mut Link,
    scope: Scope,
    rosters: &mut Rosters,
) -> Result<(), String> {
    let followed = follower.state.spaces().map_err(text)?;
    let waiting = follower.state.mls_outbox().map_err(text)?;
    for (seq, entry) in follower.state.inbox().map_err(text)? {
        let Some((envelope, kind)) = handshake(&entry) else {
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
        match kind {
            Handshake::Welcome => welcomed(follower, link, (seq, space), &envelope, rosters)?,
            Handshake::Commit => processed(follower, link, (seq, space), &envelope, rosters)?,
        }
    }
    Ok(())
}

/// Joins a channel's group from the Welcome in `envelope`, the entry `seq` of `space`, if its
/// signer is admitted to the channel, warning of each member it lists that is not; warns of a
/// Welcome it refuses.
fn welcomed(
    follower: &mut Follower,
    link: &mut Link,
    (seq, space): (u64, &state::Space),
    envelope: &Envelope,
    rosters: &Rosters,
) -> Result<(), String> {
    let channel = envelope.header.channel;
    let (roster, devices) = &rosters[&space.id];
    let admitted = admitted(follower, roster, devices, channel);
    let mut seen: Option<Joining> = None;
    let joined = step(follower, &link.keys, |device, changes| {
        changes.taken.push(seq);
        Ok(device.join(&envelope.payload, now(), |joining| {
            seen = Some(joining.clone());
            joining.channel == channel && admits(&admitted, &joining.signer)
        }))
    })?;
    let name = &space.name;
    match (joined, seen) {
        (Ok(_), Some(joining)) => {
            for member in joining
                .members
                .iter()
                .filter(|member| !admits(&admitted, member))
            {
                eprintln!(
                    "skald: warning: the group of the channel {channel} in the space {name} that \
                     this device joined holds the device {}, which is not admitted to the \
                     channel; this device removes it",
                    member.device
                );
            }
        }
        (Err(MlsError::Refused), Some(joining)) => eprintln!(
            "skald: warning: this device refused a Welcome to the channel {channel} in the space \
             {name}: its signer, the device {}, is not admitted to the channel",
            joining.signer.device
        ),
        (Err(MlsError::GroupExists(_)), _) | (Ok(_), None) => {}
        (Err(err), _) => eprintln!(
            "skald: warning: this device cannot join the channel {channel} in the space {name} \
             from a Welcome: {err}"
        ),
    }
    Ok(())
}

/// Processes another device's Commit in `envelope`, the entry `seq` of `space`. Of each device it
/// adds that is not admitted, the device warns once it accepted the space's latest revisions; it
/// then uploads the GroupInfo if the server has none of the new epoch.
fn processed(
    follower: &mut Follower,
    link: &mut Link,
    (seq, space): (u64, &state::Space),
    envelope: &Envelope,
    rosters: &mut Rosters,
) -> Result<(), String> {
    let channel = envelope.header.channel;
    let mut seen = None;
    let result = step(follower, &link.keys, |device, changes| {
        changes.taken.push(seq);
        if device.epoch(channel).is_err() {
            // A Commit of a group the device is not in.
            return Ok(None);
        }
        Ok(Some(device.process_commit(
            channel,
            &envelope.payload,
            now(),
            |change| {
                seen = Some(change.clone());
                true
            },
        )))
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
    let mut added = strangers(follower, &rosters[&space.id], channel, &change.added);
    if !added.is_empty() {
        // Before it holds them not admitted, the device accepts the space's latest revisions.
        match follower.sync(space) {
            Ok(done) => {
                added = strangers(follower, &done, channel, &change.added);
                rosters.insert(space.id, done);
            }
            Err(err) => eprintln!("skald: warning: {err}"),
        }
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
