//! The link messages of `skald-proto`: `link-hello`, `link-reply`, `link-request`, `link-grant` and
//! `link-abort` of "Link code", each a CBOR array of its type and fields. They decode into their
//! fields and encode back byte for byte; malformed and truncated input is refused.

use skald_proto::{Cbor, LinkMessage};

use crate::fixtures::SESSION;

const SHARE: [u8; 32] = [0x2b; 32];
const SEALED: &[u8] = &[0xc0; 40];

/// The head of a byte string of `len` bytes, below 256.
fn bytes_head(len: usize) -> Vec<u8> {
    let len = u8::try_from(len).unwrap();
    if len < 24 {
        vec![0x40 | len]
    } else {
        vec![0x58, len]
    }
}

/// The encoded byte string `bytes`.
fn bstr(bytes: &[u8]) -> Vec<u8> {
    [bytes_head(bytes.len()), bytes.to_vec()].concat()
}

/// The encoded array of `items`, each already encoded.
fn array(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0x80 | u8::try_from(items.len()).unwrap()];
    out.extend(items.concat());
    out
}

/// Each message of the tests, with its bytes made apart from the crate.
fn messages() -> Vec<(LinkMessage, Vec<u8>)> {
    let session = || bstr(&SESSION);
    vec![
        (
            LinkMessage::Hello {
                session: SESSION,
                share: SHARE,
            },
            array(&[vec![1], session(), bstr(&SHARE)]),
        ),
        (
            LinkMessage::Reply {
                session: SESSION,
                share: SHARE,
            },
            array(&[vec![2], session(), bstr(&SHARE)]),
        ),
        (
            LinkMessage::Request {
                session: SESSION,
                sealed: SEALED.to_vec(),
            },
            array(&[vec![3], session(), bstr(SEALED)]),
        ),
        (
            LinkMessage::Grant {
                session: SESSION,
                sealed: SEALED.to_vec(),
            },
            array(&[vec![4], session(), bstr(SEALED)]),
        ),
        (
            LinkMessage::Abort { session: SESSION },
            array(&[vec![5], session()]),
        ),
    ]
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn link_messages_decode_into_their_fields_and_encode_back() {
    for (message, encoded) in messages() {
        assert_eq!(message.to_cbor(), encoded, "{message:?}");
        assert_eq!(LinkMessage::from_cbor(&encoded), Ok(message.clone()));
    }
    let long = vec![0xc1; 300];
    let grant = LinkMessage::Grant {
        session: SESSION,
        sealed: long.clone(),
    };
    let encoded = array(&[
        vec![4],
        bstr(&SESSION),
        [vec![0x59, 0x01, 0x2c], long].concat(),
    ]);
    assert_eq!(grant.to_cbor(), encoded);
    assert_eq!(LinkMessage::from_cbor(&encoded), Ok(grant));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_link_messages_are_refused() {
    let session = bstr(&SESSION);
    let share = bstr(&SHARE);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "an unknown type",
            array(&[vec![6], session.clone(), share.clone()]),
        ),
        ("type 0", array(&[vec![0], session.clone(), share.clone()])),
        (
            "a session of 15 bytes",
            array(&[vec![1], bstr(&[0x5e; 15]), share.clone()]),
        ),
        (
            "a session of 17 bytes",
            array(&[vec![1], bstr(&[0x5e; 17]), share.clone()]),
        ),
        (
            "a share of 31 bytes",
            array(&[vec![1], session.clone(), bstr(&[0x2b; 31])]),
        ),
        (
            "a share of 33 bytes",
            array(&[vec![2], session.clone(), bstr(&[0x2b; 33])]),
        ),
        (
            "a hello without its share",
            array(&[vec![1], session.clone()]),
        ),
        (
            "a reply with an extra item",
            array(&[vec![2], session.clone(), share.clone(), vec![0]]),
        ),
        (
            "an abort with a share",
            array(&[vec![5], session.clone(), share.clone()]),
        ),
        (
            "a request without its sealed",
            array(&[vec![3], session.clone()]),
        ),
        (
            "a grant whose sealed is text",
            array(&[
                vec![4],
                session.clone(),
                [vec![0x63], b"abc".to_vec()].concat(),
            ]),
        ),
        (
            "a session as text",
            array(&[vec![5], [vec![0x70], vec![b'a'; 16]].concat()]),
        ),
        (
            "the type as text",
            array(&[vec![0x61, b'1'], session.clone(), share.clone()]),
        ),
        ("a map", [vec![0xa1, 0x01], session.clone()].concat()),
        ("an empty array", array(&[])),
        ("a byte string", session.clone()),
        (
            "an indefinite-length array",
            [vec![0x9f, 0x05], session.clone(), vec![0xff]].concat(),
        ),
        (
            "the type in a longer head",
            array(&[vec![0x18, 0x05], session.clone()]),
        ),
        (
            "trailing bytes",
            [array(&[vec![5], session.clone()]), vec![0]].concat(),
        ),
        ("nothing", vec![]),
    ];
    for (what, bytes) in cases {
        assert!(LinkMessage::from_cbor(&bytes).is_err(), "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn truncated_link_messages_are_refused() {
    for (message, encoded) in messages() {
        for len in 0..encoded.len() {
            assert!(
                LinkMessage::from_cbor(&encoded[..len]).is_err(),
                "{message:?} cut to {len} bytes"
            );
        }
    }
}
