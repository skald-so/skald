//! An instance whose listening connection drops listens again, then sends its connections what
//! they missed: entries, notices, a device that started waiting for linking, the close of an
//! ended session's or a revoked device's connections, and an account's deletion; no notice it sent
//! before. It also notices a listening connection that goes silent, and takes no connection before
//! it first listens.

use std::time::Duration;

use skald_proto::{PROTOCOL_VERSION, VERSION_HEADER};
use tungstenite::protocol::frame::coding::CloseCode;

use crate::api::Person;
use crate::common::device_flow::text;
use crate::common::devices::assert_taken;
use crate::common::{Closed, Server, delete_url};
use crate::fixture::Fixture;
use crate::instances::{CLOSE, RELISTEN, Role, WAKE, relisten_quietly, sender, store, watch};
use crate::link::{Entry, messages};
use crate::link_kit::{fresh_session, hello};
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
fn a_revision_run_sign_out_and_revocation_an_instance_missed_reach_it_once_it_listens_again() {
    let fixture = Fixture::new(3_121_290_091);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let role = Role::create(fixture.db());
    let listening = role.instance(&fixture);
    let mut bob = watch(&listening, &fixture, &t.bob);
    let mut carol = watch(&listening, &fixture, &t.carol);
    let mut dave = watch(&listening, &fixture, &t.dave);

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
    let hello = hello(&fresh_session());
    assert_taken(
        &t.carol
            .session
            .on(first)
            .send_hello(t.carol.uncertified.id, &hello),
    );
    let ended = t.bob.session.on(first).sign_out();
    assert_eq!(ended.status, 204, "{}", text(&ended));
    t.dave.revoke(&t.dave.by_master(t.dave.laptop.id, &[]));
    role.set_login(true);

    assert_eq!(carol.notice_within(RELISTEN), t.space);
    carol.link_notice_within(WAKE);
    carol.assert_quiet();
    let session_ended = Some(Closed::Frame(CloseCode::Policy, "session-ended".to_owned()));
    assert_eq!(bob.closed_within(WAKE), session_ended);
    let device_refused = Some(Closed::Frame(
        CloseCode::Policy,
        "device-refused".to_owned(),
    ));
    assert_eq!(dave.closed_within(WAKE), device_refused);
    // The connection of another session and device stays.
    assert_eq!(carol.closed_within(CLOSE), None);
    drop((bob, carol, dave));
    drop(listening);
}

/// While an instance does not listen, carol deletes her account through another. Once it listens
/// again, it closes carol's connection with the reason `account-deleted` and sends bob's the notice
/// of `acme`, which keeps her account; once it listens again after that, no notice of it.
#[test]
fn an_account_deletion_an_instance_missed_reaches_it_once_it_listens_again() {
    let fixture = Fixture::new(3_121_290_131);
    let t = &fixture.team;
    let role = Role::create(fixture.db());
    let listening = role.instance(&fixture);
    let mut carol = watch(&listening, &fixture, &t.carol);
    let mut bob = watch(&listening, &fixture, &t.bob);
    let mut alice = sender(&t.flow.stand.server, &fixture, &t.alice);

    role.set_login(false);
    role.end_listening();
    delete_account(&t.flow.stand.server, &t.carol);
    role.set_login(true);

    assert_eq!(bob.notice_within(RELISTEN), t.space);
    bob.assert_quiet();
    let account_deleted = Some(Closed::Frame(
        CloseCode::Policy,
        "account-deleted".to_owned(),
    ));
    assert_eq!(carol.closed_within(WAKE), account_deleted);
    relisten_quietly(&role, &fixture, &mut alice, &mut bob, 1);
    drop((bob, carol, alice));
    drop(listening);
}

/// Once an instance knows of a member deletion, it sends no notice of it when it listens again:
/// carol's, told by another instance, nor dave's, through itself.
#[test]
fn an_instance_that_listens_again_sends_no_notice_of_a_deletion_it_knew_of() {
    let fixture = Fixture::new(3_121_290_141);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let mut alice = sender(first, &fixture, &t.alice);
    let role = Role::create(fixture.db());
    let listening = role.instance(&fixture);
    let mut bob = watch(&listening, &fixture, &t.bob);

    delete_account(first, &t.carol);
    assert_eq!(bob.notice_within(WAKE), t.space, "carol's deletion");
    relisten_quietly(&role, &fixture, &mut alice, &mut bob, 1);
    delete_account(&listening, &t.dave);
    assert_eq!(bob.notice_within(WAKE), t.space, "dave's deletion");
    relisten_quietly(&role, &fixture, &mut alice, &mut bob, 2);
    drop((bob, alice));
    drop(listening);
}

/// `person`'s device session deletes its account through `server`.
#[track_caller]
fn delete_account(server: &Server, person: &Person) {
    let url = format!("{}/account", server.base());
    let authorization = format!("Bearer {}", person.session.token());
    let version = PROTOCOL_VERSION.to_string();
    let headers = [
        ("authorization", authorization.as_str()),
        (VERSION_HEADER, version.as_str()),
    ];
    let deleted = delete_url(&url, &headers);
    assert_eq!(deleted.status, 204, "{}", text(&deleted));
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
    proxy.await_refused(RELISTEN);

    // Until it listens, it takes no connection, and writes why to its output.
    let mut early = Watch::on(&behind, &t.flow, &t.bob);
    assert_eq!(
        early.closed_within(LISTEN_WAIT + CLOSE),
        Some(Closed::Frame(CloseCode::Error, "server-error".to_owned()))
    );
    let why = "skald-server: cannot serve a connection: the instance does not listen to the others \
               yet\n";
    behind.assert_prints_within(why, CLOSE);

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
