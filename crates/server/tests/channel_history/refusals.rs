//! Uploads the server refuses, storing nothing of them ("Uploads" and "History entry"): from a
//! device outside the item's epoch, a revoked device or a connection that proves no device; an
//! entry of an epoch before the previous one or from a device outside the current epoch; one
//! naming another channel or an epoch the channel has not had; bytes that do not decode.

use std::io::ErrorKind;
use std::time::Duration;

use skald_proto::{ChannelId, MessageId, PROTOCOL_VERSION};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;

use crate::common::device_flow::text;
use crate::common::devices::{Device, random};
use crate::fixture::Fixture;
use crate::kit::{
    Answer, ENTRIES, Item, assert_given, assert_history, assert_refused_upload, attempt,
    bob_rotates, entry, entry_of, formed, history_frame, link, link_bytes, refuses, stores,
    upload_to, wrap,
};
use crate::scene::{removing, revoke};
use crate::wire::{Socket, assert_nowhere};

/// How long a test waits for the server to close a connection.
const WAIT: Duration = Duration::from_secs(20);

#[test]
fn uploads_from_devices_outside_the_items_epoch_are_refused() {
    let (mut s, _) = formed(3_160_110_201);
    s.add_dave();
    let carol = s.carol.id;
    let removal = s.alice.commits(&s.fixture, removing(&[carol]));
    let f = &s.fixture;
    for peer in [&mut s.bob, &mut s.dave] {
        peer.process(f, &removal);
    }
    // Epoch 1: alice, bob and carol; epoch 2: dave too; epoch 3: alice, bob and dave.
    let t = &f.team;
    let g = f.general;

    let daves = wrap(g, 1, &t.alice, &t.dave);
    refuses(&mut s.dave, f, &daves, "dave's wrap of epoch 1, before his");
    let carols_link = link(g, 3, 2);
    refuses(
        &mut s.carol,
        f,
        &carols_link,
        "the removed carol's link of epoch 3",
    );
    let carols_wrap = wrap(g, 3, &t.alice, &t.carol);
    refuses(
        &mut s.carol,
        f,
        &carols_wrap,
        "the removed carol's wrap of epoch 3",
    );
    let eves = [link(g, 2, 1), wrap(g, 1, &t.alice, &f.eve), entry(g, 3)];
    for item in &eves {
        refuses(&mut s.eve, f, item, "eve's, outside the space");
    }

    // The members of each epoch upload the same keys.
    let bobs_link = link(g, 3, 2);
    stores(&mut s.bob, f, &bobs_link, "bob's link of epoch 3");
    let daves_wrap = wrap(g, 2, &t.alice, &t.dave);
    stores(&mut s.dave, f, &daves_wrap, "dave's wrap of epoch 2");
    let bobs_wrap = wrap(g, 3, &t.alice, &t.bob);
    stores(&mut s.bob, f, &bobs_wrap, "bob's wrap of epoch 3");
    let given = [&[&bobs_link][..], &[&daves_wrap, &bobs_wrap], &[]];
    assert_history(&s.alice.session, f, given, "the members' alone");
}

#[test]
fn entries_of_an_epoch_before_the_previous_one_or_from_a_device_the_last_commit_removed_are_refused()
 {
    let (mut s, _) = formed(3_160_110_211);
    bob_rotates(&mut s);
    let carol = s.carol.id;
    let removal = s.alice.commits(&s.fixture, removing(&[carol]));
    let f = &s.fixture;
    s.bob.process(f, &removal);
    // Epoch 3, the current one, without carol; epoch 2, the previous one, with her.
    let g = f.general;

    for epoch in [2, 3] {
        let carols = entry(g, epoch);
        refuses(
            &mut s.carol,
            f,
            &carols,
            &format!("carol's of epoch {epoch}"),
        );
    }
    for peer in [&mut s.alice, &mut s.bob] {
        let first = entry(g, 1);
        refuses(peer, f, &first, "of epoch 1, before the previous one");
    }
    let previous = entry(g, 2);
    stores(&mut s.bob, f, &previous, "bob's of the previous epoch");
    let current = entry(g, 3);
    stores(&mut s.bob, f, &current, "bob's of the current epoch");
    assert_given(
        &s.alice.session,
        f,
        ENTRIES,
        &[&previous, &current],
        "bob's",
    );
}

#[test]
fn uploads_naming_another_channel_or_an_epoch_the_channel_has_not_had_are_refused() {
    let (mut s, _) = formed(3_160_110_221);
    bob_rotates(&mut s);
    let f = &s.fixture;
    let t = &f.team;
    let g = f.general;
    let other = ChannelId::generate();

    let of_another = [
        link(other, 2, 1),
        wrap(other, 1, &t.alice, &t.alice),
        entry(other, 2),
    ];
    for item in &of_another {
        refuses(&mut s.alice, f, item, "an item of another channel");
    }
    let ahead = [
        link(g, 3, 2),
        link(g, 4, 1),
        wrap(g, 3, &t.alice, &t.alice),
        entry(g, 3),
    ];
    for item in &ahead {
        refuses(
            &mut s.alice,
            f,
            item,
            "an item of an epoch after the current one",
        );
    }
    let elsewhere = [link(g, 2, 1), wrap(g, 2, &t.alice, &t.alice), entry(g, 2)];
    for item in &elsewhere {
        let answer = upload_to(&mut s.alice, t.space, other, item);
        assert_refused_upload(&answer, f, item, "the channel's item uploaded to another");
    }

    let link = link(g, 2, 1);
    stores(&mut s.alice, f, &link, "alice's link of epoch 2");
    assert_history(
        &s.bob.session,
        f,
        [&[&link], &[], &[]],
        "alice's link alone",
    );
}

