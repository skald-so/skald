//! Envelopes, content and receipts: `docs/protocol/README.md`, "Envelope" and "Content", and
//! `docs/protocol/delivery.md`, "Receipts". Each type round-trips, keeps the fields it does not
//! know, and rejects what its CDDL does not allow.

mod common;

use std::fmt::Debug;

use common::*;
use skald_proto::{
    AddressError, Attachment, Body, Cbor, Content, DecodeError, Delegation, DelegationBody,
    DeliveryStatus, Envelope, FieldProblem, Grant, GrantBody, Header, HpkePayload, HpkePlaintext,
    NameError, NonEmpty, Receipt, ReceiptStatus, Scheme, Segment, ServerStatus, Unknown,
};

/// The encoding of a 16-byte ID: `h'019a6f1e2b3c7d4e8f5061a2b3c4d5e6'`.
const ID: &str = "50019a6f1e2b3c7d4e8f5061a2b3c4d5e6";
/// `"a/b/c@d"`.
const ADDRESS: &str = "67612f622f6340 64";
/// `"{}"`, an `a2a` body.
const JSON: &str = "627b7d";

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
    // {1: "a/b/c@d", 4: 0, 11: {1: "{}"}}: the `task` body, unknown to this version.
    let bytes = hex(&format!("a3 01{ADDRESS} 0400 0b a101{JSON}"));
    let content = Content::from_cbor(&bytes).unwrap();
    assert_eq!(content.body, None);
    assert_eq!(content.unknown.keys().collect::<Vec<_>>(), [11]);
    assert_eq!(content.to_cbor(), bytes);
}

#[test]
fn content_needs_exactly_one_body() {
    assert_eq!(
        decode::<Content>(&format!("a2 01{ADDRESS} 0400")),
        field("content", FieldProblem::NoBody)
    );
    assert_eq!(
        decode::<Content>(&format!("a4 01{ADDRESS} 0400 06{JSON} 09a2 0103 0281{ID}")),
        field("content", FieldProblem::TwoBodies)
    );
    assert_eq!(
        decode::<Content>(&format!("a3 01{ADDRESS} 0400 06{JSON}")),
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
fn embedded_items_are_checked_like_any_other() {
    let content = format!("a3 01{ADDRESS} 0400 06{JSON}");
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
            embed(&format!("a2 01{ADDRESS} 0400"))
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
    let grant_body = format!("a4 01{ID} 0281{ID} 03{ID} 0400");
    assert_eq!(
        decode::<Grant>(&format!("82 {} {signature}", embed(&grant_body))),
        Ok(())
    );
    assert_eq!(
        decode::<Grant>(&format!(
            "82 {} {signature}",
            embed(&format!("a4 01{ID} 0280 03{ID} 0400"))
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
            embed(&format!("a3 01{ADDRESS} 02{ID} 03{ID}"))
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
    let body = hex(&format!("a5 01{ID} 0281{ID} 03{ID} 0400 1864f5"));
    let grant = GrantBody::from_cbor(&body).unwrap();
    assert_eq!(grant.to_cbor(), body);
    let delegation = hex(&format!("a5 01{ADDRESS} 02{ID} 03{ID} 0400 20f4"));
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
        let header =
            Header::from_cbor(&hex(&format!("a5 0101 02{ID} 03{ID} 04{ID} 05{value:02x}")))
                .unwrap();
        assert_eq!(header.scheme, scheme);
    }
}

#[test]
fn errors_name_the_field_and_the_problem() {
    let error = Header::from_cbor(&hex(&format!("a5 0101 02{ID} 03{ID} 04{ID} 0503"))).unwrap_err();
    assert_eq!(error.to_string(), "header.scheme: 3 is not a defined value");
    let error = Content::from_cbor(&hex(&format!("a2 01{ADDRESS} 0400"))).unwrap_err();
    assert_eq!(error.to_string(), "content: no body");
}
