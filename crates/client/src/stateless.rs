//! A stateless service agent's `skald send` (`docs/service-agents.md`; `docs/protocol/delivery.md`,
//! "Send"): its run keeps no state ([`crate::service`]), so it is a member of no group and sends
//! under `hpke` only.
//!
//! - It takes the channel's latest GroupInfo from the server and checks it as a browser does before
//!   it writes (`docs/protocol/membership.md`, "Wraps"): of the channel, its signature verifying
//!   with a member leaf's key, every member it lists admitted.
//! - It seals one `hpke` envelope to each member device it lists, with the same `msg_id`, the
//!   content from the agent's entry with its token's grant and without `counter`.
//! - It fails unless the server stores every envelope within the run: nothing waits for a later
//!   one.

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::time::Duration;

use serde_json::json;
use skald_crypto::seal_message;
use skald_proto::{
    Cbor as _, ChannelId, DeviceId, Envelope, Frame, Header, MessageId, NonEmpty, RawEnvelope,
    Roster, Scheme, ServerName, Unknown,
};

use crate::follow::SpaceDevices;
use crate::forms::{self, References};
use crate::groups;
use crate::send::{self, Answer, Draft, Key, Reported, Wire};
use crate::service;
use crate::spaces::Follower;
use crate::state::State;

/// An envelope of the run, as it goes out: no outbox keeps it.
struct Sealed {
    recipient: DeviceId,
    msg_id: MessageId,
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
    let grant = service::grant(&mut follower.connection, &follower.token)?;
    let at = (follower.this, channel);
    let from = service::sender(at, (roster, devices), &follower.own, server, &grant)?;
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
            frame: frame.to_cbor(),
        });
    }
    let timeout = follower.timeout;
    let delivered = send::deliver(follower, &sealed, timeout, None);
    let answers: BTreeMap<DeviceId, Option<&Answer>> = (sealed.iter())
        .map(|one| (one.recipient, delivered.answer(&one.key())))
        .collect();
    show(msg_id, channel, &answers, reported);
    outcome(msg_id, &answers, (timeout, delivered.failed()))
}

/// The member devices of the latest GroupInfo of `channel`, each with the HPKE key of its
/// certificate, once that GroupInfo passes its checks against `roster`, the space's last accepted
/// revision, and `devices`, its accounts' devices.
fn recipients(
    follower: &mut Follower,
    (roster, devices): (&Roster, &SpaceDevices),
    channel: ChannelId,
) -> Result<Vec<(DeviceId, [u8; 32])>, String> {
    let space = roster.body().space;
    let (_, group_info) = groups::group_on_server(follower, space, channel)?;
    let group_info = group_info.ok_or_else(|| {
        format!(
            "the server has no GroupInfo of the channel {channel}, so this run knows no member \
             device to send to; nothing is sent"
        )
    })?;
    let view =
        groups::observe(channel, &group_info).map_err(|err| format!("{err}; nothing is sent"))?;
    let admitted = groups::admitted(follower, roster, devices, channel);
    let strangers: Vec<String> = (view.members().iter())
        .filter(|member| !groups::admits(&admitted, member))
        .map(|member| member.device.to_string())
        .collect();
    if !strangers.is_empty() {
        return Err(format!(
            "the GroupInfo of the channel {channel} lists the devices {}, which are not admitted \
             to the channel; nothing is sent",
            strangers.join(", ")
        ));
    }
    Ok((view.members().iter())
        .map(|member| (member.device, admitted[&member.device].body().hpke))
        .collect())
}

/// Says what the server answered to each envelope of the message `msg_id` in `channel`, by its
/// recipient device, as `reported` says.
fn show(
    msg_id: MessageId,
    channel: ChannelId,
    answers: &BTreeMap<DeviceId, Option<&Answer>>,
    reported: Reported,
) {
    match reported {
        Reported::Json(_) => {
            let envelopes: Vec<serde_json::Value> = (answers.iter())
                .map(|(device, answer)| match answer {
                    Some(Answer::Stored) => {
                        json!({ "recipient": device.to_string(), "answer": "stored" })
                    }
                    Some(Answer::Refused(problem)) => json!({
                        "recipient": device.to_string(),
                        "answer": "refused",
                        "problem": problem,
                    }),
                    None => json!({ "recipient": device.to_string(), "answer": "unanswered" }),
                })
                .collect();
            let fields = json!({
                "msg_id": msg_id.to_string(),
                "ref": References::new([msg_id]).of(msg_id),
                "channel": channel.to_string(),
                "envelopes": envelopes,
            });
            println!("{}", forms::object("sent", fields));
        }
        Reported::Quiet => {}
        Reported::Full | Reported::Sent(_) => {
            println!("The message {msg_id} to the channel {channel}:");
            for (device, answer) in answers {
                match answer {
                    Some(Answer::Stored) => println!("  {device}  stored"),
                    Some(Answer::Refused(problem)) => println!("  {device}  refused: {problem}"),
                    None => println!("  {device}  no answer"),
                }
            }
        }
    }
}

/// Fails unless the server stored every envelope of the message `msg_id`, by `answers`, saying
/// which it refused or has not answered within `timeout`, and why the last attempt failed, if
/// `failed`.
fn outcome(
    msg_id: MessageId,
    answers: &BTreeMap<DeviceId, Option<&Answer>>,
    (timeout, failed): (Duration, Option<&str>),
) -> Result<(), String> {
    let mut problems = Vec::new();
    let mut unanswered = Vec::new();
    for (device, answer) in answers {
        match answer {
            Some(Answer::Stored) => {}
            Some(Answer::Refused(problem)) => {
                problems.push(format!(
                    "to the device {device}: {}",
                    crate::connection::printable(problem)
                ));
            }
            None => unanswered.push(device.to_string()),
        }
    }
    if !unanswered.is_empty() {
        let why = failed.map_or_else(String::new, |err| format!(" ({err})"));
        problems.push(format!(
            "to {}: no answer within {} s{why}",
            unanswered.join(", "),
            timeout.as_secs()
        ));
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the server has not stored the message {msg_id} {}; a stateless run keeps nothing, so \
         nothing sends it later",
        problems.join("; ")
    ))
}
