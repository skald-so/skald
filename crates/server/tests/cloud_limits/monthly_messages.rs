//! The messages a space stores per calendar month: with `--monthly-messages-limit`, a message past
//! its space's number for the month is refused with `quota-exceeded` and stored nowhere. A message
//! counts once, whatever its envelopes; a retry does not count, and a refused message does not
//! either. Each space has its count, and a month's count does not hold in the next one. Without the
//! setting, a space stores any number of messages.

use skald_proto::{ChannelId, DeviceId, Header, MessageId, Role, SpaceId};

use crate::common::devices::assert_taken;
use crate::fixture::Fixture;
use crate::limits::{
    MONTHLY, MONTHLY_ENV, QUOTA_EXCEEDED, SIZE, TOO_LARGE, answer_to, assert_refused_with,
    assert_stored, clear_of_the_month_end, restarted, sized, to_the_month_before, with_alice,
};
use crate::rosters::{channel, first, member, next};
use crate::wire::{
    Answer, NOT_A_MEMBER, Socket, UNKNOWN_RECIPIENT, VERSION_MISMATCH, envelope, envelopes_of,
    header, version,
};

/// As [`with_alice`], once the month has long enough left for the test's messages to fall in it.
fn counting(github: u64, args: &[&str]) -> (Fixture, Socket) {
    let (fixture, alice) = with_alice(github, args);
    clear_of_the_month_end(fixture.db());
    (fixture, alice)
}

/// The headers of one message, one `msg_id`, to each of `recipients` in the fixture's channel.
fn message_to(fixture: &Fixture, recipients: &[DeviceId]) -> Vec<Header> {
    let msg_id = MessageId::generate();
    recipients
        .iter()
        .map(|&recipient| Header {
            msg_id,
            ..fixture.to(recipient)
        })
        .collect()
}

/// Stores the message of `headers` on `socket`: each envelope must be stored; gives the envelopes.
#[track_caller]
fn store_message(socket: &mut Socket, headers: &[Header]) -> Vec<Vec<u8>> {
    headers
        .iter()
        .map(|header| {
            let sent = envelope(header);
            assert_stored(socket, header, &sent);
            sent
        })
        .collect()
}

/// Stores `count` messages on `socket`, each to `recipients`; gives their envelopes in order.
#[track_caller]
fn store_messages(
    fixture: &Fixture,
    socket: &mut Socket,
    count: usize,
    recipients: &[DeviceId],
) -> Vec<Vec<u8>> {
    (0..count)
        .flat_map(|_| store_message(socket, &message_to(fixture, recipients)))
        .collect()
}

/// Checks that the server refuses each envelope of the message of `headers` on `socket` with
/// `quota-exceeded` and stores nothing of it.
#[track_caller]
fn assert_over_quota(fixture: &Fixture, socket: &mut Socket, headers: &[Header], what: &str) {
    for header in headers {
        let sent = envelope(header);
        assert_refused_with(fixture.db(), socket, header, &sent, QUOTA_EXCEEDED, what);
    }
}

