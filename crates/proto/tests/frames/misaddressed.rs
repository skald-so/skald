//! A `send` whose `hpke` header does not name exactly one recipient does not decode, yet
//! `Frame::misaddressed_send` gives its header, so a server can refuse it by its `msg_id`; any
//! other frame gives none.

use skald_proto::{Cbor, DeviceId, Envelope, Frame, Header, HpkePayload, NonEmpty, Scheme};

use crate::common::{alice_device, bob_device, header, hex, msg};

/// The `send` frame of `envelope`'s bytes, with `report` as `report`, an encoded item.
fn send(envelope: &[u8], report: &[u8]) -> Vec<u8> {
    let len = u8::try_from(envelope.len()).expect("an envelope under 256 bytes");
    let mut frame = vec![0x83, 0x01, 0x58, len];
    frame.extend_from_slice(envelope);
    frame.extend_from_slice(report);
    frame
}

/// An `hpke` header of the first message with `recipients`.
fn hpke(recipients: Option<NonEmpty<DeviceId>>) -> Header {
    Header {
        recipients,
        ..header(msg(1), Scheme::Hpke)
    }
}

/// The envelope of `header` with an `hpke` payload.
fn envelope(header: &Header) -> Vec<u8> {
    Envelope {
        header: header.clone(),
        payload: HpkePayload {
            enc: [0xe0; 32],
            ct: hex("c0c1c2c3"),
        }
        .to_cbor(),
    }
    .to_cbor()
}

#[test]
fn an_hpke_send_to_no_or_two_recipients_gives_its_header() {
    let two = NonEmpty::from_vec(vec![alice_device(), bob_device()]);
    for header in [hpke(None), hpke(two)] {
        let frame = send(&envelope(&header), &[0xf5]);
        assert!(Frame::from_cbor(&frame).is_err(), "{:?}", header.recipients);
        assert_eq!(
            Frame::misaddressed_send(&frame),
            Some(header.clone()),
            "{:?}",
            header.recipients
        );
    }
}

#[test]
fn any_other_frame_gives_no_header() {
    let none = hpke(None);
    let one = envelope(&hpke(Some(NonEmpty::new(bob_device()))));
    let mls = envelope(&header(msg(1), Scheme::Mls));
    let not_a_payload = Envelope {
        header: none.clone(),
        payload: hex("00"),
    }
    .to_cbor();
    let misaddressed = send(&envelope(&none), &[0xf5]);
    let cases = [
        ("a send that decodes", send(&one, &[0xf5])),
        ("an mls send", send(&mls, &[0xf5])),
        (
            "a payload that is no hpke payload",
            send(&not_a_payload, &[0xf5]),
        ),
        ("a report that is no bool", send(&envelope(&none), &[0x01])),
        (
            "a truncated send",
            misaddressed[..misaddressed.len() - 1].to_vec(),
        ),
        (
            "bytes after the send",
            [misaddressed.as_slice(), &[0x00]].concat(),
        ),
        (
            "another frame type",
            [&[0x83, 0x05], &misaddressed[2..]].concat(),
        ),
    ];
    for (what, frame) in cases {
        assert_eq!(Frame::misaddressed_send(&frame), None, "{what}");
    }
}
