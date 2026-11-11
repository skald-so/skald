//! Envelopes, content and receipts: `docs/protocol/README.md`, "Envelope" and "Content", and
//! `docs/protocol/delivery.md`, "Receipts". Each type round-trips, keeps the fields it does not
//! know, and rejects what its CDDL does not allow.

mod common;

use std::fmt::Debug;

use common::*;
use skald_proto::{
    AddressError, Attachment, Body, Cbor, Content, DecodeError, Delegation, DelegationBody,
    DeliveryStatus, EncodingProblem, Envelope, FieldProblem, Grant, GrantBody, Header, HpkePayload,
    HpkePlaintext, NameError, NonEmpty, Receipt, ReceiptStatus, Scheme, Segment, SendKey,
    ServerStatus, Stored, Unknown,
};

/// The encoding of a 16-byte ID: `h'019a6f1e2b3c7d4e8f5061a2b3c4d5e6'`.
const ID: &str = "50019a6f1e2b3c7d4e8f5061a2b3c4d5e6";
/// `"a/b/c@d"`.
const ADDRESS: &str = "67612f622f6340 64";
/// The encoding of another ID: `h'019a6f1e2b3c7d4e8f5061a2b3c4d5e7'`.
const ID2: &str = "50019a6f1e2b3c7d4e8f5061a2b3c4d5e7";
/// `"a/b/user@d"`, a person address.
const PERSON: &str = "6a612f622f757365724064";
/// `"{}"`, an `a2a` body.
const JSON: &str = "627b7d";
/// The content entry `3: 1`, a `counter`.
const COUNTER: &str = "0301";
/// The content entry `9: {1: 3, 2: [ID]}`, a `receipt` body.
const RECEIPT: &str = "09a2 0103 0281 50019a6f1e2b3c7d4e8f5061a2b3c4d5e6";
/// `{1: "{}"}`, a value for the bodies `Content` keeps in `unknown`.
const OTHER_BODY: &str = "a101627b7d";

fn round_trip<T: Cbor + PartialEq + Debug>(value: &T) {
    let bytes = value.to_cbor();
    assert_eq!(T::from_cbor(&bytes).as_ref(), Ok(value));
}

#[test]
fn values_without_unknown_fields_have_none() {
    let bytes = header(msg(1), Scheme::Mls).to_cbor();
    let header = Header::from_cbor(&bytes).unwrap();
    assert!(header.unknown.is_empty());
    assert_eq!(header.unknown.keys().count(), 0);
}

#[test]
fn non_empty_lists_hold_at_least_one_item() {
    assert_eq!(NonEmpty::<u8>::from_vec(Vec::new()), None);
    let mut list = NonEmpty::new(1);
    list.push(2);
    assert_eq!(&*list, &[1, 2]);
    assert_eq!(list.len(), 2);
    assert_eq!(NonEmpty::from_vec(vec![1, 2]), Some(list.clone()));
    assert_eq!(list.into_vec(), vec![1, 2]);
}

fn field(field: &'static str, problem: FieldProblem) -> Result<(), DecodeError> {
    Err(DecodeError::Field { field, problem })
}

/// Decodes `hex` as a `T` and drops the value.
fn decode<T: Cbor>(text: &str) -> Result<(), DecodeError> {
    T::from_cbor(&hex(text)).map(drop)
}

/// Encodes `value`, decodes the bytes and drops the result.
fn check<T: Cbor>(value: &T) -> Result<(), DecodeError> {
    T::from_cbor(&value.to_cbor()).map(drop)
}

fn embedded(field: &'static str, error: DecodeError) -> Result<(), DecodeError> {
    Err(DecodeError::Field {
        field,
        problem: FieldProblem::Embedded(Box::new(error)),
    })
}

fn json() -> Body {
    Body::A2a("{}".to_owned())
}

/// The header of the `mls` vector, `{1: 1, 2: ID, 3: ID, 4: ID, 5: 1}`, with `extra` entries,
/// already encoded and in key order, after key 5.
fn header_with(entries: usize, extra: &str) -> String {
    format!(
        "{:02x} 0101 02{ID} 03{ID} 04{ID} 0501 {extra}",
        0xa0 + 5 + entries
    )
}

