//! How a run shows the messages and statuses it shows: in the person's full lines, in an agent's
//! short ones, or, with `--json`, as a JSON object per line with every field. A short line names a
//! message by its reference, `#<ref>`, and holds no ID.
//!
//! A message's reference: the leading characters of the base32 of RFC 4648, in lowercase and
//! without padding, of SHA-256 of its `msg_id`'s 16 bytes; at least [`SHORTEST`], more only where
//! another message the device keeps shares them. Any device computes it from the `msg_id`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use skald_proto::MessageId;

use crate::connection::printable;

/// The fewest characters of a reference.
pub const SHORTEST: usize = 4;

/// The base32 alphabet of RFC 4648, in lowercase.
const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// How a run shows what it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// The person's full lines.
    Full,
    /// An agent's short lines.
    Short,
    /// A JSON object per line, with every field.
    Json,
}

impl Form {
    /// The form of a run as an agent, if `agent`, with `--json`, if `json`.
    pub fn of(agent: bool, json: bool) -> Self {
        match (json, agent) {
            (true, _) => Form::Json,
            (false, true) => Form::Short,
            (false, false) => Form::Full,
        }
    }
}

/// The whole reference of `msg_id`: each reference of the message is a prefix of it.
pub fn whole_reference(msg_id: MessageId) -> String {
    let hash = Sha256::digest(msg_id.as_bytes());
    // Each character takes the next 5 of the hash's 256 bits; the last one, 1 bit and 4 zero bits.
    (0..256_usize.div_ceil(5))
        .map(|index| {
            let (byte, shift) = (index * 5 / 8, index * 5 % 8);
            let next = hash.get(byte + 1).copied().unwrap_or(0);
            let pair = (u16::from(hash[byte]) << 8) | u16::from(next);
            char::from(BASE32[usize::from((pair >> (11 - shift)) & 31)])
        })
        .collect()
}

/// The references of messages among those the device keeps.
pub struct References {
    /// The whole reference of each message the device keeps, by its `msg_id`.
    kept: Vec<(MessageId, String)>,
}

impl References {
    /// The references among `kept`, the `msg_id`s of the messages the device keeps.
    pub fn new(kept: impl IntoIterator<Item = MessageId>) -> Self {
        References {
            kept: (kept.into_iter())
                .map(|msg_id| (msg_id, whole_reference(msg_id)))
                .collect(),
        }
    }

    /// The reference of `msg_id`: the fewest leading characters of its whole reference, at least
    /// [`SHORTEST`], that no other kept message's shares.
    pub fn of(&self, msg_id: MessageId) -> String {
        let whole = whole_reference(msg_id);
        let shared = (self.kept.iter())
            .filter(|(other, _)| *other != msg_id)
            .map(|(_, other)| {
                (whole.bytes().zip(other.bytes()))
                    .take_while(|(one, other)| one == other)
                    .count()
            })
            .max()
            .unwrap_or(0);
        let length = (shared + 1).clamp(SHORTEST, whole.len());
        whole[..length].to_owned()
    }
}

/// A message that passed the checks, as the device keeps it, sealed, until it is shown: each ID in
/// its text form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub msg_id: String,
    pub space: String,
    pub channel: String,
    /// Its content's `from`.
    pub from: String,
    /// The sending device's ID, and its certificate's `name`.
    pub device: String,
    pub device_name: String,
    pub text: String,
    /// Whether it follows missing messages from its sending device.
    pub after_missing: bool,
}

impl Message {
    /// How `form` shows it, `reference` its reference.
    pub fn shown(&self, form: Form, reference: &str) -> String {
        match form {
            Form::Full => {
                let after = if self.after_missing {
                    ", following missing messages"
                } else {
                    ""
                };
                format!(
                    "The message {} in the channel {} from {}, by the device {} ({}){after}:\n  {}",
                    self.msg_id,
                    self.channel,
                    self.from,
                    printable(&self.device_name),
                    self.device,
                    printable(&self.text)
                )
            }
            Form::Short => {
                let after = if self.after_missing {
                    " (after missing)"
                } else {
                    ""
                };
                format!(
                    "[{} · {}] #{reference}{after}: {}",
                    self.from,
                    printable(&self.device_name),
                    printable(&self.text)
                )
            }
            Form::Json => object(
                "message",
                json!({
                    "msg_id": self.msg_id,
                    "ref": reference,
                    "space": self.space,
                    "channel": self.channel,
                    "from": self.from,
                    "device": self.device,
                    "device_name": self.device_name,
                    "text": self.text,
                    "after_missing": self.after_missing,
                }),
            ),
        }
    }
}

/// A server status of a message the device sent, as the device keeps it until it is shown: each
/// ID in its text form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub msg_id: String,
    /// The message's space and channel, if the device recorded them.
    pub space: Option<String>,
    pub channel: Option<String>,
    /// `delivered` or `expired`.
    pub status: String,
    /// The recipient device's ID, and its certificate's `name` if the device knows it.
    pub device: String,
    pub device_name: Option<String>,
    pub timestamp: u64,
}

