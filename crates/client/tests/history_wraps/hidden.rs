//! An epoch a device judged unclean is wrapped to no browser while it stays the current one.

use std::collections::BTreeSet;

use skald_proto::{Role, Roster};

use crate::chain::{carol, commits_empty};
use crate::entries::killed_in_unclean_epoch;
use crate::kit::{received, rosters};
use crate::talk::{Talk, commit_sent};
use crate::world::{ACME, serve_revisions};
use crate::wraps::{assert_wrapped, held_epochs, sent_to, valid_browser};

/// The test sets up a browser of Ada's account, and Ada's next run wraps epoch 1 to it. The relay
/// then keeps the revisions of the talk's space from Ada's machine as they were, and Ada's run is
/// killed in epoch 2, which it judged unclean, as `killed_in_unclean_epoch` says. With the
/// revisions let through, Ada's next run accepts the one admitting Carol and commits nothing, so
/// epoch 2 stays the current one: no wrap of it is sent, and the server holds none. After Bob's
/// empty Commit, Ada's next run wraps the new epoch to the browser.
#[test]
fn current_epoch_judged_unclean_is_wrapped_to_no_browser() {
    let mut talk = Talk::with(
        ACME,
        &[7_300_012_001, 7_300_012_002],
        &["ada-wraps-in-the-dark", "bob-adds-carol-hidden"],
        false,
        (Role::Admin, |_| {}),
    );
    let browser = valid_browser(&talk.world.cloud, &talk.ada, "ada-browser-in-the-dark");
    received(&talk.ada.machine);
    assert_wrapped(&mut talk, &browser, 1);
    let (cloud, at) = (&talk.world.cloud, talk.at());
    let now: Vec<Roster> = rosters(cloud, &talk.ada.token, at.0);
    serve_revisions(cloud, at.0, &now.iter().collect::<Vec<&Roster>>());
    let carol = carol(&talk, 7_300_012_003, "carol-admitted-hidden");
    let (sends, uploads) = killed_in_unclean_epoch(&mut talk, &carol);
    received(&talk.ada.machine);

    assert!(
        !commit_sent(&talk.front, at.1, sends),
        "a Commit of Ada's device sent in the channel"
    );
    assert_eq!(
        held_epochs(&talk, browser.id),
        BTreeSet::from([1]),
        "the epochs wrapped to the browser"
    );
    let sent = sent_to(&talk.front, at.1, uploads, browser.id);
    assert!(sent.is_empty(), "wraps sent: {sent:?}");
    let epoch = commits_empty(&talk.world.cloud, &mut talk.played[0], at);
    received(&talk.ada.machine);
    assert_wrapped(&mut talk, &browser, epoch);
}
