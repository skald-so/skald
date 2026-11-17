//! An epoch a device judged unclean is wrapped to no browser while it stays the current one.

use std::collections::BTreeSet;
use std::time::Duration;

use skald_proto::{Role, Roster};

use crate::chain::{adds_to_channel, carol, commits_empty};
use crate::cloud::WAIT;
use crate::fixtures::kill_once;
use crate::kit::{RECV, received, rosters};
use crate::talk::{Talk, commit_sent};
use crate::world::{ACME, revisions_request, serve_revisions};
use crate::wraps::{assert_wrapped, held_epochs, sent_to, valid_browser};

/// The test sets up a browser of Ada's account, and Ada's next run wraps epoch 1 to it. The relay
/// then keeps the revisions of the talk's space from Ada's machine as they were. A revision Bob's
/// device, an admin's, signs adds Carol to the channel, and Bob's device adds Carol's: epoch 2.
/// The relay holds the request for the channel's GroupInfo Ada's `skald recv` makes once it
/// entered epoch 2 and judged it unclean, and the test kills the run there, before it commits
/// Carol's Remove. With the revisions let through, Ada's next run accepts the one admitting Carol
/// and commits nothing, so epoch 2 stays the current one: no wrap of it is sent, and the server
/// holds none. After Bob's empty Commit, Ada's next run wraps the new epoch to the browser.
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
    adds_to_channel(cloud, &talk.played[0].person, at, &carol.person);
    talk.played[0].welcomes(cloud, at, &carol.person);
    let (sends, uploads) = (talk.front.sent().len(), talk.front.uploads().len());
    let group_info = format!("GET /spaces/{}/channels/{}/group-info", at.0, at.1);
    // Held until the next run asks too.
    cloud.relay.hold(&group_info, 2, WAIT);
    let started = talk.ada.machine.start(&[RECV]);
    kill_once(started, "a GroupInfo request after epoch 2", || {
        cloud.relay.held() > 0
    });
    cloud.relay.rewrite(&revisions_request(at.0), str::to_owned);
    // Requests for the GroupInfo go on at once from now on.
    cloud.relay.hold(&group_info, 0, Duration::ZERO);

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
