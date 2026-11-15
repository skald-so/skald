//! An instance whose listening connection drops listens again, then sends its connections what
//! they missed.

use crate::fixture::Fixture;
use crate::instances::{RELISTEN, Role, WAKE, sender, store, watch};
use crate::link::{Entry, messages};

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
