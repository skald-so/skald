//! Expiry by time of a channel's `mls` envelopes (`docs/protocol/delivery.md`, "Mailbox"), on a
//! server with a mailbox retention of 7 days: an application message expires, while entries of
//! handshake messages, a Commit and a Welcome, do not expire by time, whatever their `ttl`.

use skald_proto::{Cbor, Envelope};

use crate::common::times::{EXPIRY, HOUR, RETENTION, WEEK};
use crate::link::{Entry, Link, await_seqs_within, seqs};
use crate::scene::{Scene, mls_header, payloads};
use crate::wire::{move_back, now};

#[test]
fn a_commit_and_a_welcome_stay_past_the_retention_while_an_application_message_expires() {
    let week = WEEK.to_string();
    let (mut s, formed) = Scene::formed_with(3_121_470_501, &[RETENTION, &week]);
    let f = &s.fixture;
    let message = s
        .alice
        .mls
        .encrypt(f.general, b"expires")
        .expect("a message");
    s.alice.stores(f, &message, &[]);

    // alice adds dave; her Commit and its Welcome are sent with a `ttl` of an hour.
    let package = s.dave.key_package();
    let admitted = s.admitted();
    let f = &s.fixture;
    let commit = (s
        .alice
        .mls
        .add_devices(f.general, &[&package], &admitted, now()))
    .expect("a Commit of the Add");
    let mut header = mls_header(f, &[]);
    header.ttl = Some(HOUR);
    s.alice.stores_with(&header, &commit);
    let welcome = s.alice.merges_and_uploads(f).0.expect("a Welcome");
    let mut header = mls_header(f, &[s.dave.id]);
    header.ttl = Some(HOUR);
    s.alice.stores_with(&header, &welcome);
    let db = f.db();
    assert_eq!(
        payloads(db, s.bob.id),
        [formed.welcome.clone(), message.clone(), commit.clone()]
    );
    assert_eq!(payloads(db, s.dave.id), std::slice::from_ref(&welcome));

    move_back(db, WEEK + HOUR);
    for peer in [&s.bob, &s.carol] {
        await_seqs_within(db, peer.id, &[1, 3], EXPIRY);
    }
    assert_eq!(seqs(db, s.dave.id), [1], "dave's Welcome stays");

    let t = &f.team;
    let mut bob = Link::of(&t.flow, &t.bob);
    bob.sync(0);
    let caught_up = bob.catch_up();
    let envelopes: Vec<Vec<u8>> = (caught_up.iter())
        .map(|entry| match entry {
            Entry::Message(_, envelope) => {
                (Envelope::from_cbor(envelope).expect("an envelope")).payload
            }
            Entry::Status(..) => panic!("a message, not {entry:?}"),
        })
        .collect();
    assert_eq!(envelopes, [formed.welcome, commit], "bob's catch-up");
}
