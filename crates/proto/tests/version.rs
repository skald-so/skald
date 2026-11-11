//! Protocol versions: `docs/protocol/README.md`, "Protocol version".

use std::num::NonZeroU64;

use skald_proto::{
    LATEST_HEADER, PROTOCOL_VERSION, SUBPROTOCOL_PREFIX, VERSION_HEADER, VersionProblem,
    parse_version, subprotocol,
};

fn version(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

#[test]
fn current_version_is_1() {
    assert_eq!(PROTOCOL_VERSION, version(1));
}

#[test]
fn names_follow_spec() {
    assert_eq!(VERSION_HEADER, "Skald-Protocol-Version");
    assert_eq!(LATEST_HEADER, "Skald-Protocol-Latest");
    assert_eq!(SUBPROTOCOL_PREFIX, "skald.");
}

#[test]
fn parses_positive_integers() {
    for (text, n) in [
        ("1", 1),
        ("2", 2),
        ("9", 9),
        ("10", 10),
        ("307", 307),
        ("18446744073709551615", u64::MAX),
    ] {
        assert_eq!(parse_version(text), Some(version(n)), "{text:?}");
    }
}

#[test]
fn version_above_u64_is_maximum() {
    for text in ["18446744073709551616", "99999999999999999999999999"] {
        assert_eq!(parse_version(text), Some(NonZeroU64::MAX), "{text:?}");
    }
}

#[test]
fn rejects_other_text() {
    for text in [
        "", "0", "00", "01", "007", "-1", "+1", " 1", "1 ", "1.0", "1e3", "0x1", "one", "1,2", "١",
    ] {
        assert_eq!(parse_version(text), None, "{text:?}");
    }
}

#[test]
fn subprotocol_states_version() {
    assert_eq!(subprotocol(version(1)), "skald.1");
    assert_eq!(subprotocol(version(23)), "skald.23");
}

#[test]
fn problems_have_spec_types() {
    for (problem, name) in [
        (VersionProblem::TooOld, "protocol-too-old"),
        (VersionProblem::TooNew, "protocol-too-new"),
        (VersionProblem::Invalid, "protocol-version-invalid"),
    ] {
        assert_eq!(problem.name(), name);
        assert_eq!(
            problem.type_uri(),
            format!("https://skald.so/problems/{name}")
        );
    }
}
