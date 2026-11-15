//! An instance whose listening connection drops listens again, then sends its connections what
//! they missed: entries, notices and the close of an ended session's connections. It also notices a
//! listening connection that goes silent, and takes no connection before it first listens.

use std::thread;
use std::time::{Duration, Instant};

use tungstenite::protocol::frame::coding::CloseCode;

use crate::common::Closed;
use crate::common::device_flow::text;
use crate::common::devices::assert_taken;
use crate::fixture::Fixture;
use crate::instances::{CLOSE, RELISTEN, Role, WAKE, sender, store, watch};
use crate::link::{Entry, messages};
use crate::proxy::Proxy;
use crate::rosters::{member, next};
use crate::watch::Watch;

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

#[test]
fn a_revision_and_a_sign_out_an_instance_missed_reach_it_once_it_listens_again() {
    let fixture = Fixture::new(3_121_290_091);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let role = Role::create(fixture.db());
    let listening = role.instance(&fixture);
    let mut bob = watch(&listening, &fixture, &t.bob);
    let mut carol = watch(&listening, &fixture, &t.carol);

    role.set_login(false);
    role.end_listening();
    let mut body = next(
        fixture.revisions.last().expect("a revision"),
        &t.alice.laptop,
    );
    body.members
        .push(member(&fixture.eve, skald_proto::Role::Member));
    let revision = t.alice.sign(&body);
    assert_taken(&t.alice.session.on(first).store(t.space, &revision));
    let ended = t.bob.session.on(first).sign_out();
    assert_eq!(ended.status, 204, "{}", text(&ended));
    role.set_login(true);

    assert_eq!(carol.notice_within(RELISTEN), t.space);
    carol.assert_quiet();
    let session_ended = Some(Closed::Frame(CloseCode::Policy, "session-ended".to_owned()));
    assert_eq!(bob.closed_within(WAKE), session_ended);
    // The connection of another session stays.
    assert_eq!(carol.closed_within(CLOSE), None);
    drop((bob, carol));
    drop(listening);
}

/// How long a proved connection waits for its instance to listen.
const LISTEN_WAIT: Duration = Duration::from_secs(5);

#[test]
fn an_instance_whose_first_listening_is_refused_takes_connections_once_it_listens() {
    let fixture = Fixture::new(3_121_290_111);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let proxy = Proxy::to(fixture.db());
    proxy.refuse_listening(true);
    let behind = fixture
        .team
        .flow
        .stand
        .instance_at(&proxy.url(fixture.db()), &[], &[]);
    let deadline = Instant::now() + RELISTEN;
    while proxy.refused() == 0 {
        assert!(
            Instant::now() < deadline,
            "no listening refused after {RELISTEN:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }

    // Until it listens, it takes no connection.
    let mut early = Watch::on(&behind, &t.flow, &t.bob);
    assert_eq!(
        early.closed_within(LISTEN_WAIT + CLOSE),
        Some(Closed::Frame(CloseCode::Error, "server-error".to_owned()))
    );

    proxy.refuse_listening(false);
    let mut bob = watch(&behind, &fixture, &t.bob);
    let (_, sent) = store(
        &mut sender(first, &fixture, &t.alice),
        &fixture,
        t.bob.laptop.id,
    );
    assert_eq!(bob.entry_within(WAKE), Entry::Message(1, sent));
    bob.assert_quiet();
    drop(bob);
    drop(behind);
}

#[test]
fn entries_stored_after_a_listening_connection_went_silent_reach_its_instance_s_connections() {
    let fixture = Fixture::new(3_121_290_121);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let proxy = Proxy::to(fixture.db());
    let behind = fixture
        .team
        .flow
        .stand
        .instance_at(&proxy.url(fixture.db()), &[], &[]);
    let bob = t.bob.laptop.id;
    let mut on_behind = watch(&behind, &fixture, &t.bob);
    let mut alice = sender(first, &fixture, &t.alice);

    // The listening connection stays open but carries nothing, as after a network partition.
    assert_eq!(proxy.stop_listening(), 1, "one listening connection");
    let (_, missed) = store(&mut alice, &fixture, bob);

    assert_eq!(on_behind.entry_within(RELISTEN), Entry::Message(1, missed));
    // It listens again: the next entry comes at once.
    let (_, after) = store(&mut alice, &fixture, bob);
    assert_eq!(on_behind.entry_within(WAKE), Entry::Message(2, after));
    on_behind.assert_quiet();
    drop(on_behind);
    drop(behind);
}
