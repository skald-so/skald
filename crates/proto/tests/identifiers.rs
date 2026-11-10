//! Identifiers: `docs/protocol/README.md`, "Identifiers".

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use skald_proto::{
    AccountId, ChannelId, ClaimId, DeviceId, Id, IdError, Issued, MessageId, QueueId, SpaceId,
    TaskId,
};

const TEXT: &str = "01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10";
const BYTES: [u8; 16] = [
    0x01, 0xa1, 0xc4, 0xc4, 0x94, 0xcb, 0x7a, 0x3e, 0x9c, 0x41, 0x5d, 0x2e, 0x8f, 0x6b, 0x7a, 0x10,
];

/// The public key of RFC 8032, §7.1, "TEST 1".
const RFC8032_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

fn version(bytes: &[u8; 16]) -> u8 {
    bytes[6] >> 4
}

fn variant(bytes: &[u8; 16]) -> u8 {
    bytes[8] >> 6
}

#[test]
fn text_form_round_trips() {
    let id: MessageId = TEXT.parse().unwrap();
    assert_eq!(id.as_bytes(), &BYTES);
    assert_eq!(id.to_string(), TEXT);
    assert_eq!(MessageId::from_bytes(BYTES).to_string(), TEXT);
}

#[test]
fn every_hex_digit_round_trips() {
    for text in [
        "00000000-0000-0000-0000-000000000000",
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
        "01234567-89ab-cdef-0123-456789abcdef",
        "fedcba98-7654-3210-fedc-ba9876543210",
    ] {
        let id: SpaceId = text.parse().unwrap();
        assert_eq!(id.to_string(), text);
    }
    let id: SpaceId = "01234567-89ab-cdef-0123-456789abcdef".parse().unwrap();
    assert_eq!(
        id.as_bytes(),
        &[
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef
        ]
    );
}

#[test]
fn wire_form_is_sixteen_bytes() {
    let id = ChannelId::try_from(&BYTES[..]).unwrap();
    assert_eq!(id, ChannelId::from_bytes(BYTES));
    assert_eq!(
        ChannelId::try_from(&BYTES[..15]),
        Err(IdError::ByteLength(15))
    );
    assert_eq!(
        ChannelId::try_from(&[0; 17][..]),
        Err(IdError::ByteLength(17))
    );
    assert_eq!(ChannelId::try_from(&[][..]), Err(IdError::ByteLength(0)));
}

#[test]
fn ids_are_opaque() {
    // Any version and variant parses: receivers check only a device ID, against its key.
    for text in [
        "00000000-0000-0000-0000-000000000000",
        "f47ac10b-58cc-4372-a567-0e02b2c3d479",
        "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
    ] {
        assert_eq!(text.parse::<AccountId>().unwrap().to_string(), text);
        assert_eq!(text.parse::<DeviceId>().unwrap().to_string(), text);
    }
}

#[test]
fn other_text_forms_are_rejected() {
    let cases = [
        ("", IdError::TextLength(0)),
        ("01a1c4c494cb7a3e9c415d2e8f6b7a10", IdError::TextLength(32)),
        (
            "{01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10}",
            IdError::TextLength(38),
        ),
        (
            "urn:uuid:01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10",
            IdError::TextLength(45),
        ),
        (
            "01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a1",
            IdError::TextLength(35),
        ),
        (
            "01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a100",
            IdError::TextLength(37),
        ),
        (
            "01A1C4C4-94CB-7A3E-9C41-5D2E8F6B7A10",
            IdError::Uppercase(2),
        ),
        (
            "01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a1F",
            IdError::Uppercase(35),
        ),
        (
            "01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a1g",
            IdError::InvalidChar(35),
        ),
        (
            "01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a1-",
            IdError::InvalidChar(35),
        ),
        (
            " 1a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10",
            IdError::InvalidChar(0),
        ),
        (
            "01a1c4c4094cb-7a3e-9c41-5d2e8f6b7a10",
            IdError::MissingHyphen(8),
        ),
        (
            "01a1c4c4-94cb07a3e-9c41-5d2e8f6b7a10",
            IdError::MissingHyphen(13),
        ),
        (
            "01a1c4c4-94cb-7a3e09c41-5d2e8f6b7a10",
            IdError::MissingHyphen(18),
        ),
        (
            "01a1c4c4-94cb-7a3e-9c4105d2e8f6b7a10",
            IdError::MissingHyphen(23),
        ),
        (
            "01a1c4c-494cb-7a3e-9c41-5d2e8f6b7a10",
            IdError::InvalidChar(7),
        ),
        // 36 bytes, not 36 characters: a multibyte character takes the place of two digits.
        (
            "01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7aé",
            IdError::InvalidChar(34),
        ),
    ];
    for (text, error) in cases {
        assert_eq!(text.parse::<TaskId>(), Err(error), "{text:?}");
    }
}