/// With the setting at 3: alice's message to bob and carol counts once, its retries, also with
/// other bytes, not at all; with bob's message and alice's next one to both devices, the space has
/// stored 3. Its next messages, from either device and to each recipient, are `quota-exceeded`,
/// while a retry of a stored message still gets its first `stored`.
#[test]
fn a_message_counts_once_whatever_its_envelopes_and_a_retry_not_at_all() {
    let (fixture, mut alice) = counting(3_121_310_101, &[MONTHLY, "3"]);
    let [alice_laptop, bob_laptop, carol, ..] = fixture.laptops();
    let mut bob = Socket::of(&fixture.team.flow, &fixture.team.bob);

    let one = message_to(&fixture, &[bob_laptop, carol]);
    let one_sent = store_message(&mut alice, &one);
    let first_stored = answer_to(&mut alice, &one[0], &one_sent[0]);
    for (header, sent) in one.iter().zip(&one_sent) {
        let again = answer_to(&mut alice, header, sent);
        assert!(matches!(again, Answer::Stored(_)), "a retry: {again:?}");
        let other = answer_to(&mut alice, header, &envelope(header));
        assert!(matches!(other, Answer::Stored(_)), "other bytes: {other:?}");
    }
    let two_sent = store_messages(&fixture, &mut bob, 1, &[alice_laptop]);
    let three_sent = store_messages(&fixture, &mut alice, 1, &[bob_laptop, carol]);

    let four = message_to(&fixture, &[bob_laptop, carol]);
    assert_over_quota(&fixture, &mut alice, &four, "alice's fourth message");
    let five = message_to(&fixture, &[carol]);
    assert_over_quota(&fixture, &mut bob, &five, "bob's next message");
    let again = answer_to(&mut alice, &one[0], &one_sent[0]);
    assert_eq!(again, first_stored, "a retry past the limit");

    let db = fixture.db();
    let to_bob = [one_sent[0].clone(), three_sent[0].clone()];
    assert_eq!(envelopes_of(db, bob_laptop), to_bob);
    let to_carol = [one_sent[1].clone(), three_sent[1].clone()];
    assert_eq!(envelopes_of(db, carol), to_carol);
    assert_eq!(envelopes_of(db, alice_laptop), two_sent);
}

/// Refused messages do not count: with the setting at 2 and the largest payload at 1000 bytes,
/// after sends refused as `version-mismatch`, `not-a-member`, `unknown-recipient` and `too-large`,
/// the space stores 2 messages, and the third is `quota-exceeded`.
#[test]
fn refused_messages_do_not_count() {
    let (fixture, mut alice) = counting(3_121_310_111, &[MONTHLY, "2", SIZE, "1000"]);
    let [alice_laptop, bob_laptop, _, _, eve] = fixture.laptops();
    let db = fixture.db();

    let mut other_version = fixture.to(bob_laptop);
    other_version.v = version(other_version.v.get() + 1);
    let to_eve = fixture.to(eve);
    let too_large = fixture.to(bob_laptop);
    let refusals = [
        (&other_version, envelope(&other_version), VERSION_MISMATCH),
        (&to_eve, envelope(&to_eve), UNKNOWN_RECIPIENT),
        (&too_large, sized(&too_large, 1_001), TOO_LARGE),
    ];
    for (header, sent, problem) in &refusals {
        assert_refused_with(db, &mut alice, header, sent, problem, problem);
    }
    // dave is in the space but not in the channel.
    let mut dave = Socket::of(&fixture.team.flow, &fixture.team.dave);
    let from_dave = fixture.to(alice_laptop);
    let by_dave = envelope(&from_dave);
    assert_refused_with(db, &mut dave, &from_dave, &by_dave, NOT_A_MEMBER, "dave");

    let stored = store_messages(&fixture, &mut alice, 2, &[bob_laptop]);
    let third = message_to(&fixture, &[bob_laptop]);
    assert_over_quota(&fixture, &mut alice, &third, "the third message");
    assert_eq!(envelopes_of(db, bob_laptop), stored);
}

/// Stores revision 1 of a new space `globex` of alice's and revision 2, which adds bob and a
/// channel of alice and bob; gives the space and the channel.
fn globex(fixture: &Fixture) -> (SpaceId, ChannelId) {
    let t = &fixture.team;
    let space = t.alice.create_space("globex");
    let revision_1 = t.alice.sign(&first(space, "globex", &t.alice));
    assert_taken(&t.alice.session.store(space, &revision_1));
    let id = t.alice.session.channel(space);
    let mut body = next(&revision_1, &t.alice.laptop);
    body.members.push(member(&t.bob, Role::Member));
    body.channels.push(channel(id, &[&t.alice, &t.bob]));
    let revision_2 = t.alice.sign(&body);
    assert_taken(&t.alice.session.store(space, &revision_2));
    (space, id)
}

