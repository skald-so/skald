//! The test vector of the client's first frame on a WebSocket, `auth`, which proves its device:
//! `docs/protocol/test-vectors.md` publishes its hex, and `proto` encodes and decodes it.
//!
//! The frame, as this test reads it: `auth = [10, challenge: bstr, device: id, signature: bstr
//! .size 64]`, in the section "Frame `auth`", for Alice's device, with a 32-byte challenge.

mod common;

use common::{alice_device, hex, vectors};

const DOC: &str = include_str!("../../../docs/protocol/test-vectors.md");
/// The vector's section in the document and its name among the vectors.
const NAME: &str = "Frame `auth`";

/// The bytes in the `hex` block of the document's section `name`.
fn published_hex(name: &str) -> Vec<u8> {
    let heading = format!("## {name}");
    let mut lines = DOC
        .lines()
        .skip_while(|line| *line != heading)
        .skip(1)
        .take_while(|line| !line.starts_with("## "));
    let mut text = String::new();
    lines
        .by_ref()
        .find(|line| *line == "```hex")
        .unwrap_or_else(|| panic!("no hex block under {heading:?}"));
    for line in lines.take_while(|line| *line != "```") {
        text.push_str(line);
    }
    hex(&text)
}

#[test]
fn the_auth_frame_vector_is_the_frame_of_the_spec() {
    let bytes = published_hex(NAME);

    let (head, rest) = bytes.split_at(2);
    assert_eq!(head, [0x84, 0x0a], "a 4-item array of type 10");
    let (challenge, rest) = rest.split_at(2 + 32);
    assert_eq!(challenge[..2], [0x58, 0x20], "a 32-byte challenge");
    let (device, rest) = rest.split_at(1 + 16);
    assert_eq!(device[0], 0x50, "a 16-byte device ID");
    assert_eq!(&device[1..], alice_device().as_bytes(), "Alice's device");
    assert_eq!(rest.len(), 2 + 64, "a 64-byte signature, then nothing");
    assert_eq!(rest[..2], [0x58, 0x40], "a 64-byte signature");
}

#[test]
fn proto_encodes_and_decodes_the_auth_frame_vector() {
    let vectors = vectors();
    let vector = vectors
        .iter()
        .find(|vector| vector.name == NAME)
        .unwrap_or_else(|| panic!("no vector {NAME:?} among the vectors proto encodes"));

    assert_eq!(vector.bytes, published_hex(NAME));
    assert_eq!((vector.decode)(&vector.bytes), Ok(vector.bytes.clone()));
}
