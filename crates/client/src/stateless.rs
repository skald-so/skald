//! A stateless service agent's `skald send` and `skald enqueue` (`docs/service-agents.md`;
//! `docs/protocol/delivery.md`, "Send"): its run keeps no state ([`crate::service`]), so it is a
//! member of no group and sends under `hpke` only.
//!
//! - It takes the channel's latest GroupInfo from the server and checks it as a browser does before
//!   it writes (`docs/protocol/membership.md`, "Wraps"): of the channel, its signature verifying
//!   with a member leaf's key, every member it lists admitted.
//! - It seals one `hpke` envelope to each member device it lists, with the same `msg_id`, the
//!   content from the agent's entry with its token's grant and without `counter`.
//! - It fails unless the server stores every envelope within the run: nothing waits for a later
//!   one.
//! - Its `skald enqueue` seals the task to the `key` of the server's queue key object of the epoch
//!   of the channel's latest GroupInfo, only once that GroupInfo passes the checks above, lists the
//!   object's `signer` as a member and the object's signature verifies with the certificate of
//!   `signer` ([`queue_key`]); the content is from the agent's entry with its token's grant and
//!   without `counter`. It fails unless the server stores the task within the run.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use skald_crypto::{ChannelView, check_queue_key, seal_message};
use skald_proto::{
    Cbor as _, Cert, ChannelId, DeviceId, Envelope, Frame, Header, MessageId, NonEmpty, QueueKey,
    RawEnvelope, Roster, Scheme, ServerName, Unknown,
};

use crate::follow::SpaceDevices;
use crate::groups;
use crate::queue_keys;
use crate::send::{self, Answer, Draft, Key, Reported, Shown, Wire};
use crate::service;
use crate::spaces::Follower;
use crate::state::State;

/// An envelope of the run, as it goes out: no outbox keeps it.
struct Sealed {
    recipient: DeviceId,
    msg_id: MessageId,
    channel: ChannelId,
    frame: Vec<u8>,
}

impl Wire for Sealed {
    fn key(&self) -> Key {
        (self.msg_id, Some(self.recipient))
    }

    fn frame(&self) -> &[u8] {
        &self.frame
    }

    fn answered(&self, _: &State, _: &Answer) {}
}

impl Shown for Sealed {
    fn channel(&self) -> ChannelId {
        self.channel
    }

    fn reports(&self) -> bool {
        true
    }

    fn kept(&self) -> bool {
        false
    }
}

/// Sends `draft` to `channel` of `roster`, the space's last accepted revision with `devices`, its
/// accounts' devices, from the agent's entry on `server`, as the module says, and reports as
/// `reported` says. Fails, sending nothing, if the grant or the GroupInfo fails its checks.
pub fn send(
    follower: &mut Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    (channel, server): (ChannelId, ServerName),
    draft: &Draft,
    reported: Reported,
) -> Result<(), String> {
    let (from, grant) = service::sender(follower, (roster, devices), (channel, server))?;
    let recipients = recipients(follower, (roster, devices), channel)?;
    let msg_id = MessageId::generate();
    let content = skald_proto::Content {
        counter: None,
        grant: Some(grant),
        ..send::content(&from, draft, msg_id, NonZeroU64::MIN)
    };
    let keys = follower.keys()?;
    let mut sealed = Vec::new();
    for (recipient, hpke) in recipients {
        let header = Header {
            v: follower.connection.version(),
            space: roster.body().space,
            channel,
            msg_id,
            scheme: Scheme::Hpke,
            recipients: Some(NonEmpty::new(recipient)),
            ttl: None,
            unknown: Unknown::default(),
        };
        let payload = seal_message(&header, &content, keys.signing(), &hpke)
            .map_err(|err| format!("the message to the device {recipient} does not seal: {err}"))?;
        let envelope =
            RawEnvelope::from_bytes(Envelope { header, payload }.to_cbor()).map_err(|err| {
                format!("the message to the device {recipient} does not encode: {err}")
            })?;
        let frame = Frame::Send {
            envelope,
            report: true,
        };
        sealed.push(Sealed {
            recipient,
            msg_id,
            channel,
            frame: frame.to_cbor(),
        });
    }
    let timeout = follower.timeout;
    let delivered = send::deliver(follower, &sealed, timeout, None);
    send::report(&follower.state, &sealed, &delivered, timeout, reported)
}

