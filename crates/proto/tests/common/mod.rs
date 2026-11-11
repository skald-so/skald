//! The values of the test vectors in `docs/protocol/test-vectors.md`, shared by the tests.

// Each test binary uses a part of these helpers.
#![allow(dead_code)]

use std::fmt::Debug;
use std::num::NonZeroU64;

use skald_proto::{
    Address, Attachment, Body, Cbor, ChannelId, Content, DecodeError, DeliveryStatus, DeviceId,
    Envelope, Grant, GrantBody, Header, HpkePayload, HpkePlaintext, MessageId, NonEmpty, Receipt,
    ReceiptStatus, Scheme, SendKey, ServerStatus, SpaceId, Stored, Unknown,
};

/// The bytes of a hex string; whitespace is ignored.
pub fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .map(|digit| match digit {
            b'0'..=b'9' => digit - b'0',
            b'a'..=b'f' => digit - b'a' + 10,
            _ => panic!("{:?} is not a lowercase hex digit", char::from(digit)),
        })
        .collect();
    assert!(
        digits.len().is_multiple_of(2),
        "an odd number of hex digits"
    );
    digits
        .chunks(2)
        .map(|pair| pair[0] * 16 + pair[1])
        .collect()
}

/// The lowercase hex of `bytes`.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The public keys of RFC 8032, §7.1, "TEST 1", "TEST 2" and "TEST 3".
const KEY_1: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const KEY_2: &str = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";
const KEY_3: &str = "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025";

fn device(key: &str) -> DeviceId {
    DeviceId::from_signing_key(&hex(key).try_into().unwrap())
}

/// Alice's `client` device.
pub fn alice_device() -> DeviceId {
    device(KEY_1)
}

/// Bob's `client` device.
pub fn bob_device() -> DeviceId {
    device(KEY_2)
}

/// A `service` device.
pub fn server_device() -> DeviceId {
    device(KEY_3)
}

pub fn space() -> SpaceId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e6".parse().unwrap()
}

pub fn channel() -> ChannelId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e7".parse().unwrap()
}

pub fn other_channel() -> ChannelId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e8".parse().unwrap()
}

/// The ID of the `n`th message of the vectors, from 1.
pub fn msg(n: u8) -> MessageId {
    format!("019a6f20-1a00-7c00-8a00-0000000000{n:02x}")
        .parse()
        .unwrap()
}

pub fn address(text: &str) -> Address {
    text.parse().unwrap()
}

pub fn alice() -> Address {
    address("acme/alice/claude-api@skald.example")
}

pub fn bob() -> Address {
    address("acme/bob/codex@skald.example")
}