#[test]
fn every_type_round_trips() {
    let minimal = content(alice(), Body::A2a(a2a(msg(1), "hi")));
    round_trip(&minimal);
    round_trip(&Content {
        hops: u64::MAX,
        counter: Some(nonzero(u64::MAX)),
        ..minimal.clone()
    });
    round_trip(&header(msg(1), Scheme::Mls));
    round_trip(&Header {
        recipients: Some(NonEmpty::from_vec(vec![alice_device(), bob_device()]).unwrap()),
        ttl: Some(0),
        v: nonzero(2),
        ..header(msg(1), Scheme::Mls)
    });
    round_trip(&Header {
        recipients: Some(NonEmpty::new(alice_device())),
        ..header(msg(1), Scheme::Hpke)
    });
    round_trip(&Envelope {
        header: header(msg(1), Scheme::Mls),
        payload: Vec::new(),
    });
    round_trip(&Envelope {
        header: header(msg(1), Scheme::Mls),
        payload: vec![0xab; 70_000],
    });
    round_trip(&HpkePayload {
        enc: [1; 32],
        ct: Vec::new(),
    });
    round_trip(&HpkePlaintext {
        sender: bob_device(),
        content: minimal.clone(),
        signature: SIGNATURE,
    });
    round_trip(&grant());
    round_trip(&grant().body);
    round_trip(&delegation());
    round_trip(&delegation().body);
    round_trip(&attachment());
    for status in [
        ReceiptStatus::Held,
        ReceiptStatus::Refused,
        ReceiptStatus::Read,
    ] {
        let Some(Body::Receipt(receipt)) = receipt(status, vec![msg(1)], Some("why")).body else {
            unreachable!()
        };
        round_trip(&receipt);
    }
    for status in [ServerStatus::Delivered, ServerStatus::Expired] {
        round_trip(&delivery_status(status));
    }
    round_trip(&stored());
    let mls_key = SendKey {
        msg_id: msg(1),
        recipient: None,
    };
    round_trip(&mls_key);
    round_trip(&Stored {
        send_key: mls_key,
        timestamp: u64::MAX,
    });
}

#[test]
fn fields_encode_under_the_keys_of_the_spec() {
    let receipt = Receipt {
        status: ReceiptStatus::Read,
        messages: NonEmpty::new(msg(1)),
        reason: Some("x".to_owned()),
        unknown: Unknown::default(),
    };
    assert_eq!(
        to_hex(&receipt.to_cbor()),
        format!("a3 0103 0281{} 036178", to_hex(&cbor_id(msg(1).as_bytes()))).replace(' ', "")
    );
    let status = delivery_status(ServerStatus::Expired);
    assert_eq!(
        to_hex(&status.to_cbor()),
        format!(
            "a4 01{} 02{} 0302 041a69127c80",
            to_hex(&cbor_id(msg(1).as_bytes())),
            to_hex(&cbor_id(bob_device().as_bytes()))
        )
        .replace(' ', "")
    );
}

#[test]
fn stored_is_its_frame() {
    let id = |id: &[u8; 16]| to_hex(&cbor_id(id));
    assert_eq!(
        to_hex(&stored().to_cbor()),
        format!(
            "83 02 82{}{} 1a69127c80",
            id(msg(2).as_bytes()),
            id(bob_device().as_bytes())
        )
        .replace(' ', "")
    );
    let mls = Stored {
        send_key: SendKey {
            msg_id: msg(1),
            recipient: None,
        },
        timestamp: 0,
    };
    assert_eq!(
        to_hex(&mls.to_cbor()),
        format!("83 02 81{} 00", id(msg(1).as_bytes())).replace(' ', "")
    );
}

fn cbor_id(bytes: &[u8; 16]) -> Vec<u8> {
    let mut out = vec![0x50];
    out.extend_from_slice(bytes);
    out
}

#[test]
fn unknown_fields_are_kept_in_key_order() {
    // {0: null, 1: 3, 2: [ID], 24: "x", -1: [true], -18446744073709551616: {}}
    let bytes = hex(&format!(
        "a6 00f6 0103 0281{ID} 18186178 2081f5 3bffffffffffffffffa0"
    ));
    let receipt = Receipt::from_cbor(&bytes).unwrap();
    assert_eq!(receipt.status, ReceiptStatus::Read);
    assert!(!receipt.unknown.is_empty());
    assert_eq!(
        receipt.unknown.keys().collect::<Vec<_>>(),
        [0, 24, -1, -18_446_744_073_709_551_616]
    );
    assert_eq!(receipt.unknown.get(0), Some(&[0xf6][..]));
    assert_eq!(receipt.unknown.get(24), Some(&hex("6178")[..]));
    assert_eq!(receipt.unknown.get(-1), Some(&hex("81f5")[..]));
    assert_eq!(receipt.unknown.get(3), None);
    assert_eq!(receipt.to_cbor(), bytes);

    let bytes = hex(&header_with(1, "08 a1 01 82 01 02"));
    let header = Header::from_cbor(&bytes).unwrap();
    assert_eq!(header.unknown.keys().collect::<Vec<_>>(), [8]);
    assert_eq!(header.to_cbor(), bytes);
}

