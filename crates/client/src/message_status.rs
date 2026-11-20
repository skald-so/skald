//! `skald message-status` and the MCP tool `message_status` ([`crate::mcp`]): where a message is
//! (`docs/protocol/delivery.md`, "Device to agent" and "Receipts"). They read the local state
//! alone.
//!
//! - For each channel message it sends, the device keeps the devices it goes to: the other member
//!   devices of the channel's group at sending, and each service device outside the group that
//!   gets a copy ([`crate::copies`]). Each is `stored` once the device takes the server's `stored`,
//!   then `delivered` or `expired` as the server says, each with its time, no earlier than the
//!   `stored`. Of its `to` agent, it keeps the times of `held`, `read`, `replied` and `refused`,
//!   with its reason; on one device, the handover to that agent is its `read` ([`crate::state`]).
//! - Of each message its person's run showed or it handed to an agent, it keeps, sealed with what
//!   a reply needs ([`crate::reply`]), its sending device, the trust level it gave that device and
//!   its time: the time its `msg_id` states as a UUIDv7, or else when the device took it in; and the
//!   agents it was handed to.
//! - It keeps both for the message's wait: its envelope's `ttl`, at most [`recv::AGENT_WAIT`],
//!   after the server stored it, or after the device wrote it.
//! - The command and the tool take a message as `skald reply` does, by `#<ref>`, `<ref>` or its
//!   `msg_id`, and give its `msg_id`, channel, sending device and time; of a message the device
//!   sent, each device it went to, by its name and account, with its status and time, and its `to`
//!   agent's statuses; of one it showed or handed over, the sending device's trust level and the
//!   agents it was handed to. The person's form is in full, an agent's one compact line, `--json`
//!   one object with every field. An unknown message, or a reference two kept messages share,
//!   gives nothing and says why.
//! - As an agent, only its own messages and those handed to it, in its folder's space; what it
//!   says of any other names nothing of it.

use std::path::Path;

use serde_json::{Value, json};
use skald_crypto::MemberDevice;
use skald_proto::{
    AccountId, Address, Cert, ChannelId, DeviceId, MessageId, Name, Roster, SpaceId, TrustLevel,
};

use crate::connection::printable;
use crate::copies::Reader;
use crate::devices::text;
use crate::follow::{self, SpaceDevices};
use crate::forms::{self, Form, References, bracketed};
use crate::history_reading::rfc3339;
use crate::key_packages;
use crate::recv;
use crate::reply::{self, Origin, Sealed};
use crate::secrets::{self, Secrets};
use crate::state::{self, Recipient, RecipientStatus, State, Target, Tracked};

/// Where the message `msg_id`, which the device `this` sends in `channel` of `roster`, the space's
/// last accepted revision with `devices`, its accounts' devices, goes: to each of `members`, the
/// channel's group, but this device, and to each of `readers`, which get a copy.
pub fn tracked(
    this: DeviceId,
    (roster, devices): (&Roster, &SpaceDevices),
    msg_id: MessageId,
    (members, readers): (&[MemberDevice], &[Reader]),
) -> Tracked {
    let known = |device: DeviceId, copy: bool| {
        let cert = (devices.certs.iter())
            .map(Cert::body)
            .find(|body| body.device == device);
        let member = cert.and_then(|body| roster.body().member(body.account));
        Recipient {
            device,
            copy,
            name: cert.map(|body| body.name.clone()),
            account: cert.map(|body| body.account.to_string()),
            handle: member.map(|member| member.handle.to_string()),
        }
    };
    let recipients = (members.iter())
        .map(|member| member.device)
        .filter(|&device| device != this)
        .map(|device| known(device, false))
        .chain(readers.iter().map(|reader| known(reader.device, true)))
        .collect();
    Tracked {
        msg_id,
        wait: recv::AGENT_WAIT.as_secs(),
        recipients,
    }
}

/// Where the message `msg_id` came from: the device `device` named `name`, at `level`, at the time
/// its `msg_id` states as a UUIDv7, or else now.
pub fn origin(msg_id: MessageId, (device, name): (DeviceId, &str), level: TrustLevel) -> Origin {
    Origin {
        device: device.to_string(),
        device_name: name.to_owned(),
        level: level.name().to_owned(),
        time: stated_time(msg_id).unwrap_or_else(state::unix_now),
    }
}

