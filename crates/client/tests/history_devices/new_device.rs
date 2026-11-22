//! A member's new device, linked after messages were sent, shows them once a member device has run
//! since its join: from before a rotation and from before the device existed too. A new device of
//! an account whose `history` is false shows those from the epoch that added the account on.

use skald_proto::Role;

use crate::aged::{DAY, received_ahead, with_period};
use crate::chain::removes_from_channel;
use crate::devices::{BOB, CAROL, adds_ada_without_history, said_by, second_machine};
use crate::kit::{group, received};
use crate::reading::{assert_history, sends, shown_history};
use crate::talk::Talk;
use crate::world::ACME;

/// The host name of Ada's second machine, its certificate's `name`.
const LAPTOP: &str = "ada-laptop";

/// In epoch 1 of the talk's channel Carol, Bob and Ada each say a line. A revision Ada's device
/// signs sets the history period to a day, and Ada's run with her clock two days ahead rotates the
/// key: epoch 2, where Bob, Carol and Ada each say one. A revision Ada's device signs takes Carol
/// out of the channel, and Ada's run removes her device: epoch 3, where Bob and Ada each say one.
/// Then Ada's second machine is linked and joins the channel by itself: epoch 4; Ada's first
/// machine runs. The second machine's `skald history` shows the eight lines once each, oldest
/// first.
#[test]
fn new_device_shows_every_message_from_before_a_rotation_and_before_it_existed() {
    let mut talk = Talk::new(
        &[4_169_001_001, 4_169_001_002, 4_169_001_003],
        &[
            "ada-links-a-laptop",
            "bob-talks-throughout",
            "carol-leaves-midway",
        ],
        false,
    );
    let (at, space) = (talk.at(), talk.space.name.clone());
    let mut said = vec![
        said_by(&mut talk, CAROL, (at, 1), "carol opens the quarterly plan").0,
        said_by(&mut talk, BOB, (at, 1), "bob drafts the budget").0,
        sends(&talk.ada, (&space, at.1), "ada approves the draft budget"),
    ];
    with_period(&talk, DAY);
    let mut talk = received_ahead(talk, 2 * DAY);
    let (ada, bob, carol) = (
        talk.ada.device,
        talk.played[BOB].device(),
        talk.played[CAROL].device(),
    );
    group(talk.cloud(), &talk.ada.token, at.0, at.1).assert_is(2, &[ada, bob, carol]);
    said.push(
        said_by(
            &mut talk,
            BOB,
            (at, 2),
            "bob revises the budget after the rotation",
        )
        .0,
    );
    said.push(
        said_by(
            &mut talk,
            CAROL,
            (at, 2),
            "carol signs off on the revised plan",
        )
        .0,
    );
    said.push(sends(
        &talk.ada,
        (&space, at.1),
        "ada files the signed plan",
    ));
    removes_from_channel(talk.cloud(), &talk.ada, at, &talk.played[CAROL].person);
    received(&talk.ada.machine);
    group(talk.cloud(), &talk.ada.token, at.0, at.1).assert_is(3, &[ada, bob]);
    said.push(
        said_by(
            &mut talk,
            BOB,
            (at, 3),
            "bob closes the quarter without carol",
        )
        .0,
    );
    said.push(sends(
        &talk.ada,
        (&space, at.1),
        "ada thanks bob for the quarter",
    ));
    let laptop = second_machine(&talk.world, &talk.ada, LAPTOP);
    received(&laptop.machine);
    group(talk.cloud(), &talk.ada.token, at.0, at.1).assert_is(4, &[ada, bob, laptop.device]);
    received(&talk.ada.machine);

    let shown = shown_history(&laptop.machine, at.1);

    assert_history(&shown, at.1, &said, &[]);
}

/// The lines said before a revision adds Ada's account to Bob's channel with `history` false.
const BEFORE_ADA: [&str; 2] = [
    "bob notes the old supplier terms",
    "bob notes the renegotiated terms",
];

/// A revision Bob's device, an admin's, signs adds a channel with Bob alone, whose group his device
/// creates: epoch 1, where he says a line. His device commits an empty Commit: epoch 2, where he
/// says one more. A revision Bob's device signs adds Ada to the channel with `history` false, and
/// Bob's device adds Ada's: epoch 3, which Ada's run joins from the Welcome, and where Bob and Ada
/// each say a line. Then Ada's second machine is linked and joins the channel by itself: epoch 4;
/// Ada's first machine runs. The second machine's `skald history` shows the two lines of epoch 3,
/// before its own join, and neither line from before Ada's account joined.
#[test]
fn new_device_of_a_member_without_history_shows_the_messages_from_its_account_s_join_on() {
    let mut talk = Talk::with(
        ACME,
        &[4_169_002_001, 4_169_002_002],
        &["ada-joins-without-past", "bob-keeps-the-room"],
        false,
        (Role::Admin, |_| {}),
    );
    let channel = adds_ada_without_history(&mut talk, BEFORE_ADA);
    let space = talk.space.id;
    let at = (space, channel);
    let after = vec![
        said_by(
            &mut talk,
            BOB,
            (at, 3),
            "bob welcomes ada to the supplier talks",
        )
        .0,
        sends(&talk.ada, (ACME, channel), "ada asks for the current terms"),
    ];
    let laptop = second_machine(&talk.world, &talk.ada, LAPTOP);
    received(&laptop.machine);
    let devices = [talk.ada.device, talk.played[BOB].device(), laptop.device];
    group(talk.cloud(), &talk.ada.token, space, channel).assert_is(4, &devices);
    received(&talk.ada.machine);

    let shown = shown_history(&laptop.machine, channel);

    assert_history(&shown, channel, &after, &BEFORE_ADA);
}
