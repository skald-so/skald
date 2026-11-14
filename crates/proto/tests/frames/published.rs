//! `docs/protocol/test-vectors.md` publishes the hex of each frame type, as it does for `stored`,
//! and `proto` encodes each published frame to exactly its bytes.

use skald_proto::{Cbor, Frame, Push};

use crate::common::{hex, to_hex};

const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");

/// The bytes of each `hex` block of the document.
fn hex_blocks() -> Vec<Vec<u8>> {
    let mut blocks = Vec::new();
    let mut open: Option<String> = None;
    for line in DOC.lines() {
        match (&mut open, line) {
            (None, "```hex") => open = Some(String::new()),
            (Some(text), "```") => {
                blocks.push(hex(text));
                open = None;
            }
            (Some(text), _) => text.push_str(line),
            (None, _) => {}
        }
    }
    assert!(open.is_none(), "a `hex` block is not closed");
    blocks
}

/// The frames the document publishes, each with its bytes.
fn published_frames() -> Vec<(Frame, Vec<u8>)> {
    hex_blocks()
        .into_iter()
        .filter_map(|bytes| Frame::from_cbor(&bytes).ok().map(|frame| (frame, bytes)))
        .collect()
}

/// Whether a frame is of some type.
type IsType = fn(&Frame) -> bool;

#[test]
fn the_test_vectors_publish_every_frame_type() {
    let published = published_frames();
    let types: [(&str, IsType); 9] = [
        ("send", |frame| matches!(frame, Frame::Send { .. })),
        ("stored", |frame| matches!(frame, Frame::Stored(_))),
        ("send-refused", |frame| {
            matches!(frame, Frame::SendRefused { .. })
        }),
        ("sync", |frame| matches!(frame, Frame::Sync { .. })),
        ("message", |frame| {
            matches!(frame, Frame::Push(Push::Message { .. }))
        }),
        ("status", |frame| {
            matches!(frame, Frame::Push(Push::Status { .. }))
        }),
        ("ack", |frame| matches!(frame, Frame::Ack { .. })),
        ("batch", |frame| matches!(frame, Frame::Batch(_))),
        ("space-notice", |frame| {
            matches!(frame, Frame::Push(Push::SpaceNotice { .. }))
        }),
    ];
    let missing: Vec<&str> = types
        .iter()
        .filter(|(_, is)| !published.iter().any(|(frame, _)| is(frame)))
        .map(|(name, _)| *name)
        .collect();
    assert!(
        missing.is_empty(),
        "docs/protocol/test-vectors.md has no vector of {missing:?}"
    );
}

#[test]
fn proto_encodes_each_published_frame_to_its_bytes() {
    let published = published_frames();
    assert!(
        published
            .iter()
            .any(|(frame, _)| !matches!(frame, Frame::Stored(_))),
        "docs/protocol/test-vectors.md has no frame vector besides `stored`"
    );
    for (frame, bytes) in published {
        assert!(
            !matches!(frame, Frame::Unknown(_)),
            "a vector of an unknown frame type: {}",
            to_hex(&bytes)
        );
        assert_eq!(to_hex(&frame.to_cbor()), to_hex(&bytes), "{frame:?}");
    }
}
