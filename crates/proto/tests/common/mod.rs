//! The values of the test vectors in `docs/protocol/test-vectors.md`, shared by the tests.

// Each test binary uses a part of these helpers.
#![allow(dead_code)]

use std::fmt::Debug;
use std::num::NonZeroU64;

use skald_proto::{
    AccountBody, AccountId, AccountRecord, Address, Attachment, Body, Cbor, ChannelId, Content,
    DecodeError, Delegation, DelegationBody, DeliveryStatus, DeviceId, Envelope, FieldProblem,
    Frame, Grant, GrantBody, Header, HistoryEntry, HistoryItem, HpkePayload, HpkePlaintext,
    MasterBackup, MessageId, NonEmpty, Push, RawEnvelope, Receipt, ReceiptStatus, RequestProof,
    Revocation, RevocationBody, Scheme, SendKey, ServerStatus, SpaceId, Stored, Unknown,
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

/// Asserts that `result` fails with `problem` in a field.
pub fn assert_field_problem<T: Debug>(result: Result<T, DecodeError>, problem: &FieldProblem) {
    match result {
        Err(DecodeError::Field {
            problem: actual, ..
        }) => assert_eq!(&actual, problem),
        other => panic!("expected {problem:?}, got {other:?}"),
    }
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

/// A `service` or `custodian` device.
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
        delegation: None,
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

pub fn delegation() -> Delegation {
    Delegation {
        body: DelegationBody {
            agent: address("acme/alice/hosted-mcp@skald.example"),
            custodian: server_device(),
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

/// Alice's account.
pub fn account() -> AccountId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e9".parse().unwrap()
}

/// A revocation of Bob's device on Alice's account, signed by `signer` or, with `None`, the master
/// key.
pub fn revocation(
    keep: Option<NonEmpty<DeviceId>>,
    signer: Option<DeviceId>,
    also: Option<NonEmpty<DeviceId>>,
) -> Revocation {
    let body = RevocationBody {
        account: account(),
        device: bob_device(),
        keep,
        signer,
        timestamp: 1_762_819_200,
        also,
        unknown: Unknown::default(),
    };
    Revocation::from_parts(body.to_cbor(), SIGNATURE).unwrap()
}

/// The account record of [`account`] at revision 1: its master key "TEST 1", its recovery HPKE
/// key `DeriveKeyPair` of the 32 bytes 00 to 1f, signed by "TEST 1".
fn account_record() -> AccountRecord {
    let body = AccountBody {
        account: account(),
        master: hex(KEY_1).try_into().unwrap(),
        recovery: hex("b1f1b840de7a3241b02748cf9b05b74dc8c5e8451298738817bd76aa8ebe8c2b")
            .try_into()
            .unwrap(),
        revision: 1,
        unknown: Unknown::default(),
    };
    let signature = hex(
        "d38b3cd73ed94bec2c5e1e01da5b9b917a0ecf6f1e7a6d4a64d741ce774dbbe4
         7440b0119e58628732add7f9ad417bb32fc4d62b6c41f1843701ba14d778ac01",
    );
    AccountRecord::from_parts(body.to_cbor(), signature.try_into().unwrap()).unwrap()
}

/// The master private key "TEST 1" sealed to the recovery key 00 to 1f on [`account`], with the
/// ephemeral key pair `DeriveKeyPair(ikmE)` of RFC 9180, Appendix A.1.2.
fn master_backup() -> MasterBackup {
    MasterBackup {
        enc: hex("0ad0950d9fb9588e59690b74f1237ecdf1d775cd60be2eca57af5a4b0471c91b")
            .try_into()
            .unwrap(),
        ct: hex(
            "0ab1244bbe800adae44ce071a7489d8519f58458edf9b4d1e6c376adc97b8c6c
             fd6dfd3e6ec094b32961bb7fc2b97e44",
        ),
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
    let mls_envelope = Envelope {
        header: header(msg(1), Scheme::Mls),
        payload: hex("000100020102030405060708090a0b0c0d0e0f"),
    };
    let hpke_envelope = Envelope {
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
    };
    let raw = |envelope: &Envelope| RawEnvelope::from_bytes(envelope.to_cbor()).unwrap();
    let status = |seq, status| Push::Status {
        seq: nonzero(seq),
        status: delivery_status(status),
    };
    vec![
        vector("Envelope under `mls`", mls_envelope.clone()),
        vector("Envelope under `hpke`", hpke_envelope.clone()),
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
            "Content from a custodian device",
            Content {
                counter: Some(nonzero(3)),
                delegation: Some(delegation()),
                ..content(
                    address("acme/alice/hosted-mcp@skald.example"),
                    Body::A2a(a2a(msg(6), "Done")),
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
        vector(
            "Frame `send`",
            Frame::Send {
                envelope: raw(&mls_envelope),
                report: true,
            },
        ),
        vector(
            "Frame `send-refused`",
            Frame::SendRefused {
                send_key: SendKey {
                    msg_id: msg(2),
                    recipient: Some(bob_device()),
                },
                problem: "unknown-recipient".to_owned(),
            },
        ),
        vector("Frame `sync`", Frame::Sync { cursor: 7 }),
        vector(
            "Frame `message`",
            Frame::Push(Push::Message {
                seq: nonzero(8),
                envelope: raw(&hpke_envelope),
            }),
        ),
        vector(
            "Frame `status`",
            Frame::Push(status(9, ServerStatus::Delivered)),
        ),
        vector("Frame `ack`", Frame::Ack { seq: nonzero(9) }),
        vector(
            "Frame `batch`",
            Frame::Batch(vec![
                status(10, ServerStatus::Expired),
                Push::SpaceNotice { space: space() },
            ]),
        ),
        vector(
            "Frame `space-notice`",
            Frame::Push(Push::SpaceNotice { space: space() }),
        ),
        vector(
            "Frame `auth`",
            Frame::Auth {
                challenge: (0..32).collect(),
                device: alice_device(),
                signature: SIGNATURE,
            },
        ),
        vector(
            "Frame `history`",
            Frame::History {
                upload: 5,
                space: space(),
                channel: channel(),
                item: HistoryItem::Entry {
                    msg_id: msg(1),
                    entry: HistoryEntry {
                        channel: channel(),
                        epoch: 2,
                        nonce: [0x0e; 12],
                        ciphertext: hex("c0c1c2c3c4c5c6c7c8c9cacbcccdcecf"),
                        unknown: Unknown::default(),
                    }
                    .to_cbor(),
                },
            },
        ),
        vector("Frame `history-stored`", Frame::HistoryStored { upload: 5 }),
        vector(
            "Frame `history-refused`",
            Frame::HistoryRefused {
                upload: 6,
                problem: "epoch-mismatch".to_owned(),
            },
        ),
        vector(
            "HTTPS proof",
            RequestProof {
                challenge: (0..32).collect(),
                device: alice_device(),
                signature: SIGNATURE,
            },
        ),
        vector(
            "Revocation by a device",
            revocation(
                Some(NonEmpty::new(server_device())),
                Some(alice_device()),
                None,
            ),
        ),
        vector(
            "Revocation by the master key",
            revocation(None, None, Some(NonEmpty::new(server_device()))),
        ),
        vector("Account record", account_record()),
        vector("Master backup", master_backup()),
    ]
}
