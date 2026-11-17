//! Entries a compromised server adds: one handed twice, or another with the same sender and
//! `msg_id`, shows the message once; one under a key the device does not hold is counted, not shown.

use skald_crypto::HistoryKey;
use skald_proto::{Cbor as _, HistoryBody, MessageId};

use crate::chain::key_of;
use crate::reading::{Duo, Said, add_entries, assert_history, body_of, without_key};

/// The text of the entry the proxy adds with the same sender and `msg_id` as Bob's first line.
const ALSO: &str = "the same message told otherwise";

/// The text of the entry the proxy adds under a key no device holds.
const UNKEYED: &str = "a line under a key nobody holds";

/// The epoch of that entry, which the channel has not reached.
const UNKEYED_EPOCH: u64 = 9;

/// Bob says two lines. The proxy hands the first line's entry twice, and adds an entry of Bob's
/// device of the same `msg_id` and time with another text, under his key of epoch 1. Ada's `skald
/// history` shows each message once: the first line or the other text, once in all, then the second
/// line.
#[test]
fn entry_handed_twice_and_another_of_the_same_sender_and_msg_id_show_the_message_once() {
    let mut duo = Duo::new(
        &[4_166_010_001, 4_166_010_002],
        &["ada-sees-repeats", "bob-gets-repeated"],
    );
    let (first, msg_id, entry) = duo.bob_says(&duo.bob_s(1, "said once only"));
    let (second, ..) = duo.bob_says(&duo.bob_s(2, "said after it"));
    let bob = &duo.talk.played[0];
    let body = HistoryBody {
        timestamp: first.sent.0,
        ..body_of(bob.device(), duo.at.1, (msg_id, &duo.bob_s(1, ALSO)))
    };
    let also = key_of(bob, duo.at.1, 1)
        .seal_entry(1, &body, &bob.person.key)
        .expect("an entry");
    add_entries(&duo.talk.world.cloud, duo.at, &[entry, also.to_cbor()]);

    let shown = duo.ada_s_history();

    let all = shown.shown();
    assert_eq!(
        all.matches(&first.text).count() + all.matches(ALSO).count(),
        1,
        "the message not shown once: {shown:?}"
    );
    if all.contains(&first.text) {
        assert_history(&shown, duo.at.1, &[first, second], &[]);
    } else {
        let told = Said {
            text: ALSO.to_owned(),
            ..first
        };
        assert_history(&shown, duo.at.1, &[told, second], &[]);
    }
}

/// Bob says a line. The proxy adds an entry of Bob's device of epoch 9, which the channel has not
/// reached, under a key no device holds. Ada's `skald history` shows Bob's line and not the added
/// one, and says it holds no key for one entry.
#[test]
fn entry_under_a_key_the_device_does_not_hold_is_counted_not_shown() {
    let mut duo = Duo::new(
        &[4_166_011_001, 4_166_011_002],
        &["ada-counts-unkeyed", "bob-says-keyed"],
    );
    let (said, ..) = duo.bob_says(&duo.bob_s(1, "a line ada can read"));
    let bob = &duo.talk.played[0];
    let line = duo.bob_s(2, UNKEYED);
    let body = body_of(bob.device(), duo.at.1, (MessageId::generate(), &line));
    let unkeyed = HistoryKey::from_bytes(&[0x42; 16])
        .seal_entry(UNKEYED_EPOCH, &body, &bob.person.key)
        .expect("an entry");
    add_entries(&duo.talk.world.cloud, duo.at, &[unkeyed.to_cbor()]);

    let shown = duo.ada_s_history();

    assert_history(&shown, duo.at.1, &[said], &[UNKEYED]);
    assert_eq!(
        without_key(&shown),
        Some(1),
        "not one entry counted as under no key: {shown:?}"
    );
}
