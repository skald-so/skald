//! What counts towards the rate, as towards the monthly limit: a message counts once, whatever its
//! envelopes; a retry does not count and is answered as it was first, past the rate too; a message
//! refused with another problem does not count.

use crate::rate::{
    RATE, answer_to, assert_over_rate, message_to, stores, stores_messages, with_alice,
};
use crate::wire::{
    Answer, UNKNOWN_RECIPIENT, VERSION_MISMATCH, assert_nowhere, envelope, envelopes_of, version,
};

/// With the setting at 2: alice's message to bob and carol counts once, its retries, also with
/// other bytes, not at all; with her next message to bob, her laptop has stored 2. Her third, to
/// both, is `rate-limited` for each recipient, while a retry of her first still gets its first
/// `stored`.
#[test]
fn a_message_counts_once_whatever_its_envelopes_and_a_retry_not_at_all() {
    let (fixture, mut alice) = with_alice(3_216_060_201, &[RATE, "2"]);
    let [_, bob, carol, ..] = fixture.laptops();
    let db = fixture.db();

    let one = message_to(&fixture, &[bob, carol]);
    let one_sent = stores(&mut alice, &one);
    let first: Vec<Answer> = (one.iter().zip(&one_sent))
        .map(|(header, sent)| answer_to(&mut alice, header, sent))
        .collect();
    for answer in &first {
        assert!(matches!(answer, Answer::Stored(_)), "a retry: {answer:?}");
    }
    for (header, first) in one.iter().zip(&first) {
        let other = envelope(header);
        let retried = answer_to(&mut alice, header, &other);
        assert_eq!(&retried, first, "a retry with other bytes");
        assert_nowhere(db, &other);
    }
    let two_sent = stores(&mut alice, &message_to(&fixture, &[bob]));

    let three = message_to(&fixture, &[bob, carol]);
    assert_over_rate(db, &mut alice, &three, "the third, to bob and carol");
    for ((header, sent), first) in one.iter().zip(&one_sent).zip(&first) {
        let again = answer_to(&mut alice, header, sent);
        assert_eq!(&again, first, "a retry past the rate");
    }

    let to_bob = [one_sent[0].clone(), two_sent[0].clone()];
    assert_eq!(envelopes_of(db, bob), to_bob, "bob's mailbox");
    assert_eq!(envelopes_of(db, carol), [one_sent[1].clone()], "carol's");
}

/// With the setting at 2, after sends refused as `version-mismatch` and `unknown-recipient`, alice's
/// laptop stores 2 messages, and the third is `rate-limited`.
#[test]
fn refused_messages_do_not_count() {
    let (fixture, mut alice) = with_alice(3_216_060_211, &[RATE, "2"]);
    let [_, bob, _, _, eve] = fixture.laptops();
    let db = fixture.db();

    let mut other_version = fixture.to(bob);
    other_version.v = version(other_version.v.get() + 1);
    let to_eve = fixture.to(eve);
    for (header, problem) in [
        (&other_version, VERSION_MISMATCH),
        (&to_eve, UNKNOWN_RECIPIENT),
    ] {
        let sent = envelope(header);
        match answer_to(&mut alice, header, &sent) {
            Answer::Refused(_, refused) => assert_eq!(refused, problem, "{problem}"),
            stored => panic!("{problem}, not {stored:?}"),
        }
    }

    let stored = stores_messages(&fixture, &mut alice, 2, &[bob]);
    assert_over_rate(db, &mut alice, &message_to(&fixture, &[bob]), "the third");
    assert_eq!(envelopes_of(db, bob), stored, "bob's mailbox");
}