#[test]
fn content_with_a_body_of_a_later_version_keeps_it() {
    // {1: "a/b/c@d", 4: 0, 15: {1: "{}"}}: no version defines key 15, so it may be a later body.
    // A counter is required only with a body of version 1.
    let bytes = hex(&format!("a3 01{ADDRESS} 0400 0f{OTHER_BODY}"));
    let content = Content::from_cbor(&bytes).unwrap();
    assert_eq!(content.body, None);
    assert_eq!(content.unknown.keys().collect::<Vec<_>>(), [15]);
    assert_eq!(content.to_cbor(), bytes);
}

#[test]
fn content_keeps_the_version_1_bodies_it_does_not_decode() {
    // `task`, `task-result`, `ask` and `ask-answer`.
    for key in [11_u8, 12, 13, 14] {
        let bytes = hex(&format!(
            "a4 01{ADDRESS} {COUNTER} 0400 {key:02x}{OTHER_BODY}"
        ));
        let content = Content::from_cbor(&bytes).unwrap();
        assert_eq!(content.body, None);
        assert_eq!(
            content.unknown.keys().collect::<Vec<_>>(),
            [i128::from(key)]
        );
        assert_eq!(content.to_cbor(), bytes);
    }
}

#[test]
fn content_needs_exactly_one_body() {
    assert_eq!(
        decode::<Content>(&format!("a3 01{ADDRESS} {COUNTER} 0400")),
        field("content", FieldProblem::NoBody)
    );
    assert_eq!(
        decode::<Content>(&format!("a4 01{ADDRESS} {COUNTER} 0400 06{JSON}")),
        Ok(())
    );
    // Every two bodies of version 1: `a2a`, `receipt`, `task`, `task-result`, `ask`, `ask-answer`.
    let bodies = [
        format!("06{JSON}"),
        RECEIPT.to_owned(),
        format!("0b{OTHER_BODY}"),
        format!("0c{OTHER_BODY}"),
        format!("0d{OTHER_BODY}"),
        format!("0e{OTHER_BODY}"),
    ];
    for (i, first) in bodies.iter().enumerate() {
        for second in &bodies[i + 1..] {
            assert_eq!(
                decode::<Content>(&format!(
                    "a6 01{ADDRESS} 02{ADDRESS} {COUNTER} 0400 {first} {second}"
                )),
                field("content", FieldProblem::TwoBodies),
                "{first} and {second}"
            );
        }
    }
}

