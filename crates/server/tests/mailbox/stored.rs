//! A sent envelope is stored once, as its bytes, in the mailbox of its recipient device, and
//! `stored` comes only once it is committed.

use std::slice;

use crate::common::Server;
use crate::common::sign_in::{CLIENT_SECRET, SECRET_ENV, github_settings};
use crate::fixture::Fixture;
use crate::wire::{Answer, Socket, envelope, envelopes_of};

#[test]
fn an_envelope_to_each_of_two_devices_is_stored_once_in_its_mailbox() {
    let fixture = Fixture::new(3_121_230_001);
    let t = &fixture.team;
    let mut alice = Socket::of(&t.flow, &t.alice);

    let to_bob = fixture.to(t.bob.laptop.id);
    let for_bob = envelope(&to_bob);
    alice.store(&to_bob, &for_bob);
    // Committed by the time `stored` comes.
    assert_eq!(
        envelopes_of(fixture.db(), t.bob.laptop.id),
        slice::from_ref(&for_bob)
    );

    let to_carol = fixture.to(t.carol.laptop.id);
    let for_carol = envelope(&to_carol);
    alice.store(&to_carol, &for_carol);
    assert_eq!(envelopes_of(fixture.db(), t.carol.laptop.id), [for_carol]);

    assert_eq!(envelopes_of(fixture.db(), t.bob.laptop.id), [for_bob]);
    for device in [
        &t.alice.laptop,
        &t.alice.uncertified,
        &t.dave.laptop,
        &fixture.eve.laptop,
    ] {
        assert_eq!(envelopes_of(fixture.db(), device.id), Vec::<Vec<u8>>::new());
    }
}

#[test]
fn one_msg_id_to_two_recipients_stores_two_envelopes() {
    let fixture = Fixture::new(3_121_230_011);
    let t = &fixture.team;
    let mut alice = Socket::of(&t.flow, &t.alice);
    let to_bob = fixture.to(t.bob.laptop.id);
    let mut to_carol = fixture.to(t.carol.laptop.id);
    to_carol.msg_id = to_bob.msg_id;
    let (for_bob, for_carol) = (envelope(&to_bob), envelope(&to_carol));

    alice.store(&to_bob, &for_bob);
    alice.store(&to_carol, &for_carol);

    assert_eq!(envelopes_of(fixture.db(), t.bob.laptop.id), [for_bob]);
    assert_eq!(envelopes_of(fixture.db(), t.carol.laptop.id), [for_carol]);
}

#[test]
fn a_server_killed_right_after_stored_still_holds_the_envelope() {
    let fixture = Fixture::new(3_121_230_021);
    let t = &fixture.team;
    let stand = &t.flow.stand;
    // Another instance on the database, whose connections the test kills with it.
    let settings = github_settings(&stand.public, &stand.github);
    let mut killed = Server::launch(
        &stand.db,
        "127.0.0.1:0",
        &settings,
        &[(SECRET_ENV, CLIENT_SECRET)],
    );
    let header = fixture.to(t.bob.laptop.id);
    let sent = envelope(&header);

    let mut alice = Socket::to(&killed, &t.flow, &t.alice, &t.alice.laptop);
    let first = alice.store(&header, &sent);
    killed.stop();

    assert_eq!(
        envelopes_of(fixture.db(), t.bob.laptop.id),
        slice::from_ref(&sent)
    );
    // The other instance knows the send: a retry gets the first answer.
    let mut again = Socket::of(&t.flow, &t.alice);
    again.send(&sent);
    assert_eq!(again.answer(), Answer::Stored(first));
    assert_eq!(envelopes_of(fixture.db(), t.bob.laptop.id), [sent]);
}