/// The time `msg_id` states, in Unix seconds, if it is a UUIDv7: its first 48 bits, in
/// milliseconds.
fn stated_time(msg_id: MessageId) -> Option<u64> {
    let bytes = msg_id.as_bytes();
    (bytes[6] >> 4 == 7).then(|| {
        let millis = bytes[..6]
            .iter()
            .fold(0_u64, |millis, &byte| (millis << 8) | u64::from(byte));
        millis / 1000
    })
}

/// Shows where the message `message` names, by `#<ref>`, `<ref>` or its `msg_id`, is, as `agent`
/// if given, whose run holds its warnings for the person, or as the person, by the state in `dir`,
/// its secrets in `storage`; as JSON if `json`.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    message: &str,
    agent: Option<&Name>,
    json: bool,
) -> Result<(), String> {
    recv::holding_warnings(dir, agent.is_some(), || {
        let named = reply::given(message)?;
        let (_, reader) = reply::scope_of(dir, agent)?;
        let shown = printable(message.trim());
        let unknown = || match reader {
            Some(_) => format!(
                "{shown} is no message this agent sent that the server stored, nor one handed \
                 over to it, in its folder's space, or its wait ended"
            ),
            None => format!(
                "{shown} is no message this device sent that the server stored, nor one it \
                 showed or handed over to an agent, or its wait ended"
            ),
        };
        if !dir.join(state::FILE).is_file() {
            return Err(unknown());
        }
        let state = State::open(dir).map_err(text)?;
        let now = state::unix_now();
        let sent = state.sent_ids(reader, now).map_err(text)?;
        let handed = state.handed(reader, now).map_err(text)?;
        let mut ids: Vec<MessageId> = (sent.iter().copied())
            .chain(handed.iter().map(|target| target.msg_id))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let kept = state.kept_ids().map_err(text)?;
        let msg_id = reply::pick(&named, ids, |msg_id| *msg_id, &kept).map_err(|shared| {
            if shared {
                format!(
                    "{shown} is the reference of more than one message this device keeps: give \
                     more of it, or the msg_id"
                )
            } else {
                unknown()
            }
        })?;
        let target = handed.into_iter().find(|target| target.msg_id == msg_id);
        let status = gathered(
            dir,
            storage,
            &state,
            (msg_id, sent.contains(&msg_id)),
            target,
        )?;
        let reference = References::new(kept).of(msg_id);
        println!(
            "{}",
            status.shown(Form::of(agent.is_some(), json), &reference)
        );
        Ok(())
    })
}

/// Where a message is, as the device keeps it.
struct Status {
    msg_id: MessageId,
    space: SpaceId,
    channel: ChannelId,
    /// The sending device, its name if known, and the message's time, in Unix seconds.
    device: DeviceId,
    device_name: Option<String>,
    timestamp: u64,
    sent: Option<Sent>,
    received: Option<Received>,
}

/// Where a message the device sent went.
struct Sent {
    /// Its `to`, if any.
    to: Option<String>,
    recipients: Vec<RecipientStatus>,
    held: Option<u64>,
    read: Option<u64>,
    replied: Option<u64>,
    /// Its `to` agent's `refused`, with its reason, if any.
    refused: Option<u64>,
    reason: Option<String>,
}

/// Where a message the device showed or handed over came from, and where it went.
struct Received {
    from: String,
    level: String,
    /// The agents it was handed to, by their full addresses where the device knows them.
    handed_to: Vec<String>,
}