#[test]
fn uploads_whose_bytes_do_not_decode_are_refused() {
    let (mut s, _) = formed(3_160_110_231);
    bob_rotates(&mut s);
    let f = &s.fixture;
    let t = &f.team;
    let g = f.general;
    let a_link = link_bytes(g, 2, 1);
    let Item::Wrap(a_wrap) = wrap(g, 2, &t.alice, &t.bob) else {
        unreachable!("a wrap")
    };
    let cases = [
        (Item::Link(random().to_vec()), "random bytes as a link"),
        (Item::Wrap(random().to_vec()), "random bytes as a wrap"),
        (
            Item::Entry(MessageId::generate(), random().to_vec()),
            "random bytes as an entry",
        ),
        (
            Item::Link(a_link[..a_link.len() - 5].to_vec()),
            "a truncated link",
        ),
        (Item::Wrap(a_link.clone()), "a link as a wrap"),
        (
            Item::Entry(MessageId::generate(), a_wrap.clone()),
            "a wrap as an entry",
        ),
        (Item::Link(link_bytes(g, 1, 1)), "a link of epoch 1"),
        (Item::Link(link_bytes(g, 2, 0)), "a link to epoch 0"),
        (entry_of(MessageId::generate(), g, 0), "an entry of epoch 0"),
    ];
    for (item, what) in &cases {
        refuses(&mut s.bob, f, item, what);
    }

    let link = Item::Link(a_link);
    stores(&mut s.bob, f, &link, "the link, whole");
    assert_history(
        &s.carol.session,
        f,
        [&[&link], &[], &[]],
        "the whole link alone",
    );
}

#[test]
fn uploads_from_a_revoked_device_are_not_stored() {
    let (mut s, _) = formed(3_160_110_241);
    let f = &s.fixture;
    let t = &f.team;
    let g = f.general;
    // bob's laptop is still a member of epoch 1 of the group.
    revoke(&t.bob);
    let bobs = [wrap(g, 1, &t.dave, &t.bob), entry(g, 1)];
    for item in &bobs {
        let answer = attempt(&mut s.bob.socket, t.space, g, item);
        assert!(
            !matches!(answer, Some(Answer::Stored)),
            "an upload from the revoked bob is not stored"
        );
        assert_nowhere(f.db(), item.bytes());
    }

    let alices = wrap(g, 1, &t.dave, &t.alice);
    stores(&mut s.alice, f, &alices, "alice's wrap to dave");
    assert_history(
        &s.dave.session,
        f,
        [&[], &[&alices], &[]],
        "alice's wrap alone",
    );
}

/// Opens a connection to the fixture's server and sends `frame` as its first frame, with no
/// `auth`; checks that the server closes it.
#[track_caller]
fn first_without_auth(f: &Fixture, frame: Vec<u8>) {
    let offer = skald_proto::subprotocol(PROTOCOL_VERSION);
    let server = &f.team.flow.stand.server;
    let (mut socket, _) = (server.open_websocket("/ws", Some(&offer)))
        .unwrap_or_else(|response| panic!("/ws refused: {}", text(&response)));
    if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
        stream.set_read_timeout(Some(WAIT)).expect("a timeout");
    }
    socket
        .send(Message::Binary(frame.into()))
        .expect("send the upload");
    loop {
        match socket.read() {
            Err(tungstenite::Error::Io(err))
                if [ErrorKind::WouldBlock, ErrorKind::TimedOut].contains(&err.kind()) =>
            {
                panic!("still open after {WAIT:?}")
            }
            Ok(Message::Close(_)) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

#[test]
fn uploads_on_a_connection_that_proves_no_device_are_not_stored() {
    let (mut s, _) = formed(3_160_110_251);
    bob_rotates(&mut s);
    let f = &s.fixture;
    let t = &f.team;
    let g = f.general;

    // With alice's session but not her laptop's key: no `auth`, an `auth` naming her laptop signed
    // by another key, and one of her device without a certificate.
    let unproved = [link(g, 2, 1), wrap(g, 2, &t.dave, &t.alice), entry(g, 2)];
    for (upload, item) in (1..).zip(&unproved) {
        first_without_auth(f, history_frame(upload, t.space, g, item));
    }
    let mut forged = Device::new("Forged");
    forged.id = t.alice.laptop.id;
    let server = &t.flow.stand.server;
    for device in [&forged, &t.alice.uncertified] {
        let mut socket = Socket::to(server, &t.flow, &t.alice, device);
        for item in &unproved {
            let answer = attempt(&mut socket, t.space, g, item);
            assert!(
                !matches!(answer, Some(Answer::Stored)),
                "an upload on a connection of {} is not stored",
                device.name
            );
        }
    }
    for item in &unproved {
        assert_nowhere(f.db(), item.bytes());
    }

    // alice's laptop uploads the same keys.
    let link = link(g, 2, 1);
    let wrap = wrap(g, 2, &t.dave, &t.alice);
    for item in [&link, &wrap] {
        stores(&mut s.alice, f, item, "from alice's laptop");
    }
    let given = [&[&link][..], &[&wrap], &[]];
    assert_history(&s.dave.session, f, given, "alice's laptop's alone");
}
