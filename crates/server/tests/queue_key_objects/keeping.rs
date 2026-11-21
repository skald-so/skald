//! The server keeps the first queue key object per channel and epoch from a member device of the
//! epoch and gives it, byte for byte, to the devices that may send in the channel; a later object
//! of the epoch changes nothing.

use crate::kit::World;
use crate::objects::{assert_given, assert_none_given, genuine, keeps, object};
use crate::tokens::SEND;

/// Before any upload, alice's laptop is told the server keeps no object of epoch 1. alice's laptop
/// uploads the object of epoch 1, with the `pk` of its history key of it: the server keeps it, and
/// gives it, byte for byte, to the laptops of alice, bob and carol and to the bot on a token with
/// `send` for `general`.
#[test]
fn a_member_devices_object_is_kept_and_given_byte_for_byte() {
    let mut w = World::new(5_418_100_001);
    let [alice, bob, carol, ..] = w.laptops();
    let bot = w.bot(&[SEND]);
    let (at, epoch) = ((w.space, w.channel), w.epoch());
    assert_none_given(&w, &alice, epoch, "before an upload");
    let item = genuine(w.channel, &w.alice, epoch, &w.stage.f.team.alice.laptop);

    keeps(at, &mut w.alice, &item, "alice's object of epoch 1");

    for client in [&alice, &bob, &carol, &bot] {
        assert_given(&w, client, epoch, &item, "alice's object");
    }
}

/// alice's laptop uploads an object of epoch 1, and bob's laptop another one of epoch 1: both get
/// `history-stored`, and the server gives alice's. alice's object again gets `history-stored` and
/// changes nothing. After alice's empty Commit, epoch 2, bob's object of epoch 2 is kept and given
/// for epoch 2, while epoch 1 keeps alice's.
#[test]
fn the_first_object_of_each_epoch_is_kept_and_a_later_one_changes_nothing() {
    let mut w = World::new(5_418_100_011);
    let [_, _, carol, ..] = w.laptops();
    let at = (w.space, w.channel);
    let t = &w.stage.f.team;
    let first = object(w.channel, 1, &t.alice.laptop);
    let later = object(w.channel, 1, &t.bob.laptop);
    let second = object(w.channel, 2, &t.bob.laptop);

    keeps(at, &mut w.alice, &first, "alice's object of epoch 1");
    keeps(at, &mut w.bob, &later, "bob's later object of epoch 1");
    keeps(at, &mut w.alice, &first, "alice's object again");

    assert_given(&w, &carol, 1, &first, "the first object of epoch 1");
    assert_eq!(w.rotate(), 2, "the epoch of alice's empty Commit");
    keeps(at, &mut w.bob, &second, "bob's object of epoch 2");
    assert_given(&w, &carol, 2, &second, "bob's object of epoch 2");
    assert_given(&w, &carol, 1, &first, "epoch 1 after epoch 2's upload");
}