/// Where the message `msg_id` is, by `state` in `dir`, its secrets in `storage`: what the device
/// keeps of it as its sender, if `sent`, and as `target`, a message it showed or handed over, if
/// given.
fn gathered(
    dir: &Path,
    storage: secrets::Storage,
    state: &State,
    (msg_id, sent): (MessageId, bool),
    target: Option<Target>,
) -> Result<Status, String> {
    let this = state.device().map_err(text)?;
    let sent = match sent {
        true => state.sent_status(msg_id).map_err(text)?,
        false => None,
    };
    let opened = match &target {
        Some(target) => {
            let this = this.as_ref().ok_or("this machine has no device")?;
            let secrets = Secrets::new(dir, storage);
            let key = state.mls_step(|sealed| {
                key_packages::state_key(&secrets, sealed).map(|key| (None, key))
            })?;
            let opened = reply::open(&key, this.id, target)
                .ok_or("what this device kept of the message does not open")?;
            let agents = state.handed_to(target.seq).map_err(text)?;
            Some((opened, agents))
        }
        None => None,
    };
    let (space, channel) = match (&sent, &target) {
        (Some(sent), _) => (sent.space, sent.channel),
        (None, Some(target)) => (target.space, target.channel),
        (None, None) => return Err("the device keeps nothing of the message".to_owned()),
    };
    let origin = opened
        .as_ref()
        .and_then(|(opened, _)| opened.origin.clone());
    let (device, device_name, timestamp) = match (&sent, &this, origin) {
        (Some(sent), Some(this), _) => (this.id, Some(this.name.clone()), sent.stored),
        (_, _, Some(origin)) => (
            origin.device.parse().map_err(|_| {
                format!("the local state is malformed: the device {}", origin.device)
            })?,
            Some(origin.device_name),
            origin.time,
        ),
        _ => return Err("the device keeps no sending device of the message".to_owned()),
    };
    let received = opened.map(|(opened, agents)| {
        let level =
            (opened.origin.as_ref()).map_or_else(String::new, |origin| origin.level.clone());
        let handed_to = agents
            .iter()
            .map(|agent| full_address(state, space, &opened, agent))
            .collect();
        Received {
            from: opened.from,
            level,
            handed_to,
        }
    });
    Ok(Status {
        msg_id,
        space,
        channel,
        device,
        device_name,
        timestamp,
        sent: sent.map(|sent| Sent {
            to: sent.addressee,
            recipients: sent.recipients,
            held: sent.held,
            read: sent.read,
            replied: sent.replied,
            refused: sent.refused,
            reason: sent.reason,
        }),
        received,
    })
}

/// The full address of this device's agent `agent` in `space`, where it got the message `opened`:
/// the space and server of its `from` and the account's handle in the space's last accepted
/// revision; `agent` alone if the device does not know them.
fn full_address(state: &State, space: SpaceId, opened: &Sealed, agent: &str) -> String {
    let address = || -> Option<Address> {
        let from: Address = opened.from.parse().ok()?;
        let account: AccountId = state.device().ok()??.account?.parse().ok()?;
        let kept = (state.spaces().ok()?)
            .into_iter()
            .find(|kept| kept.id == space)?;
        let roster = follow::decode(&kept.accepted, &kept.name).ok()?;
        let handle = roster.body().member(account)?.handle.clone();
        Some(Address::new(
            from.space().clone(),
            handle,
            agent.parse().ok()?,
            from.server().clone(),
        ))
    };
    address().map_or_else(|| agent.to_owned(), |address| address.to_string())
}

impl Sent {
    /// Whether its `to` is an agent: one whose `read` and `replied` come.
    fn to_agent(&self) -> bool {
        (self.to.as_deref())
            .and_then(|to| to.parse::<Address>().ok())
            .is_some_and(|to| !to.is_person())
    }
}

impl Status {
    /// How `form` shows it, `reference` its message's reference.
    fn shown(&self, form: Form, reference: &str) -> String {
        match form {
            Form::Full => self.full(),
            Form::Short => self.short(reference),
            Form::Json => self.json(reference),
        }
    }