/// The status of a message the server expired before a recipient device got it.
pub const EXPIRED: &str = "expired";

impl Status {
    /// How `form` shows it, `reference` its message's reference: an agent's short form shows only
    /// `expired`.
    pub fn shown(&self, form: Form, reference: &str) -> Option<String> {
        let device = &self.device;
        let expired = self.status == EXPIRED;
        Some(match form {
            Form::Full => {
                let what = if expired {
                    format!("expired before the device {device} got it")
                } else {
                    format!("delivered to the device {device}")
                };
                match &self.channel {
                    Some(channel) => {
                        format!(
                            "The message {} in the channel {channel}: {what}",
                            self.msg_id
                        )
                    }
                    None => format!("The message {}: {what}", self.msg_id),
                }
            }
            Form::Short if expired => {
                let name = self.device_name.as_deref().map_or_else(
                    || "a device".to_owned(),
                    |name| format!("the device {}", printable(name)),
                );
                format!("skald: #{reference} expired before {name} got it")
            }
            Form::Short => return None,
            Form::Json => object(
                "status",
                json!({
                    "msg_id": self.msg_id,
                    "ref": reference,
                    "space": self.space,
                    "channel": self.channel,
                    "status": self.status,
                    "device": self.device,
                    "device_name": self.device_name,
                    "timestamp": self.timestamp,
                }),
            ),
        })
    }
}

