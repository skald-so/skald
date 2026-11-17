//! Messages sent across several Commits, devices added and removed among them, show once each,
//! oldest first, with their channel, `from`, sending device, time and text: at a device in the
//! channel's group from its start, and at one that joined a group another device created. Text with
//! control characters shows escaped.

use crate::chain::{adds_to_channel, bob_commits_empty, carol, removes_from_channel, uploads_link};
use crate::joins::outside;
use crate::kit::received;
use crate::reading::{Said, assert_history, says, sends, shown_history};
use crate::talk::{Line, Talk};
use crate::world::ACME;

/// A text with control characters, and as the CLI shows it, escaped as `skald recv` escapes it.
const CONTROL: &str = "\u{1b}]0;pwned\u{7}\u{1b}[2Kfake history line";
const CONTROL_SHOWN: &str = r"\u{1b}]0;pwned\u{7}\u{1b}[2Kfake history line";

/// Ada's device creates the channel's group with Bob's: epoch 1, where Ada and Bob each say a line.
/// A revision Ada's device signs adds Carol, and Ada's run commits the Add of Carol's device: epoch
/// 2, where Bob and Ada each say one. Bob's device commits an empty Commit: epoch 3, where Ada says
/// one. A revision takes Carol out, and Ada's run commits the Remove of her device: epoch 4, where
/// Bob says one. Ada's `skald history` shows the six once each, oldest first.
#[test]
fn messages_across_commits_show_once_each_oldest_first_at_a_device_in_the_group_from_its_start() {
    let mut talk = Talk::new(
        &[4_166_001_001, 4_166_001_002],
        &["ada-reads-back", "bob-talks-back"],
        false,
    );
    let carol = carol(&talk, 4_166_001_003, "carol-passes-by");
    let (at, space) = (talk.at(), talk.space.name.clone());
    let from_bob = talk.address(&talk.played[0].person);
    let line = |counter, text| Line::new(&from_bob, counter, text);
    let cloud = &talk.world.cloud;
    let bob = &mut talk.played[0];
    let mut said: Vec<Said> = Vec::new();
    said.push(sends(&talk.ada, (&space, at.1), "ada opens the thread"));
    said.push(says(cloud, bob, at, &line(1, "bob answers in the first epoch")).0);
    adds_to_channel(cloud, &talk.ada, at, &carol.person);
    received(&talk.ada.machine);
    said.push(says(cloud, bob, at, &line(2, "bob writes once carol is in")).0);
    said.push(sends(
        &talk.ada,
        (&space, at.1),
        "ada replies with carol in",
    ));
    bob_commits_empty(&mut talk, 3);
    let cloud = &talk.world.cloud;
    said.push(sends(
        &talk.ada,
        (&space, at.1),
        "ada writes after the rotation",
    ));
    removes_from_channel(cloud, &talk.ada, at, &carol.person);
    received(&talk.ada.machine);
    let bob = &mut talk.played[0];
    said.push(
        says(
            cloud,
            bob,
            at,
            &line(3, "bob closes the thread without carol"),
        )
        .0,
    );
    received(&talk.ada.machine);

    let shown = shown_history(&talk.ada.machine, at.1);

    assert_history(&shown, at.1, &said, &[]);
}

/// A revision Bob's device, an admin's, signs adds a channel with Bob and Ada, whose group Bob's
/// device creates alone; Ada's run joins it by itself: epoch 2, where Bob and Ada each say a line.
/// A revision Bob's device signs adds Carol, and Bob's device adds hers: epoch 3, where Carol and
/// Ada each say one. A revision Bob's device signs takes Carol out, and Bob's device removes hers:
/// epoch 4, where Bob says one. Bob's device uploads the link of each epoch it makes. Ada's `skald
/// history` shows the five once each, oldest first.
#[test]
fn messages_across_commits_show_once_each_oldest_first_at_a_device_that_joined_the_group() {
    let (mut talk, channel) = outside(
        ACME,
        &[4_166_002_001, 4_166_002_002],
        &["ada-joins-and-reads", "bob-founds-channel"],
    );
    let mut carol = carol(&talk, 4_166_002_003, "carol-joins-briefly");
    let (at, space) = ((talk.space.id, channel), talk.space.name.clone());
    received(&talk.ada.machine);
    let from_bob = talk.address(&talk.played[0].person);
    let from_carol = talk.address(&carol.person);
    let cloud = &talk.world.cloud;
    let bob = &mut talk.played[0];
    let mut said: Vec<Said> = Vec::new();
    said.push(
        says(
            cloud,
            bob,
            at,
            &Line::new(&from_bob, 1, "bob greets the new channel"),
        )
        .0,
    );
    said.push(sends(
        &talk.ada,
        (&space, channel),
        "ada is in the new channel",
    ));
    adds_to_channel(cloud, &bob.person, at, &carol.person);
    bob.take_in(cloud);
    bob.welcomes(cloud, at, &carol.person);
    uploads_link(cloud, bob, at, (3, 2));
    let line = Line::new(&from_carol, 1, "carol says hello once added");
    said.push(says(cloud, &mut carol, at, &line).0);
    said.push(sends(&talk.ada, (&space, channel), "ada welcomes carol"));
    removes_from_channel(cloud, &bob.person, at, &carol.person);
    bob.take_in(cloud);
    bob.removes(cloud, at, carol.device());
    uploads_link(cloud, bob, at, (4, 3));
    let line = Line::new(&from_bob, 2, "bob carries on after carol left");
    said.push(says(cloud, bob, at, &line).0);
    received(&talk.ada.machine);

    let shown = shown_history(&talk.ada.machine, channel);

    assert_history(&shown, channel, &said, &[]);
}

/// Bob says a line with control characters. Ada's `skald history` shows it escaped, and its output
/// holds none of them.
#[test]
fn text_with_control_characters_shows_escaped() {
    let mut talk = Talk::new(
        &[4_166_003_001, 4_166_003_002],
        &["ada-reads-escaped", "bob-sends-escapes"],
        false,
    );
    let at = talk.at();
    let line = Line::new(&talk.address(&talk.played[0].person), 1, CONTROL);
    let (mut said, ..) = says(&talk.world.cloud, &mut talk.played[0], at, &line);
    said.text = CONTROL_SHOWN.to_owned();

    let shown = shown_history(&talk.ada.machine, at.1);

    assert_history(&shown, at.1, &[said], &[]);
    assert!(
        !shown.shown().contains(['\u{1b}', '\u{7}']),
        "control characters shown: {shown:?}"
    );
}
