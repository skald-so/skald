//! Protocol versions: `docs/protocol/README.md`, "Protocol version".

use std::num::NonZeroU64;

use skald_proto::{
    LATEST_HEADER, PROTOCOL_VERSION, SUBPROTOCOL_PREFIX, VERSION_HEADER, VersionProblem,
    VersionRange, describe_versions, parse_version, subprotocol,
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

fn range(text: &str) -> VersionRange {
    text.parse().unwrap()
}

#[test]
fn range_parses_one_version_or_two_bounds() {
    for (text, oldest, latest, list) in [
        ("3", 3, 3, &[3][..]),
        ("2-3", 2, 3, &[2, 3][..]),
        ("4-4", 4, 4, &[4][..]),
        ("1-4", 1, 4, &[1, 2, 3, 4][..]),
    ] {
        let parsed = range(text);
        assert_eq!(
            (parsed.oldest(), parsed.latest(), parsed.list()),
            (version(oldest), version(latest), list.to_vec()),
            "{text:?}"
        );
    }
}

#[test]
fn range_refuses_other_text() {
    for (text, error) in [
        ("3-2", "the oldest version, 3, is above the latest, 2"),
        ("0", r#""0" is not a protocol version, an integer from 1"#),
        ("02", r#""02" is not a protocol version, an integer from 1"#),
        ("2-", r#""" is not a protocol version, an integer from 1"#),
        ("-2", r#""" is not a protocol version, an integer from 1"#),
        (
            "2-3-4",
            r#""3-4" is not a protocol version, an integer from 1"#,
        ),
        (
            "two",
            r#""two" is not a protocol version, an integer from 1"#,
        ),
        ("", r#""" is not a protocol version, an integer from 1"#),
    ] {
        assert_eq!(
            text.parse::<VersionRange>(),
            Err(error.to_owned()),
            "{text:?}"
        );
    }
}

#[test]
fn default_range_is_current_version_and_previous_one() {
    // Version 1 is the only one so far: the range is version 1 alone.
    assert_eq!(VersionRange::default(), range("1"));
}

#[test]
fn range_contains_its_bounds_and_between() {
    let parsed = range("2-4");
    for (n, contained) in [(1, false), (2, true), (3, true), (4, true), (5, false)] {
        assert_eq!(parsed.contains(n), contained, "{n}");
    }
}

#[test]
fn versions_are_described_in_words() {
    for (versions, text) in [
        (&[][..], "no protocol version"),
        (&[3][..], "protocol version 3"),
        (&[2, 3][..], "protocol versions 2 and 3"),
        (&[1, 2, 3][..], "protocol versions 1, 2 and 3"),
        (&[5, 9, 12, 20][..], "protocol versions 5, 9, 12 and 20"),
    ] {
        assert_eq!(describe_versions(versions), text, "{versions:?}");
    }
    assert_eq!(range("3").to_string(), "protocol version 3");
    assert_eq!(range("1-3").to_string(), "protocol versions 1, 2 and 3");
}