    /// The person's lines: every ID.
    fn full(&self) -> String {
        let name = self
            .device_name
            .as_deref()
            .map(printable)
            .unwrap_or_default();
        let from = (self.received.as_ref()).map_or_else(String::new, |received| {
            format!(" from {} ({})", printable(&received.from), received.level)
        });
        let mut lines = vec![format!(
            "The message {} in the channel {}{from}, sent at {} by the device {name} ({}):",
            self.msg_id,
            self.channel,
            rfc3339(self.timestamp),
            self.device
        )];
        if let Some(sent) = &self.sent {
            lines.extend(sent.recipients.iter().map(|recipient| {
                let name = recipient.name.as_deref().map(printable).unwrap_or_default();
                let of = (recipient.handle.as_deref())
                    .map_or_else(String::new, |handle| format!(" of {}", printable(handle)));
                format!(
                    "  the device {name} ({}){of}: {} at {}",
                    recipient.device,
                    recipient.status,
                    rfc3339(recipient.timestamp)
                )
            }));
            if sent.recipients.is_empty() {
                lines.push("  no other device".to_owned());
            }
            if let (Some(to), true) = (&sent.to, sent.to_agent()) {
                let statuses = agent_statuses(sent, |time| format!(" at {}", rfc3339(time)));
                lines.push(format!("  to {}: {statuses}", printable(to)));
            }
        }
        if let Some(received) = &self.received {
            lines.push(format!("  {}", handed(&received.handed_to, printable)));
        }
        lines.join("\n")
    }

    /// An agent's line: no ID, each device by its name between brackets.
    fn short(&self, reference: &str) -> String {
        let at = rfc3339(self.timestamp);
        let device = self
            .device_name
            .as_deref()
            .map_or_else(String::new, bracketed);
        let mut line = match (&self.sent, &self.received) {
            (Some(sent), _) => {
                let mut recipients: Vec<String> = (sent.recipients.iter())
                    .map(|recipient| {
                        let name = recipient
                            .name
                            .as_deref()
                            .map_or_else(String::new, bracketed);
                        format!("[{name}] {}", recipient.status)
                    })
                    .collect();
                if recipients.is_empty() {
                    recipients.push("no other device".to_owned());
                }
                let mut line = format!("#{reference} sent at {at}: {}", recipients.join(", "));
                if let (Some(to), true) = (&sent.to, sent.to_agent()) {
                    let statuses = agent_statuses(sent, |_| String::new());
                    let to = printable(&forms::short_address(to));
                    line.push_str(&format!("; to {to}: {statuses}"));
                }
                line
            }
            (None, Some(received)) => format!(
                "#{reference} from {} [{device} · {}] sent at {at}",
                printable(&forms::short_address(&received.from)),
                received.level
            ),
            (None, None) => format!("#{reference} sent at {at}"),
        };
        if let Some(received) = &self.received {
            let names = (received.handed_to.iter())
                .map(|agent| forms::short_address(agent))
                .collect::<Vec<_>>();
            line.push_str(&format!("; {}", handed(&names, printable)));
        }
        line
    }

    /// One JSON object with every field.
    fn json(&self, reference: &str) -> String {
        let sent = self.sent.as_ref();
        let received = self.received.as_ref();
        let recipients = sent.map(|sent| {
            (sent.recipients.iter())
                .map(|recipient| {
                    json!({
                        "device": recipient.device.to_string(),
                        "device_name": recipient.name,
                        "account": recipient.account,
                        "handle": recipient.handle,
                        "status": recipient.status,
                        "timestamp": recipient.timestamp,
                    })
                })
                .collect::<Vec<Value>>()
        });
        let agent = sent.filter(|sent| sent.to_agent()).map(|sent| {
            json!({
                "held": sent.held,
                "read": sent.read,
                "replied": sent.replied,
                "refused": sent.refused,
                "reason": sent.reason,
            })
        });
        forms::object(
            "message_status",
            json!({
                "msg_id": self.msg_id.to_string(),
                "ref": reference,
                "space": self.space.to_string(),
                "channel": self.channel.to_string(),
                "device": self.device.to_string(),
                "device_name": self.device_name,
                "timestamp": self.timestamp,
                "from": received.map(|received| &received.from),
                "to": sent.and_then(|sent| sent.to.as_ref()),
                "recipients": recipients,
                "agent": agent,
                "level": received.map(|received| &received.level),
                "handed_to": received.map(|received| &received.handed_to),
            }),
        )
    }
}

