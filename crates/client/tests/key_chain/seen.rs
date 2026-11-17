//! Links the server has had: once the server listed a link to Ada's device or took it from it, her
//! device never uploads it again, so a link retention deletes stays deleted.

use std::collections::BTreeSet;

use crate::chain::{
    Pair, Sent, adds_to_channel, assert_links, assert_talk_links, bob_commits_empty, commits_empty,
    delete_links, links_sent, uploads_link,
};
use crate::front::Answers;
use crate::kit::received;
use crate::played::Played;
use crate::talk::{CAROL_HOST, Talk};

/// Bob's device commits an empty Commit, epoch 2, and uploads its link with `prev` 1 itself. A
/// revision Ada's device signs adds Carol to the channel; Ada's run enters epoch 2 and commits the
/// Add of Carol's device: epoch 3, whose link with `prev` 2 her machine uploads. So Ada's device
/// has seen both links, one listed by the server and one taken from it. The server's database
/// then deletes both, as retention does: Ada's next two runs upload neither, and the server holds
/// no link of the channel.
#[test]
fn link_the_server_has_had_is_not_uploaded_again_once_deleted() {
    let mut talk = Talk::new(
        &[4_164_009_001, 4_164_009_002],
        &["ada-sees-links", "bob-links-first"],
        false,
    );
    let at = talk.at();
    let cloud = &talk.world.cloud;
    let carol = Played::new(
        cloud,
        talk.world
            .sign_up(4_164_009_003, "carol-added-after", CAROL_HOST),
    );
    let bob = &mut talk.played[0];
    assert_eq!(
        commits_empty(cloud, bob, at),
        2,
        "Bob's empty Commit's epoch"
    );
    uploads_link(cloud, bob, at, (2, 1));
    adds_to_channel(cloud, &talk.ada, at, &carol.person);
    let from = talk.front.uploads().len();
    received(&talk.ada.machine);
    assert_links(cloud, &talk.ada.token, at, &[(2, 1), (3, 2)]);
    let sent = links_sent(&talk.front, at.1, from);
    assert!(
        sent.iter().any(|one| one.pair == (3, 2)),
        "Ada's machine sent no link of epoch 3: {sent:?}"
    );
    delete_links(cloud, at.1, &[2, 3]);
    assert_links(cloud, &talk.ada.token, at, &[]);
    let later = talk.front.uploads().len();

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    let sent = links_sent(&talk.front, at.1, later);
    assert!(sent.is_empty(), "links sent again: {sent:?}");
    assert_links(cloud, &talk.ada.token, at, &[]);
}

/// Bob's device commits an empty Commit: epoch 2. Ada's run finds the server lacks its link and
/// uploads it, so her device has seen it only as taken from it. The server's database then deletes
/// it, as retention does: Ada's next two runs do not upload it again.
#[test]
fn link_the_server_took_from_the_device_is_not_uploaded_again_once_deleted() {
    let mut talk = Talk::new(
        &[4_164_009_101, 4_164_009_102],
        &["ada-uploads-once", "bob-leaves-it-to-ada"],
        false,
    );
    bob_commits_empty(&mut talk, 2);
    received(&talk.ada.machine);
    assert_talk_links(&talk, &[(2, 1)]);
    delete_links(talk.cloud(), talk.channel, &[2]);
    let later = talk.front.uploads().len();

    for _ in 0..2 {
        received(&talk.ada.machine);
    }

    assert_eq!(links_sent(&talk.front, talk.channel, later), Vec::new());
    assert_talk_links(&talk, &[]);
}

/// Ada's next two runs send no link of the talk's channel, and the server holds none.
#[track_caller]
fn assert_none_sent_again(talk: &Talk) {
    let sent: Vec<Sent> = (0..2)
        .flat_map(|_| {
            let from = talk.front.uploads().len();
            received(&talk.ada.machine);
            links_sent(&talk.front, talk.channel, from)
        })
        .collect();
    assert!(sent.is_empty(), "links sent again: {sent:?}");
    assert_talk_links(talk, &[]);
}

/// Bob's device commits an empty Commit, epoch 2, and uploads its link with `prev` 1 itself. Ada's
/// run sends no link: the server lists it. The server's database then deletes it, as retention
/// does: Ada's next two runs do not upload it.
#[test]
fn link_another_device_uploaded_is_not_sent_again() {
    let mut talk = Talk::new(
        &[4_164_009_201, 4_164_009_202],
        &["ada-sees-bobs-link", "bob-uploads-own"],
        false,
    );
    let at = talk.at();
    bob_commits_empty(&mut talk, 2);
    uploads_link(talk.cloud(), &talk.played[0], at, (2, 1));
    let from = talk.front.uploads().len();

    received(&talk.ada.machine);

    assert_eq!(links_sent(&talk.front, at.1, from), Vec::new());
    assert_talk_links(&talk, &[(2, 1)]);
    delete_links(talk.cloud(), at.1, &[2]);
    assert_none_sent_again(&talk);
}

/// Bob's device commits two empty Commits: epochs 2 and 3. Ada's run uploads both links, with
/// `prev` 1 and 2. The server's database then deletes both, as retention does: Ada's next two runs
/// upload neither.
#[test]
fn links_uploaded_in_one_run_are_not_sent_again_once_deleted() {
    let mut talk = Talk::new(
        &[4_164_009_301, 4_164_009_302],
        &["ada-uploads-two", "bob-commits-twice"],
        false,
    );
    bob_commits_empty(&mut talk, 2);
    bob_commits_empty(&mut talk, 3);
    let from = talk.front.uploads().len();

    received(&talk.ada.machine);

    let sent = links_sent(&talk.front, talk.channel, from);
    let forwarded: BTreeSet<Pair> = (sent.iter())
        .filter(|one| one.forwarded)
        .map(|one| one.pair)
        .collect();
    assert_eq!(forwarded, BTreeSet::from([(2, 1), (3, 2)]), "{sent:?}");
    assert_talk_links(&talk, &[(2, 1), (3, 2)]);
    delete_links(talk.cloud(), talk.channel, &[2, 3]);
    assert_none_sent_again(&talk);
}

/// Bob's device commits an empty Commit: epoch 2. Ada's run uploads its link, and the front turns
/// the server's `history-stored` into `history-refused`: the run warns of the refusal. The server's
/// database then deletes the link, so the server lacks it: Ada's next two runs do not upload it.
#[test]
fn link_the_server_refused_is_not_sent_again() {
    let mut talk = Talk::new(
        &[4_164_009_401, 4_164_009_402],
        &["ada-link-refused", "bob-sees-refusal"],
        false,
    );
    bob_commits_empty(&mut talk, 2);
    talk.front
        .answer_history(Answers::Refuse("not-a-member".to_owned()));

    let ran = received(&talk.ada.machine);

    assert!(
        ran.stderr
            .contains("refused this device's link of the epoch 2"),
        "{ran:?}"
    );
    assert!(ran.stderr.contains("not-a-member"), "{ran:?}");
    talk.front.answer_history(Answers::Forward);
    delete_links(talk.cloud(), talk.channel, &[2]);
    assert_none_sent_again(&talk);
}
