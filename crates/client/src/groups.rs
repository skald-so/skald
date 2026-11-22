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
//!   last accepted revision, the Adds of the admitted devices the group lacks; with neither, an
//!   empty Commit once its current epoch is older than the `history` of the space's policy,
//!   counted from when the device entered it ("History key"). A Commit with the
//!   Remove of a revoked device carries the revocations by which this device holds it revoked; of
//!   those another device's Commit carries, it keeps the ones that count, and warns of the devices
//!   they revoke, before it judges the new epoch.
//! - Outside a group the server has, it joins by itself from the GroupInfo of the current epoch
//!   once its signer and members are admitted or revoked devices of the channel's member accounts,
//!   whose Remove it then commits, unless it lists the device and a Welcome comes. A device whose
//!   Welcome it refused joins by itself from that GroupInfo also if it lists the device, and one
//!   whose Commit it could not process rejoins from a GroupInfo of a later epoch than its own;
//!   either removes the leaf of its own the GroupInfo lists. One a Commit removed while admitted
//!   warns and rejoins. A Commit of an epoch it has passed is a replay, dropped.
//! - Its own Commit goes into the outbox with the state that holds it pending, all at once, before
//!   it is sent, and is sent again with the same bytes until the server answers. After `stored` it
//!   merges the Commit, uploads the new epoch's GroupInfo, then sends the Welcome to the devices it
//!   added, which goes through the outbox too; after a refusal it drops the Commit, and a group
//!   whose first Commit the server refused, as it kept another, waits for that one's Welcome. A
//!   member uploads the GroupInfo when the server has none of its current epoch.
//! - It judges each epoch it enters, by creating the group, a Welcome, a Commit, its own or
//!   another's, or a join or rejoin, after applying the revocations the Commit carries; once its
//!   Commit is stored, it uploads the new epoch's link ([`crate::key_chain`]) and wraps
//!   ([`crate::wraps`]).
//! - It decrypts each application message of its mailbox in its channel's group in that order,
//!   among the Welcomes and Commits, and keeps what that gave with the entry, sealed under the
//!   state key, until a run shows it; a replay gives nothing to show. Once an entry of a channel
//!   stays for a later run, so does each later one there; a message that comes before the device
//!   joins the channel's group from its Welcome waits for it, unless the channel does not admit
//!   the device.
//! - It encrypts a channel message in the channel's group only while every member device is
//!   admitted, no Commit of its own there waits for the server and another device is in it; the
//!   message goes into the outbox with the state, its history entry ([`crate::history`]) and its
//!   content's `counter`, the device's next in the channel. A task's result goes so too, or, to a
//!   sending device outside the group, under `hpke` with the counter the device keeps for it.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::num::NonZeroU64;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{
    ChannelView, DeviceCheckError, Joining, MemberDevice, MlsContent, MlsDevice, MlsError,
    MlsFraming, Processed, SealedState, StateKey, check_device, check_key_package,
    check_valid_device, seal_message,
};
use skald_proto::{
    Cbor as _, Cert, Channel, ChannelId, Content, DeviceId, Envelope, Frame, Header, MessageId,
    NonEmpty, Push, RawEnvelope, Roster, Scheme, SpaceId, TaskId, Unknown,
};

use crate::clock;
use crate::connection::{Connection, Method};
use crate::copies::Reader;
use crate::device::Keys;
use crate::devices::text;
use crate::follow::{self, SpaceDevices};
use crate::history;
use crate::key_chain;
use crate::key_changes;
use crate::key_packages;
use crate::mailbox::Mailbox;
use crate::message_status;
use crate::queue_keys;
use crate::recv;
use crate::revocations;
use crate::send::{self, Answer, Key, Wire};
use crate::spaces::{Follower, Scope, now};
use crate::state::{self, Counted, MlsChanges, MlsOutgoing, Outgoing, State};
use crate::wraps;

/// The problem of a Commit of an epoch that is not the channel's current one.
const EPOCH_MISMATCH: &str = "epoch-mismatch";

/// The spaces a run synced: each one's last accepted revision and its accounts' devices.
pub type Rosters = BTreeMap<SpaceId, (Roster, SpaceDevices)>;

/// What a run sends the device's Commits, Welcomes and links with: its keys, its connection once
/// open, and the number of the last link it uploaded.
pub struct Link {
    pub keys: Keys,
    pub mailbox: Option<Mailbox>,
    pub uploads: u64,
}