/// Another space's messages are not counted with it: at 2, once `acme` has stored 2 messages,
/// `globex`, of the same accounts and devices, stores 2 of its own, and the third of each is
/// `quota-exceeded`.
#[test]
fn each_space_has_its_own_count() {
    let (fixture, mut alice) = counting(3_121_310_121, &[MONTHLY, "2"]);
    let (space, id) = globex(&fixture);
    let bob_laptop = fixture.team.bob.laptop.id;
    let in_globex = || [header(space, id, bob_laptop)];

    let mut stored = store_messages(&fixture, &mut alice, 2, &[bob_laptop]);
    let third = message_to(&fixture, &[bob_laptop]);
    assert_over_quota(&fixture, &mut alice, &third, "acme's third");
    for _ in 0..2 {
        stored.extend(store_message(&mut alice, &in_globex()));
    }
    assert_over_quota(&fixture, &mut alice, &in_globex(), "globex's third");
    let fourth = message_to(&fixture, &[bob_laptop]);
    assert_over_quota(&fixture, &mut alice, &fourth, "acme's fourth");
    assert_eq!(envelopes_of(fixture.db(), bob_laptop), stored);
}

/// A month's count does not hold in the next: at 2, once the space's 2 messages and its refused
/// third are moved to the month before, it stores 2 messages again, and the third is
/// `quota-exceeded`.
#[test]
fn the_count_starts_again_in_the_next_month() {
    let (fixture, mut alice) = counting(3_121_310_131, &[MONTHLY, "2"]);
    let [_, bob_laptop, carol, ..] = fixture.laptops();

    let mut stored = store_messages(&fixture, &mut alice, 2, &[bob_laptop, carol]);
    let third = message_to(&fixture, &[bob_laptop]);
    assert_over_quota(&fixture, &mut alice, &third, "the month's third");

    to_the_month_before(fixture.db());

    stored.extend(store_messages(&fixture, &mut alice, 2, &[bob_laptop]));
    let next_third = message_to(&fixture, &[bob_laptop]);
    assert_over_quota(&fixture, &mut alice, &next_third, "the next month's third");
    let to_bob: Vec<Vec<u8>> = [0, 2, 4, 5].map(|at| stored[at].clone()).to_vec();
    assert_eq!(envelopes_of(fixture.db(), bob_laptop), to_bob);
}

/// Without the setting, a space stores more messages than the tests' setting of 3 allows. Started
/// again on the same database with the setting at 3, by its environment variable, the server
/// stores at most 3 more of the space's messages this month, then refuses each next one with
/// `quota-exceeded`.
#[test]
fn without_the_setting_a_space_stores_any_number_and_with_it_no_more_than_the_limit() {
    let (fixture, mut alice) = counting(3_121_310_141, &[]);
    let bob_laptop = fixture.team.bob.laptop.id;
    let stored = store_messages(&fixture, &mut alice, 5, &[bob_laptop]);
    assert_eq!(envelopes_of(fixture.db(), bob_laptop), stored);
    drop(alice);

    let Fixture { team, general, .. } = fixture;
    let flow = restarted(team.flow.stand, &[(MONTHLY_ENV, "3")]);
    let mut again = Socket::of(&flow, &team.alice);
    let answers: Vec<Answer> = (0..5)
        .map(|_| {
            let to_bob = header(team.space, general, bob_laptop);
            answer_to(&mut again, &to_bob, &envelope(&to_bob))
        })
        .collect();
    let taken = answers
        .iter()
        .take_while(|answer| matches!(answer, Answer::Stored(_)))
        .count();
    assert!(taken <= 3, "stored {taken} past a limit of 3: {answers:?}");
    for answer in &answers[taken..] {
        assert!(
            matches!(answer, Answer::Refused(_, problem) if problem == QUOTA_EXCEEDED),
            "quota-exceeded after the limit, not {answer:?}"
        );
    }
}