pub fn nonzero(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

/// A placeholder signature: the vectors fix the encoding, not the cryptography.
pub const SIGNATURE: [u8; 64] = [0x5a; 64];

pub fn header(msg_id: MessageId, scheme: Scheme) -> Header {
    Header {
        v: nonzero(1),
        space: space(),
        channel: channel(),
        msg_id,
        scheme,
        recipients: None,
        ttl: None,
        unknown: Unknown::default(),
    }
}

/// An A2A message in JSON with this `messageId` and these parts, each in JSON.
fn a2a_parts(msg_id: MessageId, parts: &[String]) -> String {
    format!(
        r#"{{"messageId":"{msg_id}","contextId":"019a6f20-0000-7000-8000-00000000c0de","role":"ROLE_USER","parts":[{}]}}"#,
        parts.join(",")
    )
}

/// An A2A message in JSON with this `messageId` and text.
pub fn a2a(msg_id: MessageId, text: &str) -> String {
    a2a_parts(msg_id, &[format!(r#"{{"text":"{text}"}}"#)])
}

/// An A2A message in JSON with this `messageId`, text and the file part of [`attachment`].
pub fn a2a_with_attachment(msg_id: MessageId, text: &str) -> String {
    a2a_parts(
        msg_id,
        &[
            format!(r#"{{"text":"{text}"}}"#),
            format!(
                r#"{{"url":"urn:uuid:{}","filename":"pr-12.diff","mediaType":"text/x-diff"}}"#,
                attachment().blob
            ),
        ],
    )
}

/// Content from `from` with `body`, `counter` 1 and no other optional field.
pub fn content(from: Address, body: Body) -> Content {
    Content {
        from,
        to: None,
        counter: Some(nonzero(1)),
        hops: 0,
        reply_to: None,
        body: Some(body),
        grant: None,
        attachments: None,
        unknown: Unknown::default(),
    }
}

pub fn attachment() -> Attachment {
    Attachment {
        blob: "019a6f20-1b00-7d00-8b00-00000000b10b".parse().unwrap(),
        key: [0x6b; 16],
        digest: [0xd1; 32],
        size: 1234,
        unknown: Unknown::default(),
    }
}

pub fn grant() -> Grant {
    Grant {
        body: GrantBody {
            service: server_device(),
            channels: NonEmpty::from_vec(vec![channel(), other_channel()]).unwrap(),
            signer: alice_device(),
            expires: 1_798_761_600,
            unknown: Unknown::default(),
        },
        signature: SIGNATURE,
    }
}

pub fn receipt(status: ReceiptStatus, messages: Vec<MessageId>, reason: Option<&str>) -> Content {
    Content {
        to: Some(alice()),
        counter: Some(nonzero(5)),
        ..content(
            bob(),
            Body::Receipt(Receipt {
                status,
                messages: NonEmpty::from_vec(messages).unwrap(),
                reason: reason.map(str::to_owned),
                unknown: Unknown::default(),
            }),
        )
    }
}

/// `stored` for the envelope of the `hpke` vector.
pub fn stored() -> Stored {
    Stored {
        send_key: SendKey {
            msg_id: msg(2),
            recipient: Some(bob_device()),
        },
        timestamp: 1_762_819_200,
    }
}

pub fn delivery_status(status: ServerStatus) -> DeliveryStatus {
    DeliveryStatus {
        msg_id: msg(1),
        device: bob_device(),
        status,
        timestamp: 1_762_819_200,
        unknown: Unknown::default(),
    }
}

/// Decodes bytes as a vector's type and encodes the result again.
pub type Decode = Box<dyn Fn(&[u8]) -> Result<Vec<u8>, DecodeError>>;

/// A test vector: a value of some [`Cbor`] type and its encoding.
pub struct Vector {
    pub name: &'static str,
    pub bytes: Vec<u8>,
    pub decode: Decode,
}

fn vector<T: Cbor + PartialEq + Debug + 'static>(name: &'static str, value: T) -> Vector {
    let bytes = value.to_cbor();
    assert_eq!(T::from_cbor(&bytes).as_ref(), Ok(&value), "{name}");
    Vector {
        name,
        bytes,
        decode: Box::new(|bytes| T::from_cbor(bytes).map(|value| value.to_cbor())),
    }
}

/// Every test vector, in the order of `docs/protocol/test-vectors.md`.
pub fn vectors() -> Vec<Vector> {
    let hpke_content = Content {
        to: Some(bob()),
        ..content(alice(), Body::A2a(a2a(msg(2), "Review PR 12")))
    };
    let hpke_plaintext = HpkePlaintext {
        sender: alice_device(),
        content: hpke_content,
        signature: SIGNATURE,
    };
    vec![
        vector(
            "Envelope under `mls`",
            Envelope {
                header: header(msg(1), Scheme::Mls),
                payload: hex("000100020102030405060708090a0b0c0d0e0f"),
            },
        ),
        vector(
            "Envelope under `hpke`",
            Envelope {
                header: Header {
                    recipients: Some(NonEmpty::new(bob_device())),
                    ttl: Some(86_400),
                    ..header(msg(2), Scheme::Hpke)
                },
                payload: HpkePayload {
                    enc: [0xe0; 32],
                    ct: hex("c0c1c2c3c4c5c6c7c8c9cacbcccdcecf"),
                }
                .to_cbor(),
            },
        ),
        vector("HPKE plaintext", hpke_plaintext),
        vector(
            "Content with an A2A message and an attachment",
            Content {
                to: Some(bob()),
                counter: Some(nonzero(7)),
                hops: 1,
                reply_to: Some(msg(3)),
                attachments: Some(NonEmpty::new(attachment())),
                ..content(
                    alice(),
                    Body::A2a(a2a_with_attachment(msg(4), "Here is the diff")),
                )
            },
        ),
        vector(
            "Content from a service device",
            Content {
                counter: None,
                grant: Some(grant()),
                ..content(
                    address("acme/ci/deploy@skald.example"),
                    Body::A2a(a2a(msg(5), "Deployed")),
                )
            },
        ),
        vector(
            "Receipt `held`",
            receipt(ReceiptStatus::Held, vec![msg(4)], None),
        ),
        vector(
            "Receipt `refused`",
            receipt(
                ReceiptStatus::Refused,
                vec![msg(4)],
                Some("Inbound mode is refuse"),
            ),
        ),
        vector(
            "Receipt `read`",
            receipt(ReceiptStatus::Read, vec![msg(1), msg(4)], None),
        ),
        vector("Server status `stored`", stored()),
        vector(
            "Server status `delivered`",
            delivery_status(ServerStatus::Delivered),
        ),
        vector(
            "Server status `expired`",
            delivery_status(ServerStatus::Expired),
        ),
    ]
}
