//! A device whose account a revision adds to a channel after messages were sent: with `history`
//! true, it shows them all, reaching their epochs through the links, whose keys never lie in the
//! clear on its disk; with `history` false, only those from its join on.

use std::fs;

use skald_proto::{ChannelId, Role};

use crate::chain::{key_of, uploads_link};
use crate::keys::holds;
use crate::kit::{Change, issue_channel, latest, received, revise};
use crate::reading::{Said, admits, assert_history, rotates, says, shown_history};
use crate::talk::{Line, Talk};
use crate::world::ACME;

/// The messages said before Ada's account joined the channel, and the one after.
struct Joined {
    talk: Talk,
    channel: ChannelId,
    before: Vec<Said>,
    after: Said,
}

/// A revision Bob's device, an admin's, signs adds a channel with Bob and Carol; Bob's device
/// creates its group and adds Carol's: epoch 2, where Bob and Carol each say a line. Bob's device
/// commits an empty Commit: epoch 3, where Carol and Bob each say one. A revision Bob's device signs
/// then adds Ada to the channel with `history`, and Bob's device adds Ada's: epoch 4, which Ada's
/// run joins from the Welcome, and where Bob says one more. Bob's device uploads the link of each
/// epoch it makes.
fn joins_after_them(history: bool, ids: &[u64; 3], logins: &[&str; 3]) -> Joined {
    let mut talk = Talk::with(ACME, ids, logins, false, (Role::Admin, |_| {}));
    let cloud = &talk.world.cloud;
    let space = talk.space.id;
    let from_bob = talk.address(&talk.played[0].person);
    let from_carol = talk.address(&talk.played[1].person);
    let (bob, carol) = talk.played.split_at_mut(1);
    let (bob, carol) = (&mut bob[0], &mut carol[0]);
    let channel = issue_channel(cloud, &bob.person.token, space);
    let at = (space, channel);
    let change = Change {
        channels: vec![(channel, vec![&bob.person, &carol.person])],
        ..Change::default()
    };
    revise(
        cloud,
        &latest(cloud, &bob.person.token, space),
        &bob.person,
        &change,
    );
    bob.creates_alone(cloud, at);
    bob.uploads_group_info(cloud, at);
    bob.welcomes(cloud, at, &carol.person);
    uploads_link(cloud, bob, at, (2, 1));
    let mut before = Vec::new();
    let line = Line::new(&from_bob, 1, "bob starts the private thread");
    before.push(says(cloud, bob, at, &line).0);
    let line = Line::new(&from_carol, 1, "carol answers in private");
    before.push(says(cloud, carol, at, &line).0);
    assert_eq!(
        rotates(cloud, bob, at),
        3,
        "the epoch of Bob's empty Commit"
    );
    let line = Line::new(&from_carol, 2, "carol writes after the rotation");
    before.push(says(cloud, carol, at, &line).0);
    let line = Line::new(&from_bob, 2, "bob writes before ada comes");
    before.push(says(cloud, bob, at, &line).0);
    admits(cloud, &bob.person, at, (&talk.ada, history));
    bob.take_in(cloud);
    bob.welcomes(cloud, at, &talk.ada);
    uploads_link(cloud, bob, at, (4, 3));
    received(&talk.ada.machine);
    let line = Line::new(&from_bob, 3, "bob welcomes ada to the thread");
    let after = says(cloud, bob, at, &line).0;
    Joined {
        talk,
        channel,
        before,
        after,
    }
}

/// With `history` true, Ada's `skald history` shows the five messages, those before her join
/// included, oldest first. No file of her machine holds Bob's keys of epochs 2 and 3, which she
/// reached only through the links, in any encoding.
#[test]
fn member_added_with_history_shows_the_messages_from_before_it_joined() {
    let joined = joins_after_them(
        true,
        &[4_166_004_001, 4_166_004_002, 4_166_004_003],
        &[
            "ada-added-with-past",
            "bob-adds-ada-late",
            "carol-talks-early",
        ],
    );
    let Joined {
        talk,
        channel,
        mut before,
        after,
    } = joined;
    before.push(after);

    let shown = shown_history(&talk.ada.machine, channel);

    assert_history(&shown, channel, &before, &[]);
    let bob = &talk.played[0];
    for epoch in [2, 3] {
        let key = key_of(bob, channel, epoch).to_bytes();
        let holding: Vec<_> = (talk.ada.machine.files().into_iter())
            .filter(|file| fs::read(file).is_ok_and(|bytes| holds(&bytes, &*key)))
            .collect();
        assert!(
            holding.is_empty(),
            "the key of epoch {epoch} in the clear: {holding:?}"
        );
    }
}

/// With `history` false, Ada's `skald history` shows only the message from her join on.
#[test]
fn member_added_without_history_shows_only_the_messages_from_its_join_on() {
    let joined = joins_after_them(
        false,
        &[4_166_005_001, 4_166_005_002, 4_166_005_003],
        &[
            "ada-added-no-past",
            "bob-adds-ada-bare",
            "carol-talks-unseen",
        ],
    );
    let earlier: Vec<&str> = joined
        .before
        .iter()
        .map(|said| said.text.as_str())
        .collect();

    let shown = shown_history(&joined.talk.ada.machine, joined.channel);

    assert_history(
        &shown,
        joined.channel,
        std::slice::from_ref(&joined.after),
        &earlier,
    );
}