/// The JSON line of `fields`, an object, with its `type` first.
pub fn object(kind: &str, fields: Value) -> String {
    let mut object = serde_json::Map::new();
    object.insert("type".to_owned(), Value::from(kind));
    if let Value::Object(fields) = fields {
        object.extend(fields);
    }
    Value::Object(object).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reference is RFC 4648's base32 of SHA-256 of the `msg_id`'s 16 bytes, in lowercase and
    /// without padding.
    #[test]
    fn whole_reference_is_base32_of_sha256_of_the_msg_id() {
        let msg_id: MessageId = "01a1dcab-eecf-768d-821b-95f7a4a8bd19".parse().unwrap();
        let whole = whole_reference(msg_id);
        // By `printf <the 16 bytes> | openssl dgst -sha256 -binary | base32`, in lowercase, without `=`.
        assert_eq!(
            whole,
            "4fd4bpdrwlevgtf76yr7aop7sd6psps6bink4745fqh6ymsio3ua"
        );
        assert_eq!(whole.len(), 52);
    }

    /// A reference has 4 characters unless another kept message shares them: then one more than
    /// the longest they share, and distinct.
    #[test]
    fn reference_is_four_characters_unless_another_kept_message_shares_them() {
        let ids: Vec<MessageId> = (0..2000).map(|_| MessageId::generate()).collect();
        let (one, other) = (0..ids.len())
            .flat_map(|i| (0..i).map(move |j| (i, j)))
            .map(|(i, j)| (ids[i], ids[j]))
            .find(|(one, other)| whole_reference(*one)[..4] == whole_reference(*other)[..4])
            .expect("two of 2000 share 4 characters");
        let shared = (whole_reference(one)
            .bytes()
            .zip(whole_reference(other).bytes()))
        .take_while(|(a, b)| a == b)
        .count();
        let apart = (ids.iter().copied())
            .find(|id| whole_reference(*id)[..4] != whole_reference(one)[..4])
            .unwrap();

        let references = References::new([one, other, apart]);

        assert_eq!(references.of(one), whole_reference(one)[..shared + 1]);
        assert_eq!(references.of(other), whole_reference(other)[..shared + 1]);
        assert_ne!(references.of(one), references.of(other));
        assert_eq!(references.of(apart), whole_reference(apart)[..4]);
        let alone = References::new([one]);
        assert_eq!(alone.of(one), whole_reference(one)[..4]);
        assert_eq!(References::new([]).of(other), whole_reference(other)[..4]);
    }

    fn message() -> Message {
        Message {
            msg_id: "01a1dcab-eecf-768d-821b-95f7a4a8bd19".to_owned(),
            space: "01a1dcab-ee8a-7872-9adc-6efd0851e3e0".to_owned(),
            channel: "01a1dcab-ee8a-7872-9adc-6efd0851e3ee".to_owned(),
            from: "acme/bob/user@skald.example".to_owned(),
            device: "a90f5ea9-75e5-899a-90ec-3a09f33aebdf".to_owned(),
            device_name: "bob\u{7}box".to_owned(),
            text: "hi\nthere".to_owned(),
            after_missing: false,
        }
    }

    /// The person's line names every ID; an agent's names none, its text and device name escaped;
    /// either marks a message after missing ones.
    #[test]
    fn message_shows_in_full_short_and_as_json() {
        let mut message = message();

        assert_eq!(
            message.shown(Form::Full, "6bx7"),
            "The message 01a1dcab-eecf-768d-821b-95f7a4a8bd19 in the channel \
             01a1dcab-ee8a-7872-9adc-6efd0851e3ee from acme/bob/user@skald.example, by the device \
             bob\\u{7}box (a90f5ea9-75e5-899a-90ec-3a09f33aebdf):\n  hi\\nthere"
        );
        assert_eq!(
            message.shown(Form::Short, "6bx7"),
            "[acme/bob/user@skald.example · bob\\u{7}box] #6bx7: hi\\nthere"
        );
        let json: Value = serde_json::from_str(&message.shown(Form::Json, "6bx7")).unwrap();
        assert_eq!(
            json,
            json!({
                "type": "message",
                "msg_id": "01a1dcab-eecf-768d-821b-95f7a4a8bd19",
                "ref": "6bx7",
                "space": "01a1dcab-ee8a-7872-9adc-6efd0851e3e0",
                "channel": "01a1dcab-ee8a-7872-9adc-6efd0851e3ee",
                "from": "acme/bob/user@skald.example",
                "device": "a90f5ea9-75e5-899a-90ec-3a09f33aebdf",
                "device_name": "bob\u{7}box",
                "text": "hi\nthere",
                "after_missing": false,
            })
        );

        message.after_missing = true;
        assert!(
            (message.shown(Form::Full, "6bx7"))
                .contains("(a90f5ea9-75e5-899a-90ec-3a09f33aebdf), following missing messages:")
        );
        assert_eq!(
            message.shown(Form::Short, "6bx7"),
            "[acme/bob/user@skald.example · bob\\u{7}box] #6bx7 (after missing): hi\\nthere"
        );
        let json: Value = serde_json::from_str(&message.shown(Form::Json, "6bx7")).unwrap();
        assert_eq!(json["after_missing"], true);
    }

    fn status(status: &str) -> Status {
        Status {
            msg_id: "01a1dcab-eecf-768d-821b-95f7a4a8bd19".to_owned(),
            space: Some("01a1dcab-ee8a-7872-9adc-6efd0851e3e0".to_owned()),
            channel: Some("01a1dcab-ee8a-7872-9adc-6efd0851e3ee".to_owned()),
            status: status.to_owned(),
            device: "a90f5ea9-75e5-899a-90ec-3a09f33aebdf".to_owned(),
            device_name: Some("bob-box".to_owned()),
            timestamp: 1_800_000_000,
        }
    }

    /// A status line names the status, the message, its channel if known, and the recipient
    /// device; an agent's names `expired` by the reference and the device's name, and no
    /// `delivered`.
    #[test]
    fn status_shows_in_full_short_and_as_json() {
        let delivered = status("delivered");
        let mut expired = status(EXPIRED);

        assert_eq!(
            delivered.shown(Form::Full, "6bx7").unwrap(),
            "The message 01a1dcab-eecf-768d-821b-95f7a4a8bd19 in the channel \
             01a1dcab-ee8a-7872-9adc-6efd0851e3ee: delivered to the device \
             a90f5ea9-75e5-899a-90ec-3a09f33aebdf"
        );
        assert_eq!(delivered.shown(Form::Short, "6bx7"), None);
        assert_eq!(
            expired.shown(Form::Short, "6bx7").unwrap(),
            "skald: #6bx7 expired before the device bob-box got it"
        );
        let json: Value =
            serde_json::from_str(&delivered.shown(Form::Json, "6bx7").unwrap()).unwrap();
        assert_eq!(
            json,
            json!({
                "type": "status",
                "msg_id": "01a1dcab-eecf-768d-821b-95f7a4a8bd19",
                "ref": "6bx7",
                "space": "01a1dcab-ee8a-7872-9adc-6efd0851e3e0",
                "channel": "01a1dcab-ee8a-7872-9adc-6efd0851e3ee",
                "status": "delivered",
                "device": "a90f5ea9-75e5-899a-90ec-3a09f33aebdf",
                "device_name": "bob-box",
                "timestamp": 1_800_000_000,
            })
        );

        expired.channel = None;
        expired.device_name = None;
        assert_eq!(
            expired.shown(Form::Full, "6bx7").unwrap(),
            "The message 01a1dcab-eecf-768d-821b-95f7a4a8bd19: expired before the device \
             a90f5ea9-75e5-899a-90ec-3a09f33aebdf got it"
        );
        assert_eq!(
            expired.shown(Form::Short, "6bx7").unwrap(),
            "skald: #6bx7 expired before a device got it"
        );
    }

    /// A run shows in full as the person, short as an agent, and as JSON with `--json`.
    #[test]
    fn form_follows_the_agent_and_json() {
        assert_eq!(Form::of(false, false), Form::Full);
        assert_eq!(Form::of(true, false), Form::Short);
        assert_eq!(Form::of(false, true), Form::Json);
        assert_eq!(Form::of(true, true), Form::Json);
    }
}