#[test]
fn content_needs_a_counter_unless_it_is_a_task_or_has_a_grant() {
    assert_eq!(
        decode::<Content>(&format!("a3 01{ADDRESS} 0400 06{JSON}")),
        field("content.counter", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Content>(&format!("a4 01{ADDRESS} 02{ADDRESS} 0400 {RECEIPT}")),
        field("content.counter", FieldProblem::Missing)
    );
    // `task-result`, `ask` and `ask-answer`.
    for key in [12_u8, 13, 14] {
        assert_eq!(
            decode::<Content>(&format!("a3 01{ADDRESS} 0400 {key:02x}{OTHER_BODY}")),
            field("content.counter", FieldProblem::Missing),
            "body {key}"
        );
    }
    // A task is no mailbox message.
    assert_eq!(
        decode::<Content>(&format!("a3 01{ADDRESS} 0400 0b{OTHER_BODY}")),
        Ok(())
    );
    // A stateless service device.
    let from_service = Content {
        counter: None,
        grant: Some(grant()),
        ..content(alice(), json())
    };
    assert_eq!(check(&from_service), Ok(()));
}

#[test]
fn content_has_a_delegation_or_a_grant_not_both() {
    let from_custodian = Content {
        delegation: Some(delegation()),
        ..content(delegation().body.agent, json())
    };
    assert_eq!(check(&from_custodian), Ok(()));
    assert_eq!(
        check(&Content {
            grant: Some(grant()),
            ..from_custodian
        }),
        field("content.grant", FieldProblem::NotAllowed)
    );
    assert_eq!(
        check(&Content {
            grant: Some(grant()),
            ..content(alice(), json())
        }),
        Ok(())
    );
}

#[test]
fn custodians_send_as_the_delegated_agent_and_services_not_as_a_person() {
    let person = address("acme/alice/user@skald.example");
    assert_eq!(
        check(&Content {
            delegation: Some(delegation()),
            ..content(alice(), json())
        }),
        field(
            "content.from",
            FieldProblem::Mismatch("delegation-body.agent")
        )
    );
    assert_eq!(
        check(&Content {
            grant: Some(grant()),
            ..content(person.clone(), json())
        }),
        field("content.from", FieldProblem::NotAllowed)
    );
    // A person is not an agent, so no one delegates it.
    let to_person = DelegationBody {
        agent: person.clone(),
        ..delegation().body
    };
    assert_eq!(
        check(&to_person),
        field("delegation-body.agent", FieldProblem::NotAllowed)
    );
    assert_eq!(
        check(&Content {
            delegation: Some(Delegation {
                body: to_person,
                signature: SIGNATURE
            }),
            ..content(person.clone(), json())
        }),
        embedded(
            "delegation.body",
            DecodeError::Field {
                field: "delegation-body.agent",
                problem: FieldProblem::NotAllowed
            }
        )
    );
    // A `client` or `browser` device sends as a person, with neither.
    assert_eq!(check(&content(person, json())), Ok(()));
}

#[test]
fn signers_are_neither_the_service_nor_the_custodian() {
    assert_eq!(
        check(&GrantBody {
            signer: server_device(),
            ..grant().body
        }),
        field(
            "grant-body.signer",
            FieldProblem::Same("grant-body.service")
        )
    );
    assert_eq!(
        check(&DelegationBody {
            signer: server_device(),
            ..delegation().body
        }),
        field(
            "delegation-body.signer",
            FieldProblem::Same("delegation-body.custodian")
        )
    );
}

#[test]
fn hpke_plaintexts_name_their_sender_in_the_grant_or_the_delegation() {
    let from_service = Content {
        counter: None,
        grant: Some(grant()),
        ..content(alice(), json())
    };
    let from_custodian = Content {
        delegation: Some(delegation()),
        ..content(delegation().body.agent, json())
    };
    let plaintext = |sender, content| HpkePlaintext {
        sender,
        content,
        signature: SIGNATURE,
    };
    assert_eq!(
        check(&plaintext(server_device(), from_service.clone())),
        Ok(())
    );
    assert_eq!(
        check(&plaintext(server_device(), from_custodian.clone())),
        Ok(())
    );
    assert_eq!(
        check(&plaintext(alice_device(), from_service)),
        field(
            "hpke-plaintext.sender",
            FieldProblem::Mismatch("grant-body.service")
        )
    );
    assert_eq!(
        check(&plaintext(alice_device(), from_custodian)),
        field(
            "hpke-plaintext.sender",
            FieldProblem::Mismatch("delegation-body.custodian")
        )
    );
    assert_eq!(
        check(&plaintext(alice_device(), content(alice(), json()))),
        Ok(())
    );
}

#[test]
fn hpke_plaintexts_hold_no_task() {
    let plaintext = |content: &str| HpkePlaintext {
        sender: alice_device(),
        content: Content::from_cbor(&hex(content)).unwrap(),
        signature: SIGNATURE,
    };
    assert_eq!(
        check(&plaintext(&format!("a3 01{ADDRESS} 0400 0b{OTHER_BODY}"))),
        embedded(
            "hpke-plaintext.content",
            DecodeError::Field {
                field: "content.task",
                problem: FieldProblem::NotAllowed
            }
        )
    );
    assert_eq!(
        check(&plaintext(&format!(
            "a5 01{ADDRESS} 02{ADDRESS} {COUNTER} 0400 0c{OTHER_BODY}"
        ))),
        Ok(())
    );
}

#[test]
fn attachments_go_only_with_a_body_that_holds_an_a2a_message() {
    let attachment = format!(
        "0a81 a4 01{ID} 0250{} 035820{} 0400",
        "00".repeat(16),
        "00".repeat(32)
    );
    let other = |key: u8| format!("{key:02x}{OTHER_BODY}");
    // `a2a`, `task`, `task-result`, a body of a later version; `receipt`, `ask`, `ask-answer`.
    for (key, body, allowed) in [
        (6, format!("06{JSON}"), true),
        (11, other(11), true),
        (12, other(12), true),
        (15, other(15), true),
        (9, RECEIPT.to_owned(), false),
        (13, other(13), false),
        (14, other(14), false),
    ] {
        // Map keys go in order: a body before `attachments`, key 10, or after it.
        let entries = if key < 10 {
            format!("{body} {attachment}")
        } else {
            format!("{attachment} {body}")
        };
        let expected = if allowed {
            Ok(())
        } else {
            field("content.attachments", FieldProblem::NotAllowed)
        };
        assert_eq!(
            decode::<Content>(&format!(
                "a6 01{ADDRESS} 02{ADDRESS} {COUNTER} 0400 {entries}"
            )),
            expected,
            "{body}"
        );
    }
    assert_eq!(
        decode::<Content>(&format!(
            "a5 01{ADDRESS} 02{ADDRESS} {COUNTER} 0400 {RECEIPT}"
        )),
        Ok(())
    );
}

#[test]
fn receipt_messages_go_from_an_agent_to_the_sender_with_no_hops() {
    assert_eq!(
        decode::<Content>(&format!("a4 01{ADDRESS} {COUNTER} 0400 {RECEIPT}")),
        field("content.to", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Content>(&format!(
            "a5 01{ADDRESS} 02{ADDRESS} {COUNTER} 0401 {RECEIPT}"
        )),
        field("content.hops", FieldProblem::Value(1))
    );
    assert_eq!(
        decode::<Content>(&format!(
            "a5 01{PERSON} 02{ADDRESS} {COUNTER} 0400 {RECEIPT}"
        )),
        field("content.from", FieldProblem::NotAllowed)
    );
    assert_eq!(
        decode::<Content>(&format!(
            "a5 01{ADDRESS} 02{PERSON} {COUNTER} 0400 {RECEIPT}"
        )),
        Ok(())
    );
    // The rules are the receipt's: an `a2a` body may come from a person, to every agent, after hops.
    assert_eq!(
        decode::<Content>(&format!("a4 01{PERSON} {COUNTER} 0401 06{JSON}")),
        Ok(())
    );
}

#[test]
fn content_rejects_invalid_fields() {
    assert_eq!(
        decode::<Content>(&format!("a2 0400 06{JSON}")),
        field("content.from", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Content>(&format!("a2 01{ADDRESS} 06{JSON}")),
        field("content.hops", FieldProblem::Missing)
    );
    // "A/b/c@d"
    assert_eq!(
        decode::<Content>(&format!("a3 0167412f622f634064 0400 06{JSON}")),
        field(
            "content.from",
            FieldProblem::Address(AddressError::Segment(
                Segment::Space,
                NameError::Uppercase('A')
            ))
        )
    );
    assert_eq!(
        decode::<Content>(&format!("a4 01{ADDRESS} 0266612f622f6364 0400 06{JSON}")),
        field(
            "content.to",
            FieldProblem::Address(AddressError::MissingServer)
        )
    );
    assert_eq!(
        decode::<Content>(&format!("a4 01{ADDRESS} 0300 0400 06{JSON}")),
        field("content.counter", FieldProblem::Zero)
    );
    assert_eq!(
        decode::<Content>(&format!("a3 01{ADDRESS} 0420 06{JSON}")),
        field("content.hops", FieldProblem::Type("an unsigned integer"))
    );
    assert_eq!(
        decode::<Content>(&format!("a4 01{ADDRESS} 0400 054100 06{JSON}")),
        field(
            "content.reply_to",
            FieldProblem::Length {
                expected: 16,
                actual: 1
            }
        )
    );
    assert_eq!(
        decode::<Content>(&format!("a3 01{ADDRESS} 0400 0641ff")),
        field("content.a2a", FieldProblem::Type("a text string"))
    );
    assert_eq!(
        decode::<Content>(&format!("a4 01{ADDRESS} 0400 06{JSON} 0a80")),
        field("content.attachments", FieldProblem::Empty)
    );
    assert_eq!(
        decode::<Content>(&format!("a3 01{ADDRESS} 0400 6178{JSON}")),
        field("content", FieldProblem::KeyType)
    );
    assert_eq!(
        decode::<Content>(&format!("83 01 {ADDRESS} 04")),
        field("content", FieldProblem::Type("a map"))
    );
}

#[test]
fn header_rejects_invalid_fields() {
    assert_eq!(decode::<Header>(&header_with(0, "")), Ok(()));
    assert_eq!(
        decode::<Header>(&format!("a5 0100 02{ID} 03{ID} 04{ID} 0501")),
        field("header.v", FieldProblem::Zero)
    );
    assert_eq!(
        decode::<Header>(&format!("a4 0101 03{ID} 04{ID} 0501")),
        field("header.space", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Header>(&format!("a4 0101 02{ID} 04{ID} 0501")),
        field("header.channel", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Header>(&format!("a4 0101 02{ID} 03{ID} 0501")),
        field("header.msg_id", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Header>(&format!("a4 0101 02{ID} 03{ID} 04{ID}")),
        field("header.scheme", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Header>(&format!("a5 0101 02{ID} 03{ID} 04{ID} 0503")),
        field("header.scheme", FieldProblem::Value(3))
    );
    assert_eq!(
        decode::<Header>(&format!(
            "a5 0101 024f{} 03{ID} 04{ID} 0501",
            "00".repeat(15)
        )),
        field(
            "header.space",
            FieldProblem::Length {
                expected: 16,
                actual: 15
            }
        )
    );
    assert_eq!(
        decode::<Header>(&header_with(1, "0680")),
        field("header.recipients", FieldProblem::Empty)
    );
    assert_eq!(
        decode::<Header>(&header_with(1, &format!("06{ID}"))),
        field("header.recipients", FieldProblem::Type("an array"))
    );
    assert_eq!(
        decode::<Header>(&header_with(1, "07616e")),
        field("header.ttl", FieldProblem::Type("an unsigned integer"))
    );
}

/// A header with `scheme` and the `recipients` entry `recipients`, already encoded, if any.
fn header_for(scheme: u8, recipients: Option<String>) -> String {
    format!(
        "{:02x} 0101 02{ID} 03{ID} 04{ID} 05{scheme:02x} {}",
        0xa5 + usize::from(recipients.is_some()),
        recipients.map_or(String::new(), |list| format!("06{list}"))
    )
}

#[test]
fn hpke_headers_name_exactly_one_recipient() {
    let one = Some(format!("81{ID}"));
    let two = Some(format!("82{ID}{ID}"));
    assert_eq!(decode::<Header>(&header_for(2, one.clone())), Ok(()));
    assert_eq!(
        decode::<Header>(&header_for(2, None)),
        field("header.recipients", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Header>(&header_for(2, two.clone())),
        field(
            "header.recipients",
            FieldProblem::Items {
                expected: 1,
                actual: 2
            }
        )
    );
    for recipients in [None, one, two] {
        assert_eq!(decode::<Header>(&header_for(1, recipients)), Ok(()));
    }
}

#[test]
fn stored_rejects_invalid_fields() {
    let key = format!("82{ID}{ID}");
    assert_eq!(decode::<Stored>(&format!("83 02 {key} 00")), Ok(()));
    assert_eq!(decode::<Stored>(&format!("83 02 81{ID} 00")), Ok(()));
    assert_eq!(
        decode::<Stored>(&format!("83 03 {key} 00")),
        field("stored.type", FieldProblem::Value(3))
    );
    assert_eq!(
        decode::<Stored>(&format!("83 60 {key} 00")),
        field("stored.type", FieldProblem::Type("an unsigned integer"))
    );
    assert_eq!(
        decode::<Stored>(&format!("82 02 {key}")),
        field(
            "stored",
            FieldProblem::Items {
                expected: 3,
                actual: 2
            }
        )
    );
    assert_eq!(
        decode::<Stored>("a0"),
        field("stored", FieldProblem::Type("an array"))
    );
    assert_eq!(
        decode::<Stored>(&format!("83 02 {key} 20")),
        field(
            "stored.timestamp",
            FieldProblem::Type("an unsigned integer")
        )
    );
    assert_eq!(
        decode::<Stored>("83 02 80 00"),
        field("send-key.msg_id", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Stored>(&format!("83 02 83{ID}{ID}{ID} 00")),
        field(
            "send-key",
            FieldProblem::Items {
                expected: 2,
                actual: 3
            }
        )
    );
    assert_eq!(
        decode::<Stored>(&format!("83 02 {ID} 00")),
        field("send-key", FieldProblem::Type("an array"))
    );
    assert_eq!(
        decode::<Stored>("83 02 814100 00"),
        field(
            "send-key.msg_id",
            FieldProblem::Length {
                expected: 16,
                actual: 1
            }
        )
    );
    assert_eq!(
        decode::<Stored>(&format!("83 02 82{ID}4100 00")),
        field(
            "send-key.recipient",
            FieldProblem::Length {
                expected: 16,
                actual: 1
            }
        )
    );
}

#[test]
fn envelope_and_hpke_arrays_need_their_items() {
    let header = header_with(0, "");
    assert_eq!(decode::<Envelope>(&format!("82 {header} 40")), Ok(()));
    assert_eq!(
        decode::<Envelope>(&format!("83 {header} 40 40")),
        field(
            "envelope",
            FieldProblem::Items {
                expected: 2,
                actual: 3
            }
        )
    );
    assert_eq!(
        decode::<Envelope>(&format!("82 {header} 60")),
        field("envelope.payload", FieldProblem::Type("a byte string"))
    );
    assert_eq!(
        decode::<Envelope>(&format!("82 40 {header}")),
        field("header", FieldProblem::Type("a map"))
    );
    assert_eq!(
        decode::<HpkePayload>(&format!("82 5820{} 40", "00".repeat(32))),
        Ok(())
    );
    assert_eq!(
        decode::<HpkePayload>(&format!("82 5821{} 40", "00".repeat(33))),
        field(
            "hpke-payload.enc",
            FieldProblem::Length {
                expected: 32,
                actual: 33
            }
        )
    );
    assert_eq!(
        decode::<HpkePayload>(&format!("81 5820{}", "00".repeat(32))),
        field(
            "hpke-payload",
            FieldProblem::Items {
                expected: 2,
                actual: 1
            }
        )
    );
}

#[test]
fn hpke_envelopes_carry_an_hpke_payload() {
    let envelope = |scheme, payload: &str| Envelope {
        header: Header {
            recipients: Some(NonEmpty::new(bob_device())),
            ..header(msg(1), scheme)
        },
        payload: hex(payload),
    };
    let payload = to_hex(
        &HpkePayload {
            enc: [0xe0; 32],
            ct: vec![0xc0],
        }
        .to_cbor(),
    );
    assert_eq!(check(&envelope(Scheme::Hpke, &payload)), Ok(()));
    assert_eq!(
        check(&envelope(Scheme::Hpke, "ff00")),
        embedded(
            "envelope.payload",
            DecodeError::Encoding {
                at: 0,
                problem: EncodingProblem::Malformed
            }
        )
    );
    assert_eq!(
        check(&envelope(Scheme::Hpke, "")),
        embedded("envelope.payload", DecodeError::Truncated)
    );
    assert_eq!(
        check(&envelope(Scheme::Hpke, "a0")),
        embedded(
            "envelope.payload",
            DecodeError::Field {
                field: "hpke-payload",
                problem: FieldProblem::Type("an array")
            }
        )
    );
    // Under `mls`, the payload is an MLSMessage, which is not CBOR.
    assert_eq!(check(&envelope(Scheme::Mls, "ff00")), Ok(()));
}

#[test]
fn embedded_items_are_checked_like_any_other() {
    let content = format!("a4 01{ADDRESS} {COUNTER} 0400 06{JSON}");
    let signature = format!("5840{}", "5a".repeat(64));
    let embed = |item: &str| {
        let item = hex(item);
        let head = match item.len() {
            len @ 0..24 => format!("{:02x}", 0x40 + len),
            len @ 24..256 => format!("58{len:02x}"),
            len => format!("59{len:04x}"),
        };
        format!("{head}{}", to_hex(&item))
    };
    assert_eq!(
        decode::<HpkePlaintext>(&format!("83 {ID} {} {signature}", embed(&content))),
        Ok(())
    );
    assert_eq!(
        decode::<HpkePlaintext>(&format!(
            "83 {ID} {} {signature}",
            embed(&format!("{content}00"))
        )),
        field(
            "hpke-plaintext.content",
            FieldProblem::Embedded(Box::new(DecodeError::TrailingBytes(1)))
        )
    );
    assert_eq!(
        decode::<HpkePlaintext>(&format!(
            "83 {ID} {} {signature}",
            embed(&format!("a3 01{ADDRESS} {COUNTER} 0400"))
        )),
        field(
            "hpke-plaintext.content",
            FieldProblem::Embedded(Box::new(DecodeError::Field {
                field: "content",
                problem: FieldProblem::NoBody
            }))
        )
    );
    assert_eq!(
        decode::<HpkePlaintext>(&format!("83 {ID} {} 415a", embed(&content))),
        field(
            "hpke-plaintext.signature",
            FieldProblem::Length {
                expected: 64,
                actual: 1
            }
        )
    );
    let grant_body = format!("a4 01{ID} 0281{ID} 03{ID2} 0400");
    assert_eq!(
        decode::<Grant>(&format!("82 {} {signature}", embed(&grant_body))),
        Ok(())
    );
    assert_eq!(
        decode::<Grant>(&format!(
            "82 {} {signature}",
            embed(&format!("a4 01{ID} 0280 03{ID2} 0400"))
        )),
        field(
            "grant.body",
            FieldProblem::Embedded(Box::new(DecodeError::Field {
                field: "grant-body.channels",
                problem: FieldProblem::Empty
            }))
        )
    );
    assert_eq!(
        decode::<Grant>(&format!("82 {grant_body} {signature}")),
        field("grant.body", FieldProblem::Type("a byte string"))
    );
    assert_eq!(
        decode::<Delegation>(&format!(
            "82 {} {signature}",
            embed(&format!("a3 01{ADDRESS} 02{ID} 03{ID2}"))
        )),
        field(
            "delegation.body",
            FieldProblem::Embedded(Box::new(DecodeError::Field {
                field: "delegation-body.expires",
                problem: FieldProblem::Missing
            }))
        )
    );
}

#[test]
fn signed_bodies_encode_to_the_bytes_their_signatures_cover() {
    // A grant body with a field of a later version: the signature covers those exact bytes.
    let body = hex(&format!("a5 01{ID} 0281{ID} 03{ID2} 0400 1864f5"));
    let grant = GrantBody::from_cbor(&body).unwrap();
    assert_eq!(grant.to_cbor(), body);
    let delegation = hex(&format!("a5 01{ADDRESS} 02{ID} 03{ID2} 0400 20f4"));
    assert_eq!(
        DelegationBody::from_cbor(&delegation).unwrap().to_cbor(),
        delegation
    );
}

#[test]
fn receipts_reject_invalid_fields() {
    assert_eq!(
        decode::<Receipt>(&format!("a2 0104 0281{ID}")),
        field("receipt.status", FieldProblem::Value(4))
    );
    assert_eq!(
        decode::<Receipt>(&format!("a2 0100 0281{ID}")),
        field("receipt.status", FieldProblem::Value(0))
    );
    assert_eq!(
        decode::<Receipt>("a1 0103"),
        field("receipt.messages", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Receipt>("a2 0103 0280"),
        field("receipt.messages", FieldProblem::Empty)
    );
    assert_eq!(
        decode::<Receipt>(&format!("a3 0103 0281{ID} 0340")),
        field("receipt.reason", FieldProblem::Type("a text string"))
    );
    assert_eq!(
        decode::<DeliveryStatus>(&format!("a4 01{ID} 02{ID} 0303 0400")),
        field("delivery-status.status", FieldProblem::Value(3))
    );
    assert_eq!(
        decode::<DeliveryStatus>(&format!("a3 01{ID} 02{ID} 0301")),
        field("delivery-status.timestamp", FieldProblem::Missing)
    );
    assert_eq!(
        decode::<Attachment>(&format!("a4 01{ID} 024100 035820{} 0400", "00".repeat(32))),
        field(
            "attachment.key",
            FieldProblem::Length {
                expected: 16,
                actual: 1
            }
        )
    );
}

#[test]
fn receipt_statuses_have_the_values_of_the_spec() {
    for (status, value) in [
        (ReceiptStatus::Held, 1),
        (ReceiptStatus::Refused, 2),
        (ReceiptStatus::Read, 3),
    ] {
        let receipt = Receipt::from_cbor(&hex(&format!("a2 01{value:02x} 0281{ID}"))).unwrap();
        assert_eq!(receipt.status, status);
    }
    for (status, value) in [(ServerStatus::Delivered, 1), (ServerStatus::Expired, 2)] {
        let decoded =
            DeliveryStatus::from_cbor(&hex(&format!("a4 01{ID} 02{ID} 03{value:02x} 0400")))
                .unwrap();
        assert_eq!(decoded.status, status);
    }
    for (scheme, value) in [(Scheme::Mls, 1), (Scheme::Hpke, 2)] {
        let header = Header::from_cbor(&hex(&header_for(value, Some(format!("81{ID}"))))).unwrap();
        assert_eq!(header.scheme, scheme);
    }
}

#[test]
fn errors_name_the_field_and_the_problem() {
    let error = Header::from_cbor(&hex(&format!("a5 0101 02{ID} 03{ID} 04{ID} 0503"))).unwrap_err();
    assert_eq!(error.to_string(), "header.scheme: 3 is not a defined value");
    let error = Content::from_cbor(&hex(&format!("a2 01{ADDRESS} 0400"))).unwrap_err();
    assert_eq!(error.to_string(), "content: no body");
    let error = Content::from_cbor(
        &Content {
            delegation: Some(delegation()),
            ..content(alice(), json())
        }
        .to_cbor(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "content.from: differs from delegation-body.agent"
    );
    let error = GrantBody::from_cbor(
        &GrantBody {
            signer: server_device(),
            ..grant().body
        }
        .to_cbor(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "grant-body.signer: the same as grant-body.service"
    );
}
