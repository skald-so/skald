//! Uploads the server refuses, keeping nothing of them: from a device outside the object's epoch, a
//! revoked device, a `service` device outside the group or a connection that proves no device; for
//! an epoch the channel has not had; naming another channel than the upload, or bytes that do not
//! decode as a queue key object.

use skald_proto::ChannelId;

use crate::bot::{socket, token};
use crate::common::devices::{Device, random};
use crate::history::Item;
use crate::kit::World;
use crate::objects::{
    EPOCH_MISMATCH, MALFORMED, NOT_A_MEMBER, assert_given, assert_none_given, keeps, not_kept,
    object, refuses, remove_carol,
};
use crate::scene::{Peer, revoke};
use crate::tokens::{QUEUES, RECEIVE, SEND};
use crate::wire::Socket;

/// alice's laptop commits carol's Remove: epoch 2, of alice and bob. Refused with `not-a-member`:
/// carol's object of epoch 2, after her removal; objects of epoch 1 from dave's laptop, outside
/// the channel, and eve's, outside the space; and an object of epoch 1 from the bot, outside the
/// group, on a connection of its token with `send`, `receive` and `queues` for `general`. The
/// server gives no object of either epoch; bob's object of epoch 2 is then kept.
#[test]
fn objects_from_devices_outside_the_epoch_are_refused() {
    let mut w = World::new(5_418_200_001);
    let [alice_reads, ..] = w.laptops();
    remove_carol(&mut w);
    let (db, at) = (w.stage.f.db(), (w.space, w.channel));
    let (t, eve) = (&w.stage.f.team, &w.stage.f.eve);
    let mut dave = Peer::of(&w.stage.f, &t.dave);
    let mut eve_peer = Peer::of(&w.stage.f, eve);

    let refused = Some(NOT_A_MEMBER);
    let carols = object(w.channel, 2, &t.carol.laptop);
    refuses(
        (db, at),
        &mut w.carol,
        &carols,
        refused,
        "the removed carol's",
    );
    let daves = object(w.channel, 1, &t.dave.laptop);
    refuses(
        (db, at),
        &mut dave,
        &daves,
        refused,
        "dave's, outside the channel",
    );
    let eves = object(w.channel, 1, &eve.laptop);
    refuses(
        (db, at),
        &mut eve_peer,
        &eves,
        refused,
        "eve's, outside the space",
    );
    let created = token(&w.stage, &[SEND, RECEIVE, QUEUES]);
    let mut bot = socket(&w.stage, &created.token);
    let bots = object(w.channel, 1, &w.stage.bot);
    not_kept((db, at), &mut bot, &bots, "the bot's, outside the group");

    for epoch in [1, 2] {
        assert_none_given(&w, &alice_reads, epoch, "after the refusals");
    }
    let bobs = object(w.channel, 2, &w.stage.f.team.bob.laptop);
    keeps(at, &mut w.bob, &bobs, "bob's object of epoch 2");
    assert_given(&w, &alice_reads, 2, &bobs, "bob's object");
}

/// bob's laptop, still a member of epoch 1, is revoked: its object of epoch 1 is not kept. alice's
/// is then kept.
#[test]
fn objects_from_a_revoked_device_are_not_kept() {
    let mut w = World::new(5_418_200_011);
    let bob = &w.stage.f.team.bob;
    revoke(bob);

    let bobs = object(w.channel, 1, &bob.laptop);
    let at = (w.space, w.channel);
    not_kept(
        (w.stage.f.db(), at),
        &mut w.bob.socket,
        &bobs,
        "the revoked bob's",
    );

    let [.., carol, _, _] = w.laptops();
    assert_none_given(&w, &carol, 1, "after the revoked bob's upload");
    let alices = object(w.channel, 1, &w.stage.f.team.alice.laptop);
    keeps(at, &mut w.alice, &alices, "alice's object");
}

/// With alice's session but another key than her laptop's, a connection's `auth` naming her
/// laptop fails, and so does one of her device without a certificate; with the bot's token, one
/// naming the bot signed by another key: an object of epoch 1 uploaded on each is not kept.
#[test]
fn objects_on_a_connection_that_proves_no_device_are_not_kept() {
    let w = World::new(5_418_200_021);
    let (t, server) = (&w.stage.f.team, w.server());
    let (db, at) = (w.db(), (w.space, w.channel));
    let mut forged = Device::new("Forged");
    forged.id = t.alice.laptop.id;
    let created = token(&w.stage, &[SEND, RECEIVE, QUEUES]);
    let mut forged_bot = Device::new("Forged bot");
    forged_bot.id = w.stage.bot.id;

    for device in [&forged, &t.alice.uncertified] {
        let mut socket = Socket::to(server, &t.flow, &t.alice, device);
        let item = object(w.channel, 1, &t.alice.laptop);
        not_kept((db, at), &mut socket, &item, &device.name);
    }
    let mut socket = Socket::on_session(server, &t.flow, &created.token, &forged_bot);
    let item = object(w.channel, 1, &w.stage.bot);
    not_kept((db, at), &mut socket, &item, "the bot's token alone");

    let [.., carol, _, _] = w.laptops();
    assert_none_given(&w, &carol, 1, "after the unproved uploads");
}

/// alice's laptop uploads objects of epoch 2 and 5 while the channel is at epoch 1, refused with
/// `epoch-mismatch`; an object of another channel, and the channel's object uploaded naming
/// another channel, and bytes that do not decode as a queue key object, refused with `malformed`.
/// The server gives no object of epoch 1 or 2; alice's object of epoch 1 is then kept.
#[test]
fn objects_of_an_epoch_the_channel_has_not_had_or_of_another_channel_are_refused() {
    let mut w = World::new(5_418_200_031);
    let (alice, other) = (&w.stage.f.team.alice.laptop, ChannelId::generate());
    let (db, at) = (w.stage.f.db(), (w.space, w.channel));

    for epoch in [2, 5] {
        let ahead = object(w.channel, epoch, alice);
        let what = format!("an object of epoch {epoch}");
        refuses((db, at), &mut w.alice, &ahead, Some(EPOCH_MISMATCH), &what);
    }
    let malformed = Some(MALFORMED);
    let of_another = object(other, 1, alice);
    refuses(
        (db, at),
        &mut w.alice,
        &of_another,
        malformed,
        "of another channel",
    );
    let elsewhere = object(w.channel, 1, alice);
    let what = "the channel's object uploaded to another";
    refuses((db, (w.space, other)), &mut w.alice, &elsewhere, None, what);
    let garbage = Item::QueueKey(random().to_vec());
    refuses(
        (db, at),
        &mut w.alice,
        &garbage,
        malformed,
        "bytes that do not decode",
    );

    let [.., carol, _, _] = w.laptops();
    for epoch in [1, 2] {
        assert_none_given(&w, &carol, epoch, "after the refusals");
    }
    let alices = object(w.channel, 1, &w.stage.f.team.alice.laptop);
    keeps(at, &mut w.alice, &alices, "alice's object of epoch 1");
}
