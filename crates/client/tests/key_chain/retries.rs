//! A link that did not reach the server: with Ada's upload cut by a proxy, or her CLI killed after
//! sending its Commit, a later run uploads it, and only once.

use crate::chain::{assert_links, assert_opens, forwarded, joins_channel, joins_space, links_sent};
use crate::fixtures::kill_once;
use crate::front::Answers;
use crate::kit::{RECV, received};
use crate::played::Played;
use crate::talk::{CAROL_HOST, Talk, commit_sent};
use crate::world::run;

/// How Ada's run that adds Carol's device loses the link of the new epoch.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lost {
    /// The front closes her connection at the link, which the server never gets.
    Cut,
    /// The front loses each `stored`, and the test kills the run once it sent its Commit.
    Killed,
}

/// A revision Ada's device signs adds Carol to the space, and Ada's run adds Carol's device to the
/// default channel. Another adds Carol to the channel, and Ada's run commits the Add of Carol's
/// device, its only Commit, losing the link of epoch 2 as `lost` says. The next two runs, with the
/// front forwarding again, leave the link of epoch 2 with `prev` 1 on the server, which Carol's key
/// of epoch 2 opens to Bob's key of epoch 1; the front forwarded that link to the server once.
fn lost_link_is_uploaded_once(lost: Lost, ids: &[u64; 3], logins: &[&str; 3]) {
    let talk = Talk::new(&ids[..2], &logins[..2], true);
    let at = talk.at();
    let cloud = &talk.world.cloud;
    let mut carol = Played::new(cloud, talk.world.sign_up(ids[2], logins[2], CAROL_HOST));
    joins_space(cloud, &talk.ada, at.0, &carol.person);
    received(&talk.ada.machine);
    joins_channel(cloud, &talk.ada, at, &carol.person);
    let (sends, uploads) = (talk.front.sent().len(), talk.front.uploads().len());

    match lost {
        Lost::Cut => {
            talk.front.cut_next_link(at.1);
            // The run may fail, as its connection is closed.
            drop(run(&talk.ada.machine, &[RECV]));
            let sent = links_sent(&talk.front, at.1, uploads);
            assert!(
                sent.iter().any(|one| !one.forwarded && one.pair == (2, 1)),
                "no link of epoch 2 cut: {sent:?}"
            );
        }
        Lost::Killed => {
            talk.front.answer(Answers::Lose);
            let started = talk.ada.machine.start(&[RECV]);
            let front = &talk.front;
            kill_once(started, "a Commit of Ada's device", || {
                commit_sent(front, at.1, sends)
            });
            talk.front.answer(Answers::Forward);
        }
    }

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    let held = assert_links(cloud, &talk.ada.token, at, &[(2, 1)]);
    carol.take_in(cloud);
    assert_opens(&held, at.1, (2, 1), (&carol, &talk.played[0]));
    assert_eq!(
        forwarded(&talk.front, at.1, uploads, (2, 1)),
        1,
        "the link of epoch 2 not uploaded once: {:?}",
        links_sent(&talk.front, at.1, uploads)
    );
}

#[test]
fn link_whose_upload_a_proxy_cuts_is_uploaded_once_by_a_later_run() {
    lost_link_is_uploaded_once(
        Lost::Cut,
        &[4_164_008_001, 4_164_008_002, 4_164_008_003],
        &["ada-upload-cut", "bob-waits-link", "carol-added-cut"],
    );
}

#[test]
fn link_of_a_cli_killed_after_its_commit_is_uploaded_once_by_a_later_run() {
    lost_link_is_uploaded_once(
        Lost::Killed,
        &[4_164_008_101, 4_164_008_102, 4_164_008_103],
        &[
            "ada-killed-commit",
            "bob-waits-killed",
            "carol-added-killed",
        ],
    );
}