#[test]
fn errors_name_the_problem() {
    let message = |text: &str| text.parse::<QueueId>().unwrap_err().to_string();
    assert_eq!(
        message("01a1c4c494cb7a3e9c415d2e8f6b7a10"),
        "an ID is 36 characters, such as 01a1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10, not 32"
    );
    assert_eq!(
        message("01a1c4c4094cb-7a3e-9c41-5d2e8f6b7a10"),
        "an ID needs '-' at position 8"
    );
    assert_eq!(
        message("01A1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10"),
        "an ID has an uppercase letter at position 2; IDs are lowercase"
    );
    assert_eq!(
        message("01x1c4c4-94cb-7a3e-9c41-5d2e8f6b7a10"),
        "an ID has a character other than 0-9 and a-f at position 2"
    );
    assert_eq!(
        QueueId::try_from(&[0; 3][..]).unwrap_err().to_string(),
        "an ID is 16 bytes, not 3"
    );
}

#[test]
fn debug_names_the_kind() {
    let text = format!("{:?}", DeviceId::from_bytes(BYTES));
    assert_eq!(text, format!("DeviceId({TEXT})"));
    assert_eq!(
        format!("{:?}", ClaimId::from_bytes(BYTES)),
        format!("ClaimId({TEXT})")
    );
}

#[test]
fn ids_compare_by_bytes() {
    let low = MessageId::from_bytes([0; 16]);
    let high = MessageId::from_bytes(BYTES);
    assert!(low < high);
    assert_eq!(low.cmp(&low), std::cmp::Ordering::Equal);
    assert_eq!(HashSet::from([low, high, low]).len(), 2);
}

/// Checks the UUIDv7 IDs `generate` makes for one kind.
fn check_generated<K: Issued>() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let ids: Vec<Id<K>> = (0..64).map(|_| Id::generate()).collect();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    for id in &ids {
        let bytes = id.as_bytes();
        assert_eq!(version(bytes), 7, "{id}");
        assert_eq!(variant(bytes), 0b10, "{id}");
        let mut time = [0; 8];
        time[2..].copy_from_slice(&bytes[..6]);
        let time = u128::from(u64::from_be_bytes(time));
        assert!(
            before <= time && time <= after,
            "{id}: {time} not in {before}..={after}"
        );
    }
    // 74 random bits: the IDs differ, and so does each random byte across them.
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
    for at in 6..16 {
        let values: HashSet<u8> = ids.iter().map(|id| id.as_bytes()[at]).collect();
        assert!(values.len() > 1, "byte {at} is the same in every ID");
    }
}

#[test]
fn issued_ids_are_uuidv7_with_random_bits() {
    check_generated::<skald_proto::kind::Account>();
    check_generated::<skald_proto::kind::Space>();
    check_generated::<skald_proto::kind::Channel>();
    check_generated::<skald_proto::kind::Message>();
    check_generated::<skald_proto::kind::Queue>();
    check_generated::<skald_proto::kind::Task>();
    check_generated::<skald_proto::kind::Claim>();
}

#[test]
fn device_id_is_uuidv8_of_its_signing_key() {
    // SHA-256 over 82 6c "skald device" 58 20 <key>, its first 16 bytes with version 8 and variant 0b10.
    let cases = [
        (RFC8032_KEY, "19ac5f78-4938-8235-86a6-c5809a18f6dd"),
        ([0; 32], "be6507b5-0f90-896c-a46a-e5b5d23a819b"),
    ];
    for (key, text) in cases {
        let id = DeviceId::from_signing_key(&key);
        assert_eq!(id.to_string(), text);
        assert_eq!(version(id.as_bytes()), 8);
        assert_eq!(variant(id.as_bytes()), 0b10);
    }
}

#[test]
fn device_id_is_checked_against_its_signing_key() {
    let id = DeviceId::from_signing_key(&RFC8032_KEY);
    assert!(id.is_derived_from(&RFC8032_KEY));
    let mut other = RFC8032_KEY;
    other[31] ^= 1;
    assert!(!id.is_derived_from(&other));
    let parsed: DeviceId = "19ac5f78-4938-8235-86a6-c5809a18f6dd".parse().unwrap();
    assert!(parsed.is_derived_from(&RFC8032_KEY));
    assert!(!DeviceId::from_bytes(BYTES).is_derived_from(&RFC8032_KEY));
}
