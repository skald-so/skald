//! A device judges each epoch once, when it enters it: a revision it accepts later leaves the
//! judgment, so a clean epoch stays the `prev` of the next link, and an unclean one never gets a
//! link.

use skald_proto::Role;

use crate::chain::{
    adds_to_channel, assert_chain, assert_none_sent_of, assert_talk_links, bob_commits_empty,
    bob_opens, carol,
};
use crate::edits::{Edit, ada_makes};
use crate::kit::{received, rosters};
use crate::talk::Talk;
use crate::world::{ACME, MEMBER, assert_role, members, revisions_request, serve_revisions};

/// Bob's device commits an empty Commit: epoch 2, with the devices of Ada, Bob and Carol, which
/// Ada's run judges clean and links to epoch 1. Ada's `skald exclude-device` then bars Carol's
/// device, and Ada's device commits its Remove: epoch 3. The link of epoch 3 has `prev` 2, though
/// the latest revision no longer admits a device of epoch 2: Bob's key of epoch 3 opens it to his
/// key of epoch 2.
#[test]
fn epoch_judged_clean_stays_clean_once_a_revision_excludes_one_of_its_devices() {
    let mut talk = Talk::new(
        &[4_164_006_001, 4_164_006_002, 4_164_006_003],
        &[
            "ada-keeps-judgment",
            "bob-rotates-early",
            "carol-barred-later",
        ],
        false,
    );
    bob_commits_empty(&mut talk, 2);
    received(&talk.ada.machine);
    assert_talk_links(&talk, &[(2, 1)]);

    ada_makes(&talk, Edit::Exclude(&talk.played[1].person));

    let ada_and_bob = [talk.ada.device, talk.played[0].device()];
    let held = assert_chain(&talk, (3, &ada_and_bob), &[(2, 1), (3, 2)]);
    bob_opens(&mut talk, &held, (3, 2));
}

/// The relay keeps the revisions of the talk's space from Ada's machine as they were, as a
/// compromised server would. A revision Bob's device, an admin's, signs adds Carol to the channel,
/// and Bob's device adds Carol's: epoch 2. Ada's `skald recv` does not admit Carol's device, so it
/// judges epoch 2 unclean and commits its Remove: epoch 3, linked to epoch 1. The relay then lets
/// the revisions through, and Ada's machine accepts the one admitting Carol: its later runs upload
/// no link of epoch 2, nor one with it as `prev`, while the link of Bob's next Commit, epoch 4,
/// has `prev` 3.
#[test]
fn epoch_entered_while_a_proxy_hid_its_device_s_admission_stays_unclean() {
    let mut talk = Talk::with(
        ACME,
        &[4_164_007_001, 4_164_007_002],
        &["ada-kept-in-the-dark", "bob-adds-carol"],
        false,
        (Role::Admin, |_| {}),
    );
    let carol = carol(&talk, 4_164_007_003, "carol-admitted-late");
    let (cloud, at) = (&talk.world.cloud, talk.at());
    let known = rosters(cloud, &talk.ada.token, at.0);
    serve_revisions(cloud, at.0, &known.iter().collect::<Vec<_>>());
    adds_to_channel(cloud, &talk.played[0].person, at, &carol.person);
    talk.played[0].welcomes(cloud, at, &carol.person);
    let from = talk.front.uploads().len();

    received(&talk.ada.machine);

    let ada_and_bob = [talk.ada.device, talk.played[0].device()];
    assert_chain(&talk, (3, &ada_and_bob), &[(3, 1)]);
    cloud.relay.rewrite(&revisions_request(at.0), str::to_owned);

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    let listed = members(&talk.ada.machine, ACME);
    assert_role(&listed, &carol.person.handle, MEMBER);
    assert_talk_links(&talk, &[(3, 1)]);
    bob_commits_empty(&mut talk, 4);
    received(&talk.ada.machine);
    let held = assert_talk_links(&talk, &[(3, 1), (4, 3)]);
    assert_none_sent_of(&talk.front, talk.channel, from, 2);
    bob_opens(&mut talk, &held, (4, 3));
}
