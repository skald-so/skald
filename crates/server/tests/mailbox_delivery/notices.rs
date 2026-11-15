//! Notices of new roster revisions (`docs/protocol/delivery.md`, "Frames", and
//! `docs/protocol/membership.md`, "Roster"): once the server has stored a revision, it sends
//! `space-notice` with the space to each connection of the devices of the accounts that get the
//! space, and to no other. It is not a mailbox entry: a device offline at the time gets none later.
//!
//! The interfaces, as these tests use them, besides those of delivering a mailbox:
//! - `space-notice = [9, space: id]` from the server, alone or in a `batch`;
//! - the accounts that get a space: its creator's before revision 1, then the members of its
//!   latest revision, as the tests of spaces say;
//! - the table `devices`, which a test writes a second certified `client` device of an account to,
//!   with the columns `id`, `account_id`, `kind`, `signing`, `hpke`, `name` and `cert`.

use skald_proto::Role;

use crate::common::devices::assert_taken;
use crate::fixture::Fixture;
use crate::listener::{Listener, second_device};
use crate::rosters::{first, member, next};
use crate::wire::mailbox;

/// A revision of `acme` sends a notice of `acme` to each connection of each device of its members'
/// accounts: alice's, who stores it, bob's two connections of one device and his second device,
/// carol's and dave's; eve's, outside the space, gets none.
#[test]
fn revision_notifies_every_connection_of_every_device_of_the_members() {
    let mut fixture = Fixture::new(3_121_280_001);
    let tablet = second_device(fixture.db(), &fixture.team.bob, "Tablet");
    let flow = &fixture.team.flow;
    let t = &fixture.team;
    let mut members = vec![
        Listener::of(flow, &t.alice),
        Listener::of(flow, &t.bob),
        Listener::of(flow, &t.bob),
        Listener::device(flow, &t.bob, &tablet),
        Listener::of(flow, &t.carol),
        Listener::of(flow, &t.dave),
    ];
    let mut eve = Listener::of(flow, &fixture.eve);
    let space = t.space;

    fixture.revise(|_| {});

    for listener in &mut members {
        listener.assert_notice(space);
    }
    eve.assert_no_notice();
}

/// Revision 1 of eve's own space `globex` sends a notice of `globex` to eve's device, its creator's,
/// and none to bob's, a device of no account that gets `globex`; a revision of `acme` then sends
/// bob's a notice of `acme` and eve's none.
#[test]
fn revision_notifies_only_the_accounts_of_its_space() {
    let mut fixture = Fixture::new(3_121_280_101);
    let flow = &fixture.team.flow;
    let mut bob = Listener::of(flow, &fixture.team.bob);
    let mut eve = Listener::of(flow, &fixture.eve);
    let globex = fixture.eve.create_space("globex");
    let revision = fixture.eve.sign(&first(globex, "globex", &fixture.eve));

    assert_taken(&fixture.eve.session.store(globex, &revision));

    eve.assert_notice(globex);
    bob.assert_no_notice();

    fixture.revise(|_| {});

    bob.assert_notice(fixture.team.space);
    eve.assert_no_notice();
}

/// A revision that adds eve's account to `acme` sends eve's device a notice of `acme`: her account
/// gets the space from that revision on.
#[test]
fn revision_that_adds_an_account_notifies_its_devices() {
    let mut fixture = Fixture::new(3_121_280_201);
    let mut eve = Listener::of(&fixture.team.flow, &fixture.eve);
    let added = member(&fixture.eve, Role::Member);

    fixture.revise(|body| body.members.push(added));

    eve.assert_notice(fixture.team.space);
}

/// A revision stored while bob's device is offline is no mailbox entry: bob's mailbox stays empty,
/// and his device, connecting after it, gets no notice of it, in catch-up or after; it gets one of
/// the next revision, stored while it is connected.
#[test]
fn device_that_connects_after_a_revision_gets_no_notice_of_it() {
    let mut fixture = Fixture::new(3_121_280_301);
    let bob = fixture.team.bob.laptop.id;

    fixture.revise(|_| {});

    assert!(
        mailbox(fixture.db(), bob).is_empty(),
        "bob's mailbox holds an entry"
    );
    // `Listener::of` checks that catch-up holds no notice.
    let mut listener = Listener::of(&fixture.team.flow, &fixture.team.bob);
    listener.assert_no_notice();
    fixture.revise(|_| {});
    listener.assert_notice(fixture.team.space);
    assert!(
        mailbox(fixture.db(), bob).is_empty(),
        "bob's mailbox holds an entry"
    );
}

/// A revision the server refuses, signed by carol's device, which is no admin's, and stored from
/// her session, sends no notice; the next one it stores does.
#[test]
fn refused_revision_sends_no_notice() {
    let mut fixture = Fixture::new(3_121_280_401);
    let mut bob = Listener::of(&fixture.team.flow, &fixture.team.bob);
    let t = &fixture.team;
    let last = fixture.revisions.last().expect("a revision");
    let refused = t.carol.sign(&next(last, &t.carol.laptop));
    t.refused(
        &t.carol,
        &refused,
        &fixture.revisions,
        "a revision by carol",
    );

    bob.assert_no_notice();

    fixture.revise(|_| {});
    bob.assert_notice(fixture.team.space);
}
