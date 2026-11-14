//! Frames of a type `proto` does not know: a reader ignores them, alone or in a `batch`
//! (`docs/protocol/delivery.md`, "Frames").

mod common;

use common::{assert_field_problem, hex, space, to_hex};
use skald_proto::{Cbor, FieldProblem, Frame, Push};

const SPACE: &str = "019a6f1e2b3c7d4e8f5061a2b3c4d5e6";

#[test]
fn an_unknown_frame_encodes_as_its_type_alone() {
    for (frame_type, text) in [(100, "81 1864"), (u64::MAX, "81 1bffffffffffffffff")] {
        let frame = Frame::Unknown(frame_type);
        assert_eq!(to_hex(&frame.to_cbor()), to_hex(&hex(text)), "{text}");
        assert_eq!(Frame::from_cbor(&hex(text)), Ok(frame), "{text}");
    }
}

#[test]
fn a_batch_keeps_its_entries_of_unknown_types_as_they_came_in() {
    let unknown = ["83 1864 6178 a1 01 02", "82 1903e8 80"];
    let text = format!("82 08 83 82 09 50{SPACE} {} {}", unknown[0], unknown[1]);
    let bytes = hex(&text);
    let frame = Frame::from_cbor(&bytes).expect("a batch with entries of unknown types");
    assert_eq!(to_hex(&frame.to_cbor()), to_hex(&bytes));
    let Frame::Batch(entries) = frame else {
        panic!("not a batch: {frame:?}");
    };
    let [known, first, second] = &entries[..] else {
        panic!("not three entries: {entries:?}");
    };
    assert_eq!(known, &Push::SpaceNotice { space: space() });
    for (entry, frame_type, text) in [(first, 100, unknown[0]), (second, 1000, unknown[1])] {
        let Push::Unknown(entry) = entry else {
            panic!("not unknown: {entry:?}");
        };
        assert_eq!(entry.frame_type(), frame_type);
        assert_eq!(to_hex(entry.bytes()), to_hex(&hex(text)));
    }
}

#[test]
fn a_batch_holds_no_frame_of_another_known_type() {
    for frame_type in [1, 2, 3, 4, 7, 8, 10] {
        assert_field_problem(
            Frame::from_cbor(&hex(&format!("82 08 81 81 {frame_type:02x}"))),
            &FieldProblem::Value(frame_type),
        );
    }
}
