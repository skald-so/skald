//! The largest message: with `--message-size-limit`, an envelope whose payload is longer than the
//! setting is refused with `too-large` and stored nowhere, and the connection goes on; one whose
//! payload is as long is stored, whatever the length of its whole envelope. Without the setting,
//! any payload is taken.

use crate::fixture::Fixture;
use crate::limits::{
    KB_64, SIZE, SIZE_ENV, TOO_LARGE, assert_refused_with, assert_stored, restarted, sized,
    with_alice,
};
use crate::wire::{Socket, envelopes_of, header};

/// With the setting at 64 KB, a payload of 64 KB is stored; one of a byte more, or of twice as
/// much, is `too-large`; a payload a byte shorter is stored on the same connection after them.
#[test]
fn payload_at_the_size_limit_is_stored_and_one_byte_over_it_is_too_large() {
    let limit = KB_64.to_string();
    let (fixture, mut alice) = with_alice(3_121_310_001, &[SIZE, &limit]);
    let [_, bob, carol, ..] = fixture.laptops();

    let at = fixture.to(bob);
    let at_limit = sized(&at, KB_64);
    assert_stored(&mut alice, &at, &at_limit);

    for (what, len) in [("a byte over", KB_64 + 1), ("twice the limit", 2 * KB_64)] {
        let over = fixture.to(carol);
        let sent = sized(&over, len);
        assert_refused_with(fixture.db(), &mut alice, &over, &sent, TOO_LARGE, what);
    }

    let under = fixture.to(carol);
    let below = sized(&under, KB_64 - 1);
    assert_stored(&mut alice, &under, &below);
    assert_eq!(envelopes_of(fixture.db(), bob), [at_limit]);
    assert_eq!(envelopes_of(fixture.db(), carol), [below]);
}

/// The setting gives the limit, and the payload's length counts, not the envelope's: at 1000, a
/// payload of 1000 bytes in an envelope of more is stored, and one of 1001 is `too-large`, from
/// any sending device to any recipient.
#[test]
fn the_setting_gives_the_largest_payload() {
    let (fixture, mut alice) = with_alice(3_121_310_011, &[SIZE, "1000"]);
    let [alice_laptop, bob_laptop, carol, ..] = fixture.laptops();
    let mut bob = Socket::of(&fixture.team.flow, &fixture.team.bob);

    let to_bob = fixture.to(bob_laptop);
    let at_limit = sized(&to_bob, 1_000);
    assert!(
        at_limit.len() > 1_000,
        "an envelope longer than its payload"
    );
    assert_stored(&mut alice, &to_bob, &at_limit);

    for (what, recipient) in [("to carol", carol), ("to alice", alice_laptop)] {
        let over = fixture.to(recipient);
        let sent = sized(&over, 1_001);
        assert_refused_with(fixture.db(), &mut bob, &over, &sent, TOO_LARGE, what);
    }
    assert_eq!(envelopes_of(fixture.db(), bob_laptop), [at_limit]);
    let none: [Vec<u8>; 0] = [];
    assert_eq!(envelopes_of(fixture.db(), carol), none);
    assert_eq!(envelopes_of(fixture.db(), alice_laptop), none);
}

/// Without the setting, payloads of a byte over 64 KB and of 200 KB are stored. Started again on
/// the same database with the setting at 64 KB, by its environment variable, the server refuses a
/// payload of a byte over 64 KB with `too-large`.
#[test]
fn without_the_setting_any_payload_is_stored_and_with_it_one_over_the_limit_is_too_large() {
    let (fixture, mut alice) = with_alice(3_121_310_021, &[]);
    let [_, bob, carol, ..] = fixture.laptops();
    let stored: Vec<Vec<u8>> = [KB_64 + 1, 200_000]
        .into_iter()
        .map(|len| {
            let to_bob = fixture.to(bob);
            let sent = sized(&to_bob, len);
            assert_stored(&mut alice, &to_bob, &sent);
            sent
        })
        .collect();
    assert_eq!(envelopes_of(fixture.db(), bob), stored);
    drop(alice);

    let Fixture { team, general, .. } = fixture;
    let limit = KB_64.to_string();
    let flow = restarted(team.flow.stand, &[(SIZE_ENV, &limit)]);
    let db = &flow.stand.db;
    let mut again = Socket::of(&flow, &team.alice);
    let to_carol = header(team.space, general, carol);
    let over = sized(&to_carol, KB_64 + 1);
    assert_refused_with(
        db,
        &mut again,
        &to_carol,
        &over,
        TOO_LARGE,
        "after the restart",
    );
    assert_eq!(envelopes_of(db, bob), stored);
}