impl Link {
    pub fn new(follower: &Follower) -> Result<Self, String> {
        Ok(Link {
            keys: follower.keys()?,
            mailbox: None,
            uploads: 0,
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
    fn answered(&self, _: &State, _: &Answer) {}
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
pub fn read<T>(
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
    // A `service` device is admitted where the channel's `services` list it, unless its run is
    // stateless: such a device is no member.
    let this = match follower.service {
        Some(member) if member.stateless => false,
        Some(_) => (body.channels.iter())
            .any(|listed| listed.id == channel && listed.services.contains(&follower.this)),
        None => own.is_some() && listed && !excluded,
    };
    if this {
        admitted.insert(follower.this, follower.cert.clone());
    }
    admitted
}

/// Whether `member`, a leaf of a group, is one of `admitted`, with its certificate's signing key.
pub fn admits(admitted: &BTreeMap<DeviceId, Cert>, member: &MemberDevice) -> bool {
    admitted
        .get(&member.device)
        .is_some_and(|cert| cert.body().signing == member.signing)
}

/// The devices of `members`, leaves of a group, that are not of `admitted`, as [`admits`] says.
fn unadmitted<'m>(
    admitted: &BTreeMap<DeviceId, Cert>,
    members: impl IntoIterator<Item = &'m MemberDevice>,
) -> Vec<DeviceId> {
    (members.into_iter())
        .filter(|member| !admits(admitted, member))
        .map(|member| member.device)
        .collect()
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
        link.mailbox.take(),
    );
    link.mailbox = delivered.mailbox.take();
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

/// The server stored the device's Commit `msg_id` in `channel`: the device merges it, judges the
/// new epoch, uploads its GroupInfo and its link, and sends the Commit's Welcome, if any, to the
/// devices it added.
fn committed(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    msg_id: MessageId,
) -> Result<bool, String> {
    let version = follower.connection.version();
    let admitted = admitted_now(follower, space, channel);
    let (welcome, judgment) = step(follower, &link.keys, |device, changes| {
        changes.answered.push(msg_id);
        let devices = |device: &MlsDevice| -> Result<Vec<DeviceId>, MlsError> {
            let members = device.members(channel)?;
            Ok(members.iter().map(|member| member.device).collect())
        };
        // A group the device dropped meanwhile has nothing to merge.
        let Ok(before) = devices(device) else {
            return Ok((None, None));
        };
        // The device is in the group of the channel's current epoch: an external Commit of its own
        // was applied at once, another one merges now.
        changes.rejoined.push(channel);
        let welcome = match device.commit_stored(channel) {
            Ok(welcome) => welcome,
            Err(MlsError::NoCommitPending(_)) => None,
            Err(err) => return Err(failed(channel, err)),
        };
        entered(device, channel, changes);
        let judgment = admitted
            .as_ref()
            .map(|admitted| judged(device, channel, admitted))
            .transpose()?;
        changes.judged.extend(judgment);
        let Some(welcome) = welcome else {
            return Ok((None, judgment));
        };
        let added = (devices(device)
            .map_err(|err| failed(channel, err))?
            .into_iter())
        .filter(|member| !before.contains(member))
        .collect();
        let queued = NonEmpty::from_vec(added)
            .map(|added| outgoing(version, (space, channel), &welcome, Some(added)));
        changes.queued.extend(queued.clone());
        Ok((queued, judgment))
    })?;
    if let Err(err) = upload_group_info(follower, link, space, channel) {
        eprintln!("skald: warning: {err}");
    }
    if let Some((_, epoch, true)) = judgment {
        key_chain::upload_new(follower, link, (space, channel), epoch);
        wraps::upload_new(follower, link, (space, channel), epoch);
        queue_keys::upload_new(follower, link, (space, channel), epoch);
    }
    match welcome {
        Some(welcome) => send_out(follower, link, welcome),
        None => Ok(true),
    }
}

/// Records that the device entered its current epoch of `channel`'s group now, by the clock of
/// epochs' ages, unless it recorded that epoch already.
fn entered(device: &MlsDevice, channel: ChannelId, changes: &mut MlsChanges) {
    if let Ok(epoch) = device.epoch(channel) {
        changes.entered.push((channel, epoch, clock::epoch_now()));
    }
}

/// The judgment of the epoch of `channel`'s group the device is in, by `admitted`, the devices the
/// space's last accepted revision admits to the channel ("Clean epochs").
fn judged(
    device: &MlsDevice,
    channel: ChannelId,
    admitted: &BTreeMap<DeviceId, Cert>,
) -> Result<(ChannelId, u64, bool), String> {
    let epoch = device.epoch(channel).map_err(|err| failed(channel, err))?;
    let members = device
        .members(channel)
        .map_err(|err| failed(channel, err))?;
    Ok((channel, epoch, key_chain::clean(admitted, &members)))
}

/// Judges the epoch of `channel`'s group the device entered, by `admitted`, as [`judged`] says,
/// unless it judged it already.
fn judge(
    follower: &Follower,
    keys: &Keys,
    channel: ChannelId,
    admitted: &BTreeMap<DeviceId, Cert>,
) -> Result<(), String> {
    let judged = read(follower, keys, |device| judged(device, channel, admitted))??;
    change(
        follower,
        MlsChanges {
            judged: vec![judged],
            ..MlsChanges::default()
        },
    )
}

/// The devices admitted to `channel` of `space` by the last revision of the space the run accepted,
/// accepting the space's latest revisions first if the run has not; none if it cannot.
fn admitted_now(
    follower: &mut Follower,
    space: SpaceId,
    channel: ChannelId,
) -> Option<BTreeMap<DeviceId, Cert>> {
    if !follower.accepted.contains_key(&space) {
        let held = follower.state.spaces().ok()?;
        let held = held.iter().find(|held| held.id == space)?;
        if let Err(err) = follower.sync(held) {
            eprintln!("skald: warning: {err}");
            return None;
        }
    }
    let (roster, devices) = follower.accepted.get(&space)?;
    Some(admitted(follower, roster, devices, channel))
}

/// A new envelope of `channel` in `space`, of protocol version `version`, with `payload`, an MLS
/// message, to `recipients`, none for the channel, as it goes into the outbox.
fn outgoing(
    version: NonZeroU64,
    (space, channel): (SpaceId, ChannelId),
    payload: &[u8],
    recipients: Option<NonEmpty<DeviceId>>,
) -> MlsOutgoing {
    let msg_id = MessageId::generate();
    let envelope = mls_envelope(
        version,
        (space, channel),
        msg_id,
        payload.to_vec(),
        recipients,
    );
    MlsOutgoing {
        msg_id,
        space,
        channel,
        envelope,
    }
}

/// The bytes of the `mls` envelope `msg_id` of `channel` in `space`, of protocol version `version`,
/// with `payload`, to `recipients`, none for the channel.
fn mls_envelope(
    version: NonZeroU64,
    (space, channel): (SpaceId, ChannelId),
    msg_id: MessageId,
    payload: Vec<u8>,
    recipients: Option<NonEmpty<DeviceId>>,
) -> Vec<u8> {
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
    Envelope { header, payload }.to_cbor()
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
pub fn group_on_server(
    follower: &mut Follower,
    space: SpaceId,
    channel: ChannelId,
) -> Result<(bool, Option<Vec<u8>>), String> {
    let token = follower.token.clone();
    fetch_group(&mut follower.connection, &token, (space, channel))
}

/// Whether the GroupInfo of the current epoch of `channel` of `space` that the server gives the
/// session or service token `token`, if it has one, lists `device`.
pub fn group_info_lists(
    connection: &mut Connection,
    token: &str,
    (space, channel): (SpaceId, ChannelId),
    device: DeviceId,
) -> Result<bool, String> {
    let (_, group_info) = fetch_group(connection, token, (space, channel))?;
    Ok(group_info.is_some_and(|bytes| {
        ChannelView::observe(channel, &bytes)
            .is_ok_and(|view| view.members().iter().any(|member| member.device == device))
    }))
}

/// As [`group_on_server`], on `connection` as the session or service token `token`.
fn fetch_group(
    connection: &mut Connection,
    token: &str,
    (space, channel): (SpaceId, ChannelId),
) -> Result<(bool, Option<Vec<u8>>), String> {
    let body = connection
        .get_as(&group_info_path(space, channel), token)
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
            Err(err) => {
                let warning = format!(
                    "the server handed out a KeyPackage of the device {device} that fails its \
                     checks: {err}; this device does not add it"
                );
                let key = key_changes::key_of("key package", &package);
                key_changes::keep_once(&follower.state, &key, &warning)?;
                eprintln!("skald: warning: {warning}");
            }
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
/// holds and does not rejoin, it uploads the GroupInfo first if the server has none of its epoch. A
/// channel whose Commit of this device waits for the server is left as it is. Warns of a group it
/// fails to bring in line.
fn align(
    follower: &mut Follower,
    link: &mut Link,
    space: SpaceId,
    (roster, devices): (&Roster, &SpaceDevices),
) -> Result<(), String> {
    let body = roster.body();
    let waiting = follower.state.mls_outbox().map_err(text)?;
    let rejoining = follower.state.rejoining().map_err(text)?;
    let welcomes = welcomes_waiting(follower)?;
    let held = held(follower, &link.keys, &body.channels)?;
    for channel in &body.channels {
        let id = channel.id;
        let mut admitted = admitted(follower, roster, devices, id);
        if waiting.iter().any(|outgoing| outgoing.channel == id)
            || admitted.remove(&follower.this).is_none()
        {
            continue;
        }
        let at = (space, id);
        let aligned = match held.get(&id) {
            // A Welcome in the mailbox comes first.
            None if welcomes.contains(&id) => Ok(()),
            None => outside(
                follower,
                link,
                at,
                (roster, devices, &admitted),
                rejoining.contains(&id),
            ),
            Some(_) if rejoining.contains(&id) => {
                behind(follower, link, at, (roster, devices, &admitted))
            }
            Some(members) => keep_group_info(follower, link, space, id).and_then(|()| {
                keep_in_line(follower, link, at, (roster, devices, &admitted), members)
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
/// `others`, the other admitted devices by `roster` and `devices`: creates the group if the server
/// has none, unless it is a `service` device, which waits for a member's; otherwise joins by itself
/// from the GroupInfo of the current epoch, if the server has one, unless it lists this device,
/// which another device added and whose Welcome comes, or `rejoining`.
fn outside(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    (roster, devices, others): (&Roster, &SpaceDevices, &BTreeMap<DeviceId, Cert>),
    rejoining: bool,
) -> Result<(), String> {
    let (group, group_info) = group_on_server(follower, space, channel)?;
    if !group && follower.service.is_none() {
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
        (roster, devices, others),
    )
}

/// Rejoins `channel` of `space`, whose group this device holds but whose Commit it could not
/// process or whose Welcome it refused, from the GroupInfo of the current epoch, if the server has
/// one of an epoch past the device's; with `others`, the other admitted devices by `roster` and
/// `devices`.
fn behind(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    (roster, devices, others): (&Roster, &SpaceDevices, &BTreeMap<DeviceId, Cert>),
) -> Result<(), String> {
    let epoch = held_epoch(follower, link, channel)?;
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
            (roster, devices, others),
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

/// The epoch of `channel`'s group, which the device holds.
fn held_epoch(follower: &Follower, link: &Link, channel: ChannelId) -> Result<u64, String> {
    read(follower, &link.keys, |device| device.epoch(channel))?.map_err(|err| failed(channel, err))
}

/// `channel`'s GroupInfo `group_info`, the server's, as seen from outside the group.
pub fn observe(channel: ChannelId, group_info: &[u8]) -> Result<ChannelView, String> {
    ChannelView::observe_joinable(channel, group_info).map_err(|err| {
        format!("the server gave a GroupInfo of the channel {channel} that does not check: {err}")
    })
}

/// Of `signer` and `members`, a GroupInfo's, those that are not of `others`, the other admitted
/// devices, nor of `revoked`, as [`revoked_members`] gives them, nor this device, `this`, with its
/// own key `signing`.
fn not_admitted(
    (this, signing): (DeviceId, &[u8; 32]),
    (others, revoked): (&BTreeMap<DeviceId, Cert>, &BTreeMap<DeviceId, [u8; 32]>),
    (signer, members): (&MemberDevice, &[MemberDevice]),
) -> BTreeSet<DeviceId> {
    let own = |member: &&MemberDevice| member.device == this && member.signing == *signing;
    let listed = members.iter().chain([signer]).filter(|member| !own(member));
    let revoked = |member: &&MemberDevice| revoked.get(&member.device) == Some(&member.signing);
    unadmitted(others, listed.filter(|member| !revoked(member)))
        .into_iter()
        .collect()
}

/// The revoked devices of the accounts in `channel`'s `channel-member` entries in `roster`, by
/// `devices`, each with its signing key: their certificates pass checks 1 and 2 against the master
/// key [`follow::master_of`] gives, and check 3 or 4 fails ("Group changes").
fn revoked_members(
    follower: &Follower,
    roster: &Roster,
    devices: &SpaceDevices,
    channel: ChannelId,
) -> BTreeMap<DeviceId, [u8; 32]> {
    let body = roster.body();
    let mut revoked = BTreeMap::new();
    let Some(channel) = body.channels.iter().find(|listed| listed.id == channel) else {
        return revoked;
    };
    let SpaceDevices {
        certs, revocations, ..
    } = devices;
    for entry in &channel.members {
        let Some(master) = follow::master_of(roster, &follower.own, entry.account) else {
            continue;
        };
        for cert in certs
            .iter()
            .filter(|cert| cert.body().account == entry.account)
        {
            let device = cert.body().device;
            if let Ok(valid) = check_device(&master, certs, device)
                && let Err(DeviceCheckError::Revoked { .. }) =
                    check_valid_device(&master, certs, revocations, device)
            {
                revoked.insert(device, valid.body().signing);
            }
        }
    }
    revoked
}

/// Joins `channel` of `space` by itself from `group_info`, seen as `view`, if its signer and every
/// member it lists are admitted, with `others`, the other admitted devices by `roster` and
/// `devices`, or revoked devices of the channel's member accounts ([`revoked_members`]), once the
/// device accepted the space's latest revisions if one is not; warns naming the others. The
/// external Commit goes through the outbox as the device's other Commits do. Once the server
/// stored it, the device commits the Remove of the revoked devices the group holds.
fn join_by_itself(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    (view, group_info): (&ChannelView, &[u8]),
    (roster, devices, others): (&Roster, &SpaceDevices, &BTreeMap<DeviceId, Cert>),
) -> Result<(), String> {
    let own = (follower.this, &link.keys.signing().public_key());
    let listed = (&view.signer(), view.members());
    let none = BTreeMap::new();
    let (mut roster, mut devices) = (roster.clone(), devices.clone());
    let mut admitted = others.clone();
    if !not_admitted(own, (&admitted, &none), listed).is_empty() {
        // Before it holds them not admitted, the device accepts the space's latest revisions.
        let synced = follower.state.spaces().map_err(text)?;
        if let Some(held) = synced.iter().find(|held| held.id == space) {
            (roster, devices) = follower.sync(held)?;
            admitted = self::admitted(follower, &roster, &devices, channel);
            admitted.remove(&follower.this);
        }
    }
    let revoked = revoked_members(follower, &roster, &devices, channel);
    let strangers = not_admitted(own, (&admitted, &revoked), listed);
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
            joining.channel == channel
                && not_admitted(own, (&admitted, &revoked), listed).is_empty()
        })
    })?;
    let waiting = follower.state.mls_outbox().map_err(text)?;
    if waiting.iter().any(|outgoing| outgoing.channel == channel)
        || not_admitted(own, (&admitted, &none), listed).is_empty()
    {
        return Ok(());
    }
    let Some(members) = members(follower, &link.keys, channel)? else {
        return Ok(());
    };
    keep_in_line(
        follower,
        link,
        (space, channel),
        (&roster, &devices, &admitted),
        &members,
    )
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

/// Commits the Remove of each of `members`, the group of `channel`, that is not admitted, carrying
/// the revocations of `devices` by which those revoked are; with none, the Adds of the devices of
/// `others`, the other admitted devices, that the group lacks: all of them if this device signed
/// `roster`, otherwise those it certified; with neither, an empty Commit if the epoch is older than
/// the period of `roster`'s policy ([`rotate_if_aged`]).
fn keep_in_line(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    (roster, devices, others): (&Roster, &SpaceDevices, &BTreeMap<DeviceId, Cert>),
    members: &[MemberDevice],
) -> Result<(), String> {
    let this = follower.this;
    let strangers = unadmitted(
        others,
        members.iter().filter(|member| member.device != this),
    );
    if !strangers.is_empty() {
        let own = &follower.own;
        let carried = revocations::carried(
            devices,
            |account| follow::master_of(roster, own, account),
            &strangers,
        );
        return commit(follower, link, (space, channel), |device, _| {
            device.remove_devices(channel, &strangers, &carried, now())
        });
    }
    // The signer of the revision adds every device the group lacks; any other device, those it
    // certified ("Group changes").
    let signed = roster.body().signer == this;
    let lacking: BTreeMap<DeviceId, Cert> = others
        .iter()
        .filter(|(device, _)| !members.iter().any(|member| member.device == **device))
        .filter(|(_, cert)| signed || cert.body().issuer == Some(this))
        .map(|(device, cert)| (*device, cert.clone()))
        .collect();
    let packages = key_packages(follower, &lacking)?;
    if !packages.is_empty() {
        let certs: Vec<Cert> = lacking.into_values().collect();
        return commit(follower, link, (space, channel), |device, _| {
            device.add_devices(channel, &packages, &certs, now())
        });
    }
    let period = roster.body().policy.history;
    rotate_if_aged(follower, link, (space, channel), period)
}

/// Commits an empty Commit in `channel` of `space` if the device entered its current epoch more
/// than `period` seconds ago, by the clock of epochs' ages ("History key"). An epoch it has no
/// record of entering counts from now: the device records it.
fn rotate_if_aged(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    period: u64,
) -> Result<(), String> {
    let epoch = held_epoch(follower, link, channel)?;
    let at = clock::epoch_now();
    match follower.state.epoch_entered(channel).map_err(text)? {
        Some((held, entered)) if held == epoch => {
            if at.saturating_sub(entered) <= period {
                return Ok(());
            }
        }
        _ => {
            return change(
                follower,
                MlsChanges {
                    entered: vec![(channel, epoch, at)],
                    ..MlsChanges::default()
                },
            );
        }
    }
    commit(follower, link, (space, channel), |device, _| {
        device.commit_empty(channel, &[], now())
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
    unadmitted(&admitted(follower, roster, devices, channel), added)
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
        unadmitted(admitted, seen.iter().flat_map(|joining| &joining.members))
    };
    if joined.is_ok() && !synced && !strangers(&admitted).is_empty() {
        sync_again(follower, space, rosters);
        admitted = admitted_in(follower, rosters, (space.id, channel));
    }
    let name = &space.name;
    match (joined, &seen) {
        (Ok(_), _) => {
            if let Err(err) = judge(follower, &link.keys, channel, &admitted) {
                eprintln!("skald: warning: {err}");
            }
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
            Ok(_) => {
                taken.rejoined.push(channel);
                entered(&device, channel, &mut taken);
            }
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
    let of_epoch = match MlsFraming::read(&envelope.payload) {
        Some(MlsFraming::Message { epoch, .. }) => epoch,
        _ => 0,
    };
    let mut seen = None;
    let mut learned = Vec::new();
    let (roster, devices) = &rosters[&space.id];
    let own = &follower.own;
    let master = |account| follow::master_of(roster, own, account);
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
            learned = revocations::learned(&change.authenticated_data, devices, master);
            seen = Some(change.clone());
            true
        });
        match processed {
            Err(_) => changes.rejoin.push(channel),
            Ok(entered) => {
                changes.revocations = learned.iter().map(skald_proto::Cbor::to_cbor).collect();
                if let Processed::Entered { epoch } = entered {
                    changes.entered.push((channel, epoch, clock::epoch_now()));
                }
            }
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
    let admitted = admitted_in(follower, rosters, (space.id, channel));
    if let Err(err) = judge(follower, &link.keys, channel, &admitted) {
        eprintln!("skald: warning: {err}");
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
/// devices, in the channel's group, and puts the `mls` envelope in the outbox, as from `agent` if
/// given, with the state it leaves, the message's history entry ([`crate::history`]) and what keeps
/// it for this device once the server stores it ([`recv::own_message`]); after it, a copy under
/// `hpke` to each of `readers`, of that content with the counter the device keeps for that reader
/// in the channel ([`crate::copies`]). Gives the message ID and the readers that get their first
/// copy of the channel's messages. Sends nothing, and says why, while a member device of the group
/// is not admitted, while a Commit of this device in the channel waits for the server, or when no
/// other device is in the group, nothing on this device gets the message and no reader gets a copy.
pub fn encrypt<'r>(
    follower: &Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    (channel, agent): (ChannelId, Option<&str>),
    readers: &'r [Reader],
    content: impl FnOnce(MessageId, NonZeroU64) -> Content,
) -> Result<(MessageId, Vec<&'r Reader>), String> {
    encrypted(
        follower,
        (roster, devices),
        channel,
        Sending::Message(agent, readers),
        content,
    )?
}

/// As [`encrypt`], a receipt that reports the receipts `issued`, by their `seq`, the device owed,
/// `held` ones if `held`, otherwise `read` or `refused` ones: it goes out with `report` false, and
/// the device owes them no more, all at once.
/// Gives why it waits, as [`encrypt`] says why it sends nothing, and fails only if the device or
/// its state does.
pub fn encrypt_receipt(
    follower: &Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    (channel, issued, held): (ChannelId, Vec<u64>, bool),
    content: impl FnOnce(MessageId, NonZeroU64) -> Content,
) -> Result<Result<MessageId, String>, String> {
    let sent = encrypted(
        follower,
        (roster, devices),
        channel,
        Sending::Receipt(issued, held),
        content,
    )?;
    Ok(sent.map(|(msg_id, _)| msg_id))
}

/// As [`encrypt`], the result of the task `task` its claim held (`docs/protocol/queues.md`,
/// "Results"), from `agent` if given: in the channel's group when `sender`, the task's sending
/// device, is a member device of its current epoch, otherwise under `hpke` to `sender`, with the
/// counter the device keeps for it in the channel and `cert`, its certificate. It has no copies,
/// and nothing on this device keeps it; the device's claim of the task ends with it. Gives why it
/// waits, as [`encrypt`] says why it sends nothing, and fails only if the device or its state
/// does.
pub fn encrypt_result(
    follower: &Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    (channel, agent, task): (ChannelId, Option<&str>, TaskId),
    (sender, cert): (DeviceId, Option<&Cert>),
    content: impl FnOnce(MessageId, NonZeroU64) -> Content,
) -> Result<Result<MessageId, String>, String> {
    let sending = Sending::Result {
        agent,
        task,
        sender,
        cert,
    };
    let sent = encrypted(follower, (roster, devices), channel, sending, content)?;
    Ok(sent.map(|(msg_id, _)| msg_id))
}

/// What the device encrypts in a channel.
enum Sending<'a, 'r> {
    /// A message of its user's, from the agent if given, with its copies to the readers.
    Message(Option<&'a str>, &'r [Reader]),
    /// A receipt, with the `seq` of each receipt it reports, `held` ones if true.
    Receipt(Vec<u64>, bool),
    /// The result of the task `task`, from the agent if given, to the task's sending device
    /// `sender`, of the certificate `cert` if the device has a valid one.
    Result {
        agent: Option<&'a str>,
        task: TaskId,
        sender: DeviceId,
        cert: Option<&'a Cert>,
    },
}

/// [`encrypt`] of what `sending` says; gives why it sends nothing in place of the message ID and
/// the readers that get their first copy, and fails only if the device or its state does.
fn encrypted<'r>(
    follower: &Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    channel: ChannelId,
    sending: Sending<'_, 'r>,
    content: impl FnOnce(MessageId, NonZeroU64) -> Content,
) -> Result<Result<(MessageId, Vec<&'r Reader>), String>, String> {
    let keys = follower.keys()?;
    let admitted = admitted(follower, roster, devices, channel);
    let waiting = (follower.state.mls_outbox().map_err(text)?)
        .iter()
        .any(|outgoing| outgoing.channel == channel);
    let (space, version) = (roster.body().space, follower.connection.version());
    follower.state.mls_change(|sealed| {
        let mut device = load(follower, &keys, sealed)?;
        // Why nothing is sent, which changes nothing.
        let unsent = |why: String| Ok((None, MlsChanges::default(), Err(why)));
        let members = match device.members(channel) {
            Ok(members) => members,
            Err(MlsError::NoGroup(_)) => {
                return unsent(format!(
                    "this device is in no group of the channel {channel} yet; nothing is sent"
                ));
            }
            Err(err) => return Err(failed(channel, err)),
        };
        // A result to a device outside the group goes under `hpke`, whatever the group holds.
        let hpke_to = match &sending {
            Sending::Result { sender, cert, .. }
                if !members.iter().any(|member| member.device == *sender) =>
            {
                let Some(cert) = cert else {
                    return unsent(format!(
                        "the task's sending device {sender} is outside the group of the channel \
                         {channel} and has no valid certificate, so its result has no key to go \
                         to"
                    ));
                };
                Some(*cert)
            }
            _ => None,
        };
        let strangers: Vec<String> = (unadmitted(&admitted, &members).iter())
            .map(ToString::to_string)
            .collect();
        if hpke_to.is_none() && !strangers.is_empty() {
            return unsent(format!(
                "the group of the channel {channel} holds the devices {}, which are not admitted \
                 to the channel; nothing is sent there until the server stores this device's \
                 Commit that removes them",
                strangers.join(", ")
            ));
        }
        if hpke_to.is_none() && waiting {
            return unsent(format!(
                "this device's Commit in the channel {channel} waits for the server's answer; \
                 nothing is sent there until the server stores it"
            ));
        }
        let last = match hpke_to {
            Some(cert) => (follower.state).copy_counter(channel, cert.body().device),
            None => follower.state.channel_counter(channel),
        }
        .map_err(text)?;
        let counter = last
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or_else(|| format!("the counter in the channel {channel} has no next"))?;
        let msg_id = MessageId::generate();
        let content = content(msg_id, counter);
        let (own, gets) = match &sending {
            Sending::Message(agent, _) => {
                let key = key_packages::state_key(&follower.secrets, sealed)?;
                let seq = follower.state.next_own_seq().map_err(text)?;
                let at = ((space, channel, msg_id), seq);
                let (own, gets) = recv::own_message(follower, &key, at, (&content, *agent))?;
                (Some(own), gets)
            }
            Sending::Receipt(..) | Sending::Result { .. } => (None, false),
        };
        let (agent, readers, issued, ended) = match sending {
            Sending::Message(agent, readers) => (agent.map(str::to_owned), readers, None, None),
            Sending::Receipt(issued, held) => (None, &[][..], Some((issued, held)), None),
            Sending::Result { agent, task, .. } => {
                (agent.map(str::to_owned), &[][..], None, Some(task))
            }
        };
        let alone = members.iter().all(|member| member.device == follower.this);
        if hpke_to.is_none() && alone && !gets && readers.is_empty() {
            return unsent(format!(
                "no device can receive the message: the group of the channel {channel} holds no \
                 other device, no agent of this device in the space, nor its person, gets it, and \
                 no service agent outside the group gets a copy; nothing is sent"
            ));
        }
        let epoch = device.epoch(channel).map_err(|err| failed(channel, err))?;
        let judgments = follower.state.judgments(channel).map_err(text)?;
        let entry = history::entry(
            (&device, &keys),
            (space, channel, epoch),
            &judgments,
            msg_id,
            &content,
        );
        let (envelope, recipient) = match hpke_to {
            Some(cert) => {
                let to = cert.body().device;
                let header = Header {
                    v: version,
                    space,
                    channel,
                    msg_id,
                    scheme: Scheme::Hpke,
                    recipients: Some(NonEmpty::new(to)),
                    ttl: None,
                    unknown: Unknown::default(),
                };
                let payload = seal_message(&header, &content, keys.signing(), &cert.body().hpke)
                    .map_err(|err| format!("the result to the device {to} does not seal: {err}"))?;
                (Envelope { header, payload }.to_cbor(), Some(to))
            }
            None => {
                let payload = device
                    .encrypt(channel, &content.to_cbor())
                    .map_err(|err| failed(channel, err))?;
                let envelope = mls_envelope(version, (space, channel), msg_id, payload, None);
                (envelope, None)
            }
        };
        let message = Counted {
            outgoing: Outgoing {
                msg_id,
                recipient,
                space,
                envelope,
                report: issued.is_none(),
            },
            channel,
            counter: counter.get(),
            agent,
            addressee: content.to.as_ref().map(ToString::to_string),
        };
        // A receipt is no message of the device's user: nothing reports on it; nor on a result.
        let tracked = (issued.is_none() && ended.is_none()).then(|| {
            let to = (&members[..], readers);
            message_status::tracked(follower.this, (roster, devices), msg_id, to)
        });
        let mut messages = vec![message];
        let mut first = Vec::new();
        for reader in readers {
            let copy = copy((follower, &keys), version, reader, &messages[0], &content)?;
            if copy.counter == 1 {
                first.push(reader);
            }
            messages.push(copy);
        }
        let (issued, issued_held) = match issued {
            Some((issued, true)) => (Vec::new(), issued),
            Some((issued, false)) => (issued, Vec::new()),
            None => (Vec::new(), Vec::new()),
        };
        let changes = MlsChanges {
            messages,
            entries: entry.into_iter().collect(),
            issued,
            issued_held,
            own: own.into_iter().collect(),
            tracked: tracked.into_iter().collect(),
            ended: ended.into_iter().collect(),
            ..MlsChanges::default()
        };
        Ok((Some(device.seal()), changes, Ok((msg_id, first))))
    })
}

/// The copy under `hpke` to `reader`, sent with the protocol `version` by the device of `keys`, of
/// `message`, whose content is `content`: that content with the next counter the device keeps for
/// `reader` in the channel.
fn copy(
    (follower, keys): (&Follower, &Keys),
    version: NonZeroU64,
    reader: &Reader,
    message: &Counted,
    content: &Content,
) -> Result<Counted, String> {
    let (channel, Outgoing { msg_id, space, .. }) = (message.channel, &message.outgoing);
    let last = (follower.state)
        .copy_counter(channel, reader.device)
        .map_err(text)?;
    let counter = last
        .checked_add(1)
        .and_then(NonZeroU64::new)
        .ok_or_else(|| format!("the counter to the device {} has no next", reader.device))?;
    let header = Header {
        v: version,
        space: *space,
        channel,
        msg_id: *msg_id,
        scheme: Scheme::Hpke,
        recipients: Some(NonEmpty::new(reader.device)),
        ttl: None,
        unknown: Unknown::default(),
    };
    let content = Content {
        counter: Some(counter),
        ..content.clone()
    };
    let payload = seal_message(&header, &content, keys.signing(), &reader.hpke).map_err(|err| {
        format!(
            "the copy to the service agent {} does not seal: {err}",
            reader.agent
        )
    })?;
    Ok(Counted {
        outgoing: Outgoing {
            envelope: Envelope { header, payload }.to_cbor(),
            recipient: Some(reader.device),
            ..message.outgoing.clone()
        },
        counter: counter.get(),
        ..message.clone()
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
