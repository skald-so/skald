//! Links the server has had: once the server listed a link to Ada's device or took it from it, her
//! device never uploads it again, so a link retention deletes stays deleted.

use crate::chain::{
    adds_to_channel, assert_links, assert_talk_links, bob_commits_empty, commits_empty,
    delete_links, links_sent, uploads_link,
};
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
