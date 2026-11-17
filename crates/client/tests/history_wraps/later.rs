//! A browser linked later gets a wrap of the current epoch at a member device's next run, none of
//! earlier epochs, and reads them through the key chain.

use std::collections::{BTreeMap, BTreeSet};

use crate::chain::bob_commits_empty;
use crate::entries::ada_s_message;
use crate::kit::received;
use crate::talk::Talk;
use crate::wraps::{assert_reads, held_epochs, sent_to, valid_browser};

/// Ada's `skald send` sends a message at epoch 1; Bob's device commits an empty Commit, epoch 2,
/// and Ada sends another message at epoch 2. Only then does the test set up a browser of Ada's
/// account and one of Bob's. Ada's next `skald recv` uploads to each a wrap of epoch 2 and none of
/// epoch 1: the server holds only the wrap of epoch 2 to each. Each browser opens it, signed by
/// Ada's device, to the key of epoch 2 Bob's device derives, and reads both of Ada's entries
/// through the link of epoch 2.
#[test]
fn browser_set_up_later_gets_the_current_epoch_s_wrap_at_the_next_run_and_none_earlier() {
    let mut talk = Talk::new(
        &[7_300_002_001, 7_300_002_002],
        &["ada-wraps-latecomers", "bob-links-late"],
        false,
    );
    let from = talk.front.sent().len();
    let sent = talk.send("a line of the first epoch");
    assert!(sent.success, "{sent:?}");
    let first = ada_s_message(&talk, from);
    bob_commits_empty(&mut talk, 2);
    let from = talk.front.sent().len();
    let sent = talk.send("a line of the second epoch");
    assert!(sent.success, "{sent:?}");
    let second = ada_s_message(&talk, from);
    let cloud = &talk.world.cloud;
    let browsers = [
        valid_browser(cloud, &talk.ada, "ada-late-browser"),
        valid_browser(cloud, &talk.played[0].person, "bob-late-browser"),
    ];
    let uploads = talk.front.uploads().len();

    received(&talk.ada.machine);

    let read = BTreeMap::from([(1, vec![first]), (2, vec![second])]);
    for browser in &browsers {
        assert_eq!(
            held_epochs(&talk, browser.id),
            BTreeSet::from([2]),
            "the epochs wrapped to {}",
            browser.id
        );
        let sent = sent_to(&talk.front, talk.channel, uploads, browser.id);
        assert!(
            sent.iter().all(|one| one.epoch == 2),
            "a wrap of an earlier epoch sent to {}: {sent:?}",
            browser.id
        );
        assert_reads(&mut talk, (browser, 2), &read);
    }
}
