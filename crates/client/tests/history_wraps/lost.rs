//! A wrap whose upload a proxy cuts, so the server never gets it, is uploaded by a later run, once.

use crate::chain::{adds_to_channel, bob_commits_empty, carol};
use crate::kit::{RECV, received};
use crate::talk::Talk;
use crate::world::run;
use crate::wraps::{assert_wrapped, sent_to, valid_browser};

/// How the new epoch whose wrap the front cuts comes about.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Epoch {
    /// Ada's run commits the Add of Carol's device and uploads the new epoch's wrap.
    AdasCommit,
    /// Bob's device commits an empty Commit, and Ada's run uploads the new epoch's wrap after its
    /// catch-up.
    BobsCommit,
}

/// The test sets up a browser of Ada's account, and Ada's next run wraps epoch 1 to it. Epoch 2
/// comes about as `epoch` says, and the front closes Ada's connection at the run's first wrap of
/// the channel, of epoch 2 to the browser, which the server never gets. Ada's next two runs leave
/// the wrap of epoch 2 on the server, signed by Ada's device, which the browser opens to the key
/// of epoch 2 Bob's device derives; the front forwarded a wrap of epoch 2 to the browser once.
fn cut_wrap_is_uploaded_once(epoch: Epoch, ids: &[u64; 3], logins: &[&str; 3]) {
    let mut talk = Talk::new(&ids[..2], &logins[..2], true);
    let browser = valid_browser(&talk.world.cloud, &talk.ada, "ada-browser-behind-proxy");
    received(&talk.ada.machine);
    assert_wrapped(&mut talk, &browser, 1);
    match epoch {
        Epoch::AdasCommit => {
            let carol = carol(&talk, ids[2], logins[2]);
            adds_to_channel(talk.cloud(), &talk.ada, talk.at(), &carol.person);
        }
        Epoch::BobsCommit => bob_commits_empty(&mut talk, 2),
    }
    let uploads = talk.front.uploads().len();
    talk.front.cut_next_wrap(talk.channel);
    // The run may fail, as its connection is closed.
    let cut = run(&talk.ada.machine, &[RECV]);
    let first = sent_to(&talk.front, talk.channel, uploads, browser.id)
        .first()
        .copied();
    assert!(
        first.is_some_and(|one| one.epoch == 2 && !one.forwarded),
        "no wrap of epoch 2 cut: {first:?}, {cut:?}"
    );

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    assert_wrapped(&mut talk, &browser, 2);
    let sent = sent_to(&talk.front, talk.channel, uploads, browser.id);
    let forwarded = (sent.iter())
        .filter(|one| one.epoch == 2 && one.forwarded)
        .count();
    assert_eq!(
        forwarded, 1,
        "the wrap of epoch 2 not uploaded once: {sent:?}"
    );
}

#[test]
fn wrap_of_a_commit_s_sender_cut_by_a_proxy_is_uploaded_once_by_a_later_run() {
    cut_wrap_is_uploaded_once(
        Epoch::AdasCommit,
        &[7_300_009_001, 7_300_009_002, 7_300_009_003],
        &[
            "ada-commit-wrap-cut",
            "bob-sees-cut-wrap",
            "carol-added-wrap-cut",
        ],
    );
}

#[test]
fn wrap_after_catch_up_cut_by_a_proxy_is_uploaded_once_by_a_later_run() {
    cut_wrap_is_uploaded_once(
        Epoch::BobsCommit,
        &[7_300_010_001, 7_300_010_002, 7_300_010_003],
        &[
            "ada-catch-up-wrap-cut",
            "bob-rotates-wrap-cut",
            "carol-unused-wrap-cut",
        ],
    );
}
