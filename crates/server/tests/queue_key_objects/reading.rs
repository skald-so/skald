//! Who reads a channel's queue key objects: devices that may send in the channel, member devices and
//! a service token's device with `send` for the channel, each proving its key; nobody else.

use crate::bot::ops;
use crate::kit::World;
use crate::objects::{assert_given, genuine, keeps, read};
use crate::presence::{assert_refused_with, assert_unproved};
use crate::tokens::{QUEUES, RECEIVE, SEND};

/// Before any upload, alice's laptop reading epoch 1 gets `404` with a problem details object, and
/// so does a read of epoch 7, which the channel has not had. After alice's upload of epoch 1,
/// epoch 7 still gets `404`.
#[test]
fn an_epoch_without_an_object_is_not_found() {
    let mut w = World::new(5_418_300_001);
    let [alice, ..] = w.laptops();
    let at = (w.space, w.channel);
    for epoch in [1, 7] {
        let response = read(&w, &alice, epoch, true);
        assert_eq!(response.status, 404, "epoch {epoch} before an upload");
        assert_refused_with(&response, None, "no object before an upload");
    }
    let item = genuine(w.channel, &w.alice, 1, &w.stage.f.team.alice.laptop);
    keeps(at, &mut w.alice, &item, "alice's object of epoch 1");

    assert_given(&w, &alice, 1, &item, "alice's object");
    let response = read(&w, &alice, 7, true);
    assert_eq!(response.status, 404, "epoch 7 after epoch 1's upload");
    assert_refused_with(&response, None, "no object of epoch 7");
}

/// alice's laptop uploads the object of epoch 1. Refused, with a problem details object and not
/// the object: dave's laptop, outside the channel; eve's, outside the space; the bot on a token
/// without `send`, and on one with `send` for another channel alone; and, with `401` and
/// `auth-required`, carol's session without her laptop's proof and the bot's sending token alone.
#[test]
fn others_and_a_session_or_token_without_the_proof_read_no_object() {
    let mut w = World::new(5_418_300_011);
    let other = ops(&mut w.stage);
    let [_, _, carol, dave, eve] = w.laptops();
    let at = (w.space, w.channel);
    let item = genuine(w.channel, &w.alice, 1, &w.stage.f.team.alice.laptop);
    keeps(at, &mut w.alice, &item, "alice's object of epoch 1");

    let other_only = w.bot_in(
        other,
        &[SEND, RECEIVE, QUEUES],
        "the bot in another channel",
    );
    let without_send = w.bot(&[RECEIVE, QUEUES]);
    for client in [&other_only, &without_send, &eve, &dave] {
        let response = read(&w, client, 1, true);
        assert_refused_with(&response, None, &client.who);
    }

    let sending = w.bot(&[SEND]);
    for client in [&carol, &sending] {
        assert_unproved(&read(&w, client, 1, false), &client.who);
    }
    assert_given(&w, &sending, 1, &item, "the sending bot with its proof");
}
