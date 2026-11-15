//! What one instance stores or adds reaches a device connected to another at once: an envelope, a
//! `delivered` and a `space-notice`.

use skald_proto::Role;

use crate::common::devices::assert_taken;
use crate::fixture::Fixture;
use crate::instances::{WAKE, instance, sender, store, watch};
use crate::link::{Entry, assert_delivered};
use crate::rosters::{member, next};
use crate::watch::Watch;
use crate::wire::now;

#[test]
fn an_envelope_stored_through_one_instance_reaches_a_device_on_another_at_once() {
    let fixture = Fixture::new(3_121_290_001);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let other = instance(&fixture, &[]);
    let bob = t.bob.laptop.id;

    // bob's laptop on the first instance, alice's sends through the other.
    let mut on_first = watch(first, &fixture, &t.bob);
    let mut alice = sender(&other, &fixture, &t.alice);
    for seq in 1..=3 {
        let (_, sent) = store(&mut alice, &fixture, bob);
        assert_eq!(on_first.entry_within(WAKE), Entry::Message(seq, sent));
    }
    on_first.assert_quiet();
    drop(on_first);

    // The other way round: bob's laptop on the other instance, alice's sends through the first.
    let mut on_other = Watch::from(&other, &t.flow, &t.bob, 3, &[]);
    let mut alice = sender(first, &fixture, &t.alice);
    for seq in 4..=5 {
        let (_, sent) = store(&mut alice, &fixture, bob);
        assert_eq!(on_other.entry_within(WAKE), Entry::Message(seq, sent));
    }
    on_other.assert_quiet();
}

#[test]
fn each_connection_of_a_device_gets_what_another_instance_stores() {
    let fixture = Fixture::new(3_121_290_011);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let other = instance(&fixture, &[]);
    let third = instance(&fixture, &[]);
    let bob = t.bob.laptop.id;
    let mut on_first = watch(first, &fixture, &t.bob);
    let mut on_third = watch(&third, &fixture, &t.bob);

    let mut alice = sender(&other, &fixture, &t.alice);
    let (_, sent) = store(&mut alice, &fixture, bob);

    let expected = Entry::Message(1, sent);
    assert_eq!(on_first.entry_within(WAKE), expected);
    assert_eq!(on_third.entry_within(WAKE), expected);
    on_first.assert_quiet();
    on_third.assert_quiet();
}

#[test]
fn delivered_added_through_one_instance_reaches_the_sender_on_another_at_once() {
    let fixture = Fixture::new(3_121_290_021);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let other = instance(&fixture, &[]);
    let bob = t.bob.laptop.id;
    let mut alice = watch(first, &fixture, &t.alice);
    let (header, sent) = store(&mut sender(first, &fixture, &t.alice), &fixture, bob);

    let mut on_other = Watch::from(&other, &t.flow, &t.bob, 0, &[Entry::Message(1, sent)]);
    let before = now();
    on_other.ack(1);

    let delivered = alice.entry_within(WAKE);
    assert_eq!(assert_delivered(&delivered, &header, bob, before), 1);
    alice.assert_quiet();
}

#[test]
fn a_revision_stored_through_one_instance_sends_a_space_notice_on_another_at_once() {
    let fixture = Fixture::new(3_121_290_031);
    let other = instance(&fixture, &[]);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let mut bob = watch(first, &fixture, &t.bob);
    let mut carol = watch(first, &fixture, &t.carol);

    let mut body = next(
        fixture.revisions.last().expect("a revision"),
        &t.alice.laptop,
    );
    body.members.push(member(&fixture.eve, Role::Member));
    let revision = t.alice.sign(&body);
    assert_taken(&t.alice.session.on(&other).store(t.space, &revision));

    assert_eq!(bob.notice_within(WAKE), t.space);
    assert_eq!(carol.notice_within(WAKE), t.space);
    bob.assert_quiet();
    carol.assert_quiet();
}
