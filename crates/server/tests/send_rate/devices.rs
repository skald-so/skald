//! Each device has its own count: another device's sends, of the same account or another, are not
//! counted with it; one device's sends through two instances of the server count together, over a
//! WebSocket and over HTTPS alike.

use crate::https::Https;
use crate::rate::{
    RATE, assert_over_rate, assert_rate_limited, message_to, second_device, stores, with_alice,
};
use crate::wire::{Socket, assert_nowhere, envelope};

/// With the setting at 2, alice's laptop, her phone and bob's laptop each send a message in turn,
/// twice: each is stored. Then the third of each is `rate-limited`.
#[test]
fn another_device_s_sends_are_not_counted_with_it() {
    let (fixture, mut laptop) = with_alice(3_216_060_401, &[RATE, "2"]);
    let t = &fixture.team;
    let db = fixture.db();
    let [alice, bob, carol, ..] = fixture.laptops();
    let phone = second_device(db, &t.alice, "alice-phone");
    let mut phone = Socket::to(&t.flow.stand.server, &t.flow, &t.alice, &phone);
    let mut bobs = Socket::of(&t.flow, &t.bob);

    for _ in 0..2 {
        stores(&mut laptop, &message_to(&fixture, &[bob]));
        stores(&mut phone, &message_to(&fixture, &[carol]));
        stores(&mut bobs, &message_to(&fixture, &[alice]));
    }

    assert_over_rate(
        db,
        &mut laptop,
        &message_to(&fixture, &[bob]),
        "laptop's third",
    );
    assert_over_rate(
        db,
        &mut phone,
        &message_to(&fixture, &[carol]),
        "phone's third",
    );
    assert_over_rate(
        db,
        &mut bobs,
        &message_to(&fixture, &[alice]),
        "bob's third",
    );
}

/// With the setting at 3 on two instances of the server on one database, alice's laptop sends
/// over a WebSocket to the first and over HTTPS to the other, in turn: 3 messages are stored, and
/// the next one through each instance is `rate-limited`. The HTTPS answers conform to the spec.
#[test]
fn one_device_s_sends_through_two_instances_count_together() {
    let (fixture, mut first) = with_alice(3_216_060_411, &[RATE, "3"]);
    let t = &fixture.team;
    let db = fixture.db();
    let other = t.flow.stand.instance(&[RATE, "3"], &[]);
    let second = Https::to(&other, &t.flow, &t.alice, &t.alice.laptop);
    let bob = t.bob.laptop.id;

    stores(&mut first, &message_to(&fixture, &[bob]));
    let header = fixture.to(bob);
    second.store(&header, &envelope(&header));
    stores(&mut first, &message_to(&fixture, &[bob]));

    let header = fixture.to(bob);
    let sent = envelope(&header);
    assert_rate_limited(&second.send(&sent, true), "the fourth, over the other");
    assert_nowhere(db, &sent);
    assert_over_rate(
        db,
        &mut first,
        &message_to(&fixture, &[bob]),
        "the fifth, over the first",
    );
}
