//! A device linked with the recovery key of an account a channel added with `history` false shows
//! only the messages from the account's join on.

use skald_proto::Role;

use crate::devices::{BOB, adds_ada_without_history, said_by};
use crate::kit::received;
use crate::reading::{assert_history, sends, shown_history};
use crate::world::ACME;
use crate::{recovers, talk_with_key};

/// The lines said before a revision adds Ada's account to Bob's channel with `history` false.
const BEFORE_ADA: [&str; 2] = [
    "bob notes the first warehouse lease",
    "bob notes the renewed warehouse lease",
];

/// A revision Bob's device, an admin's, signs adds a channel with Bob alone, whose group his device
/// creates: epoch 1, where he says a line. His device commits an empty Commit: epoch 2, where he
/// says one more. A revision Bob's device signs adds Ada to the channel with `history` false, and
/// Bob's device adds the device of Ada's first machine: epoch 3, which its run joins from the
/// Welcome, and where Bob and Ada each say a line. That machine runs once more and is wiped. Her new
/// machine, linked with the recovery key naming that device lost, shows with `skald history` the
/// two lines of epoch 3, and neither line from before her account joined.
#[test]
fn recovered_device_of_a_member_without_history_shows_the_messages_from_its_join_on() {
    let (mut talk, key) = talk_with_key(
        &[4_177_705_001, 4_177_705_002],
        &["ada-recovers-without-past", "bob-keeps-the-lease"],
        Role::Admin,
    );
    let channel = adds_ada_without_history(&mut talk, BEFORE_ADA);
    let at = (talk.space.id, channel);
    let after = vec![
        said_by(
            &mut talk,
            BOB,
            (at, 3),
            "bob welcomes ada to the lease talks",
        )
        .0,
        sends(&talk.ada, (ACME, channel), "ada asks for the lease terms"),
    ];
    received(&talk.ada.machine);
    let first = talk.ada.device;

    let new = recovers(&talk, &key, &[first]);
    let shown = shown_history(&new.machine, channel);

    assert_history(&shown, channel, &after, &BEFORE_ADA);
}
