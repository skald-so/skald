//! An epoch with a member device the members do not admit is unclean: no member uploads its link,
//! and the link of the epoch after the device's Remove skips it.

use crate::chain::{assert_chain, assert_none_sent_of, assert_talk_links, bob_opens};
use crate::joins::{intruder, intruder_joins};
use crate::kit::{received, warnings_naming};
use crate::talk::Talk;

/// A device the server lets in on Bob's account, its certificate chained to another master key,
/// joins the channel's group by itself: epoch 2. Ada's `skald recv` warns naming it and commits its
/// Remove: epoch 3 with Ada's and Bob's devices. The server holds the link of epoch 3 alone, with
/// `prev` 1, which Bob's key of epoch 3 opens to his key of epoch 1; Ada's machine sent no link of
/// epoch 2, nor one with it as `prev`, and its next run sends none either.
#[test]
fn device_the_server_joins_makes_its_epoch_unclean_and_the_next_link_skips_it() {
    let mut talk = Talk::new(
        &[4_164_003_001, 4_164_003_002],
        &["ada-skips-intruder", "bob-account-borrowed"],
        false,
    );
    let from = talk.front.uploads().len();
    let intruder = intruder(talk.cloud(), &talk.played[0].person, "bob-server-device");
    intruder_joins(talk.cloud(), &intruder, talk.at());

    let ran = received(&talk.ada.machine);

    assert!(
        !warnings_naming(&ran, intruder.id()).is_empty(),
        "no warning naming the joiner: {ran:?}"
    );
    let ada_and_bob = [talk.ada.device, talk.played[0].device()];
    let held = assert_chain(&talk, (3, &ada_and_bob), &[(3, 1)]);
    bob_opens(&mut talk, &held, (3, 1));
    received(&talk.ada.machine);
    assert_talk_links(&talk, &[(3, 1)]);
    assert_none_sent_of(&talk.front, talk.channel, from, 2);
}
