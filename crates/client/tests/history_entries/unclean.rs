//! A message sent in an epoch the device judged unclean leaves no entry: no copy under a key that a
//! device the members did not admit holds.

use std::time::Duration;

use skald_crypto::MlsFraming;
use skald_proto::{Role, Roster};

use crate::chain::{adds_to_channel, assert_none_sent_of, bob_commits_empty, carol};
use crate::cloud::WAIT;
use crate::entries::{ada_s_message, assert_ada_s_entry, entries_of, entries_sent, talk_entries};
use crate::fixtures::kill_once;
use crate::kit::{RECV, rosters};
use crate::talk::{Talk, commit_sent, messages_sent};
use crate::world::{ACME, revisions_request, serve_revisions};

/// The epoch of the `mls` message envelope `payload` holds.
fn epoch_of(payload: &[u8]) -> u64 {
    match MlsFraming::read(payload) {
        Some(MlsFraming::Message { epoch, .. }) => epoch,
        other => panic!("no MLS message: {other:?}"),
    }
}

/// From now on, the relay gives Ada's machine the revisions of the talk's space the server holds
/// now, and no later one.
fn hide_later_revisions(talk: &Talk) {
    let space = talk.space.id;
    let now: Vec<Roster> = rosters(talk.cloud(), &talk.ada.token, space);
    serve_revisions(talk.cloud(), space, &now.iter().collect::<Vec<&Roster>>());
}

/// The relay keeps the revisions of the talk's space from Ada's machine as they were, as a
/// compromised server would. A revision Bob's device, an admin's, signs adds Carol to the channel,
/// and Bob's device adds Carol's: epoch 2. The relay holds the next request for the channel's
/// GroupInfo, which Ada's `skald recv` makes once it entered epoch 2 and judged it unclean, as
/// it does not admit Carol's device; the test kills the run there, before it commits Carol's
/// Remove. The relay then lets the revisions through, and Ada's `skald send` accepts the one
/// admitting Carol and, committing nothing, sends its message in epoch 2: no link of epoch 2 is
/// sent, no entry of the message either, and the server holds none. Bob's device then commits an
/// empty Commit, epoch 3, which Ada's machine judges clean: her next message leaves the server's one
/// entry, under Bob's key of epoch 3.
#[test]
fn message_sent_in_an_epoch_judged_unclean_leaves_no_entry() {
    let mut talk = Talk::with(
        ACME,
        &[4_165_006_001, 4_165_006_002],
        &["ada-judges-in-the-dark", "bob-adds-carol-unseen"],
        false,
        (Role::Admin, |_| {}),
    );
    hide_later_revisions(&talk);
    let carol = carol(&talk, 4_165_006_003, "carol-admitted-unseen");
    let (cloud, at) = (&talk.world.cloud, talk.at());
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
    assert!(
        !commit_sent(&talk.front, at.1, sends),
        "Carol's device removed before the kill"
    );
    cloud.relay.rewrite(&revisions_request(at.0), str::to_owned);
    // Requests for the GroupInfo go on at once from now on.
    cloud.relay.hold(&group_info, 0, Duration::ZERO);

    let ran = talk.send("a line in an unclean epoch");

    assert!(ran.success, "{ran:?}");
    assert!(
        !commit_sent(&talk.front, at.1, sends),
        "a Commit sent in the channel: {ran:?}"
    );
    let unclean = ada_s_message(&talk, sends);
    let sent = messages_sent(&talk.front, at.1, sends);
    assert_eq!(
        epoch_of(&sent[0].envelope.envelope().payload),
        2,
        "the message not sent in epoch 2"
    );
    assert_none_sent_of(&talk.front, at.1, uploads, 2);
    let of_unclean = entries_of(&talk.front, at.1, uploads, unclean);
    assert!(
        of_unclean.is_empty(),
        "an entry of the message in epoch 2: {of_unclean:?}"
    );
    let held = talk_entries(&talk);
    assert!(held.is_empty(), "the server holds an entry: {held:?}");

    bob_commits_empty(&mut talk, 3);
    let from = talk.front.sent().len();
    let ran = talk.send("a line in a clean epoch");
    assert!(ran.success, "{ran:?}");
    let clean = ada_s_message(&talk, from);

    let held = talk_entries(&talk);
    assert_eq!(held.len(), 1, "not one entry on the server: {held:?}");
    assert_ada_s_entry(&mut talk, &held[0], (clean, 3));
    let all = entries_sent(&talk.front, talk.channel, uploads);
    assert!(
        all.iter().all(|one| one.msg_id == clean),
        "an entry of another message sent: {all:?}"
    );
}
