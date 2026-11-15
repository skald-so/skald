//! An instance whose listening connection drops listens again, then sends its connections what
//! they missed: entries, notices and the close of an ended session's connections.

use tungstenite::protocol::frame::coding::CloseCode;

use crate::common::device_flow::text;
use crate::common::devices::assert_taken;
use crate::fixture::Fixture;
use crate::instances::{CLOSE, RELISTEN, Role, WAKE, sender, store, watch};
use crate::link::{Entry, messages};
use crate::rosters::{member, next};
use crate::watch::Closed;

#[test]
fn entries_stored_while_an_instance_did_not_listen_reach_its_connections_once_it_listens_again() {
    let fixture = Fixture::new(3_121_290_051);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let role = Role::create(fixture.db());
    let listening = role.instance(&fixture);
    let bob = t.bob.laptop.id;
    let mut on_listening = watch(&listening, &fixture, &t.bob);
    let mut alice = sender(first, &fixture, &t.alice);
    let (_, before) = store(&mut alice, &fixture, bob);
    assert_eq!(on_listening.entry_within(WAKE), Entry::Message(1, before));

    // The instance's listening connection ends, and it cannot open another until the role may
    // sign in again; its other connections stay.
    role.set_login(false);
    let ended = role.end_listening();
    assert!(
        role.listening().iter().all(|pid| !ended.contains(pid)),
        "{ended:?} still listen"
    );
    let missed: Vec<Vec<u8>> = (0..2).map(|_| store(&mut alice, &fixture, bob).1).collect();
    role.set_login(true);

    assert_eq!(
        on_listening.entries_within(2, RELISTEN),
        messages(2, &missed)
    );
    // It listens again: the next entry comes at once.
    let (_, after) = store(&mut alice, &fixture, bob);
    assert_eq!(on_listening.entry_within(WAKE), Entry::Message(4, after));
    on_listening.assert_quiet();
    drop(on_listening);
    drop(listening);
}

/// Ends the listening connection of `role`'s instance, runs `meanwhile` while it cannot listen,
/// then lets it listen again.
fn without_listening(role: &Role, meanwhile: impl FnOnce()) {
    role.set_login(false);
    role.end_listening();
    meanwhile();
    role.set_login(true);
}

#[test]
fn a_revision_stored_while_an_instance_did_not_listen_sends_its_notice_once_it_listens_again() {
    let fixture = Fixture::new(3_121_290_091);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let role = Role::create(fixture.db());
    let listening = role.instance(&fixture);
    let mut carol = watch(&listening, &fixture, &t.carol);

    without_listening(&role, || {
        let mut body = next(
            fixture.revisions.last().expect("a revision"),
            &t.alice.laptop,
        );
        body.members
            .push(member(&fixture.eve, skald_proto::Role::Member));
        let revision = t.alice.sign(&body);
        assert_taken(&t.alice.session.on(first).store(t.space, &revision));
    });

    assert_eq!(carol.notice_within(RELISTEN), t.space);
    carol.assert_quiet();
    drop(carol);
    drop(listening);
}

#[test]
fn a_session_ended_while_an_instance_did_not_listen_closes_its_connections_once_it_listens_again() {
    let fixture = Fixture::new(3_121_290_101);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let role = Role::create(fixture.db());
    let listening = role.instance(&fixture);
    let mut bob = watch(&listening, &fixture, &t.bob);
    let mut alice = watch(&listening, &fixture, &t.alice);

    without_listening(&role, || {
        let ended = t.bob.session.on(first).sign_out();
        assert_eq!(ended.status, 204, "{}", text(&ended));
    });

    let session_ended = Some(Closed::Frame(CloseCode::Policy, "session-ended".to_owned()));
    assert_eq!(bob.closed_within(RELISTEN), session_ended);
    // The connection of another session stays.
    assert_eq!(alice.closed_within(CLOSE), None);
    drop((bob, alice));
    drop(listening);
}