/// The member devices of the latest GroupInfo of `channel`, each with the HPKE key of its
/// certificate, once that GroupInfo passes its checks against `roster`, the space's last accepted
/// revision, and `devices`, its accounts' devices.
fn recipients(
    follower: &mut Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    channel: ChannelId,
) -> Result<Vec<(DeviceId, [u8; 32])>, String> {
    let (view, admitted) = checked_group(follower, (roster, devices), channel, NOTHING_SENT)?;
    Ok((view.members().iter())
        .map(|member| (member.device, admitted[&member.device].body().hpke))
        .collect())
}

/// What a run that sends nothing ends its reason with.
const NOTHING_SENT: &str = "nothing is sent";

/// What a run that enqueues nothing ends its reason with.
const NOTHING_ENQUEUED: &str = "nothing is enqueued";

/// The latest GroupInfo of `channel` and the devices `roster`, the space's last accepted revision,
/// and `devices`, its accounts' devices, admit to the channel, once the GroupInfo passes its
/// checks: of the channel, its signature verifying with a member leaf's key, every member it lists
/// admitted. A refusal says why, ending with `nothing`.
fn checked_group(
    follower: &mut Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    channel: ChannelId,
    nothing: &str,
) -> Result<(ChannelView, BTreeMap<DeviceId, Cert>), String> {
    let space = roster.body().space;
    let (_, group_info) = groups::group_on_server(follower, space, channel)?;
    let group_info = group_info.ok_or_else(|| {
        format!(
            "the server has no GroupInfo of the channel {channel}, so this run knows no member \
             device of it; {nothing}"
        )
    })?;
    let view = groups::observe(channel, &group_info).map_err(|err| format!("{err}; {nothing}"))?;
    let admitted = groups::admitted(follower, roster, devices, channel);
    let strangers: Vec<String> = (view.members().iter())
        .filter(|member| !groups::admits(&admitted, member))
        .map(|member| member.device.to_string())
        .collect();
    if !strangers.is_empty() {
        return Err(format!(
            "the GroupInfo of the channel {channel} lists the devices {}, which are not admitted \
             to the channel; {nothing}",
            strangers.join(", ")
        ));
    }
    Ok((view, admitted))
}

/// The epoch of the latest GroupInfo of `channel` and the `key` of the server's queue key object of
/// that epoch, which a stateless run seals its task to (`docs/protocol/queues.md`, "Queue key"):
/// only once the GroupInfo passes its checks against `roster` and `devices`, as [`checked_group`]
/// says, lists the object's `signer` as a member and the object's signature verifies with the
/// certificate of `signer`. Fails, saying why, otherwise.
pub fn queue_key(
    follower: &mut Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    channel: ChannelId,
) -> Result<(u64, [u8; 32]), String> {
    let (view, admitted) = checked_group(follower, (roster, devices), channel, NOTHING_ENQUEUED)?;
    let epoch = view.epoch();
    let keys = follower.keys()?;
    let at = (roster.body().space, channel);
    let bytes = (queue_keys::held(follower, &keys, at, epoch)
        .map_err(|err| format!("{err}; {NOTHING_ENQUEUED}"))?)
    .ok_or_else(|| {
        format!(
            "the server keeps no queue key object of the epoch {epoch} of the channel {channel}; \
             a member device's next run uploads it; {NOTHING_ENQUEUED}"
        )
    })?;
    let object = QueueKey::from_cbor(&bytes).map_err(|_| {
        format!(
            "the server gave a queue key object of the channel {channel} that does not decode; \
             {NOTHING_ENQUEUED}"
        )
    })?;
    let body = object.body();
    if body.channel != channel || body.epoch != epoch {
        return Err(format!(
            "the server gave the queue key object of the epoch {} of the channel {} for the epoch \
             {epoch} of the channel {channel}, that of its latest GroupInfo; {NOTHING_ENQUEUED}",
            body.epoch, body.channel
        ));
    }
    let signer = body.signer;
    let cert = (view.members().iter())
        .any(|member| member.device == signer)
        .then(|| admitted.get(&signer))
        .flatten()
        .ok_or_else(|| {
            format!(
                "the queue key object of the channel {channel} is signed by the device {signer}, \
                 which the channel's latest GroupInfo does not list as a member; \
                 {NOTHING_ENQUEUED}"
            )
        })?;
    check_queue_key(&object, &cert.body().signing).map_err(|_| {
        format!(
            "the signature of the queue key object of the channel {channel} does not verify with \
             the certificate of its signer, the device {signer}; {NOTHING_ENQUEUED}"
        )
    })?;
    Ok((epoch, body.key))
}
