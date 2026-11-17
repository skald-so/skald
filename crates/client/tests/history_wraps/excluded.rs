//! A browser its account's `excluded` lists in the latest revision a member device accepted gets no
//! wrap of a later epoch.

use std::collections::BTreeSet;

use skald_proto::NonEmpty;

use crate::chain::commits_empty;
use crate::kit::{latest, received};
use crate::talk::Talk;
use crate::world::{next_with, store};
use crate::wraps::{assert_wrapped, held_epochs, sent_to, valid_browser};

/// Two browsers of Ada's account get the wrap of epoch 1. A revision Ada's device signs then lists
/// one of them in Ada's `excluded`, and Ada's next `skald recv` accepts it. After Bob's empty
/// Commit, Ada's next run wraps the new epoch to the other browser alone: the server holds the
/// excluded browser's wrap of epoch 1 alone, and Ada's machine sent it no wrap since.
#[test]
fn browser_its_account_excludes_gets_no_wrap_of_a_later_epoch() {
    let mut talk = Talk::new(
        &[7_300_008_001, 7_300_008_002],
        &["ada-bars-a-browser", "bob-commits-after-exclusion"],
        false,
    );
    let cloud = &talk.world.cloud;
    let kept = valid_browser(cloud, &talk.ada, "ada-home-browser");
    let barred = valid_browser(cloud, &talk.ada, "ada-shared-browser");
    received(&talk.ada.machine);
    for browser in [&kept, &barred] {
        assert_wrapped(&mut talk, browser, 1);
    }
    let (cloud, ada) = (&talk.world.cloud, &talk.ada);
    let previous = latest(cloud, &ada.token, talk.space.id);
    let excluding = next_with(&previous, (ada.device, &ada.key), |body| {
        let listed = (body.members.clone().into_vec().into_iter())
            .map(|mut member| {
                if member.account == ada.account {
                    member.excluded = Some(NonEmpty::new(barred.id));
                }
                member
            })
            .collect();
        body.members = NonEmpty::from_vec(listed).expect("the members");
    });
    store(cloud, &ada.token, &excluding);
    received(&talk.ada.machine);
    let from = talk.front.uploads().len();
    let at = talk.at();

    let epoch = commits_empty(&talk.world.cloud, &mut talk.played[0], at);
    received(&talk.ada.machine);

    assert_wrapped(&mut talk, &kept, epoch);
    assert_eq!(
        held_epochs(&talk, barred.id),
        BTreeSet::from([1]),
        "the epochs wrapped to the excluded browser"
    );
    let sent = sent_to(&talk.front, talk.channel, from, barred.id);
    assert!(sent.is_empty(), "wraps to the excluded browser: {sent:?}");
}