/// The statuses of the `to` agent of `sent`, each with `time` of its time.
fn agent_statuses(sent: &Sent, time: impl Fn(u64) -> String) -> String {
    let mut statuses: Vec<String> = [
        (forms::HELD, sent.held),
        (forms::READ, sent.read),
        (forms::REPLIED, sent.replied),
    ]
    .into_iter()
    .filter_map(|(status, at)| at.map(|at| format!("{status}{}", time(at))))
    .collect();
    if let Some(at) = sent.refused {
        let reason = (sent.reason.as_deref())
            .map_or_else(String::new, |reason| format!(": {}", printable(reason)));
        statuses.push(format!("{}{}{reason}", forms::REFUSED, time(at)));
    }
    if statuses.is_empty() {
        "not handed over yet".to_owned()
    } else {
        statuses.join(", ")
    }
}

/// The agents a message was handed to, `agents`, each as `shown` shows it.
fn handed<S: AsRef<str>>(agents: &[S], shown: impl Fn(&str) -> String) -> String {
    if agents.is_empty() {
        return "handed over to no agent".to_owned();
    }
    let agents: Vec<String> = agents.iter().map(|agent| shown(agent.as_ref())).collect();
    format!("handed over to {}", agents.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A UUIDv7 states its time; another `msg_id` none.
    #[test]
    fn uuid_v7_states_its_time() {
        let msg_id: MessageId = "01a1dcab-eecf-768d-821b-95f7a4a8bd19".parse().unwrap();
        assert_eq!(stated_time(msg_id), Some(0x01a1_dcab_eecf / 1000));
        let v4: MessageId = "01a1dcab-eecf-468d-821b-95f7a4a8bd19".parse().unwrap();
        assert_eq!(stated_time(v4), None);
        let now = state::unix_now();
        let generated = stated_time(MessageId::generate()).unwrap();
        assert!(generated.abs_diff(now) <= 1, "{generated} is not {now}");
    }

    fn recipient(name: &str, status: &str) -> RecipientStatus {
        RecipientStatus {
            device: "a90f5ea9-75e5-899a-90ec-3a09f33aebdf".parse().unwrap(),
            name: Some(name.to_owned()),
            account: Some("0190a1a2-0000-7000-8000-000000000001".to_owned()),
            handle: Some("bob".to_owned()),
            status: status.to_owned(),
            timestamp: 1_800_000_060,
        }
    }

    fn status(sent: Option<Sent>, received: Option<Received>) -> Status {
        Status {
            msg_id: "01a1dcab-eecf-768d-821b-95f7a4a8bd19".parse().unwrap(),
            space: "01a1dcab-ee8a-7872-9adc-6efd0851e3e0".parse().unwrap(),
            channel: "01a1dcab-ee8a-7872-9adc-6efd0851e3ee".parse().unwrap(),
            device: "b90f5ea9-75e5-899a-90ec-3a09f33aebdf".parse().unwrap(),
            device_name: Some("ada-laptop".to_owned()),
            timestamp: 1_800_000_000,
            sent,
            received,
        }
    }

    fn sent() -> Sent {
        Sent {
            to: Some("acme/bob/claude@skald.example".to_owned()),
            recipients: vec![recipient("bob\u{7}box ] #x", "delivered")],
            held: None,
            read: Some(1_800_000_120),
            replied: None,
            refused: None,
            reason: None,
        }
    }

    /// A sent message's lines name each device with its status: the person's with IDs, times and
    /// the account's handle, an agent's by the device's name, escaped, between brackets; its JSON
    /// has every field, with the `to` agent's times.
    #[test]
    fn sent_message_shows_its_devices_and_agent() {
        let shown = status(Some(sent()), None);

        assert_eq!(
            shown.full(),
            "The message 01a1dcab-eecf-768d-821b-95f7a4a8bd19 in the channel \
             01a1dcab-ee8a-7872-9adc-6efd0851e3ee, sent at 2027-01-15T08:00:00Z by the device \
             ada-laptop (b90f5ea9-75e5-899a-90ec-3a09f33aebdf):\n  the device bob\\u{7}box ] #x \
             (a90f5ea9-75e5-899a-90ec-3a09f33aebdf) of bob: delivered at 2027-01-15T08:01:00Z\n  \
             to acme/bob/claude@skald.example: read at 2027-01-15T08:02:00Z"
        );
        assert_eq!(
            shown.short("k2m4"),
            "#k2m4 sent at 2027-01-15T08:00:00Z: [bob\\u{7}box \\u{5d} #x] delivered; to \
             bob/claude: read"
        );
        let json: Value = serde_json::from_str(&shown.json("k2m4")).unwrap();
        assert_eq!(json["type"], "message_status");
        assert_eq!(json["ref"], "k2m4");
        assert_eq!(
            json["agent"],
            json!({
                "held": null,
                "read": 1_800_000_120_u64,
                "replied": null,
                "refused": null,
                "reason": null,
            })
        );
        assert_eq!(json["recipients"][0]["status"], "delivered");
        assert_eq!(json["recipients"][0]["handle"], "bob");
        assert_eq!(json["level"], Value::Null);

        let mut alone = sent();
        alone.recipients.clear();
        alone.read = None;
        let alone = status(Some(alone), None);
        assert!(alone.full().contains(
            "\n  no other device\n  to acme/bob/claude@skald.example: not handed over yet"
        ));
        assert!(
            alone
                .short("k2m4")
                .ends_with(": no other device; to bob/claude: not handed over yet")
        );
        let mut everyone = sent();
        everyone.to = None;
        let everyone = status(Some(everyone), None);
        assert!(!everyone.short("k2m4").contains("; to "));
        assert_eq!(
            serde_json::from_str::<Value>(&everyone.json("k2m4")).unwrap()["agent"],
            Value::Null
        );

        let mut refused = sent();
        (refused.held, refused.read) = (Some(1_800_000_090), None);
        refused.refused = Some(1_800_000_180);
        refused.reason = Some("off \u{7}list".to_owned());
        let refused = status(Some(refused), None);
        assert!(
            refused.full().ends_with(
                "to acme/bob/claude@skald.example: held at 2027-01-15T08:01:30Z, refused at \
                 2027-01-15T08:03:00Z: off \\u{7}list"
            ),
            "{}",
            refused.full()
        );
        assert!(
            refused
                .short("k2m4")
                .ends_with("; to bob/claude: held, refused: off \\u{7}list"),
            "{}",
            refused.short("k2m4")
        );
        let json: Value = serde_json::from_str(&refused.json("k2m4")).unwrap();
        assert_eq!(json["agent"]["held"], 1_800_000_090_u64);
        assert_eq!(json["agent"]["refused"], 1_800_000_180_u64);
        assert_eq!(json["agent"]["reason"], "off \u{7}list");
        assert_eq!(json["agent"]["read"], Value::Null);
    }

    /// A message the device showed or handed over shows its sender, level and the agents it was
    /// handed to: the person's by their full addresses, an agent's short.
    #[test]
    fn received_message_shows_its_sender_and_handover() {
        let received = Received {
            from: "acme/ada/codex@skald.example".to_owned(),
            level: "own".to_owned(),
            handed_to: vec!["acme/ada/claude@skald.example".to_owned()],
        };
        let shown = status(None, Some(received));

        assert_eq!(
            shown.full(),
            "The message 01a1dcab-eecf-768d-821b-95f7a4a8bd19 in the channel \
             01a1dcab-ee8a-7872-9adc-6efd0851e3ee from acme/ada/codex@skald.example (own), sent \
             at 2027-01-15T08:00:00Z by the device ada-laptop \
             (b90f5ea9-75e5-899a-90ec-3a09f33aebdf):\n  handed over to \
             acme/ada/claude@skald.example"
        );
        assert_eq!(
            shown.short("k2m4"),
            "#k2m4 from ada/codex [ada-laptop · own] sent at 2027-01-15T08:00:00Z; handed over \
             to ada/claude"
        );
        let json: Value = serde_json::from_str(&shown.json("k2m4")).unwrap();
        assert_eq!(json["handed_to"], json!(["acme/ada/claude@skald.example"]));
        assert_eq!(json["level"], "own");
        assert_eq!(json["recipients"], Value::Null);
    }
}
