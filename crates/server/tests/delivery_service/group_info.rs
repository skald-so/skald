//! The channel's GroupInfo on the server (`docs/protocol/membership.md`, "MLS profile"): it keeps
//! an upload of the current epoch from a session of an account with a member device that the
//! GroupInfo lists, with `ratchet_tree` and `external_pub`, that verifies and has the GroupContext
//! of the group it follows, and refuses any other. It gives it only while it is of the current
//! epoch, and only to sessions of accounts the latest revision lists as the channel's members.

use crate::api::{Session, assert_refused};
use crate::fixture::channel_members;
use crate::foreign;
use crate::scene::{
    Scene, assert_latest, assert_upload_refused, assert_uploaded, empty, latest, latest_of,
    removing, upload,
};
use crate::wire::now;

#[test]
fn a_group_info_of_an_earlier_epoch_or_without_either_extension_is_refused() {
    let (mut s, formed) = Scene::formed(3_121_470_201);
    let f = &s.fixture;
    // dave joins by himself, on a client of the tests' own; the server has no GroupInfo of
    // epoch 2 until one is uploaded.
    let (daves, external) = foreign::joined(&f.team.dave.laptop, &formed.group_info);
    s.dave.stores(f, &external, &[]);
    assert_eq!(
        latest_of(&s.carol.session, f),
        (true, None),
        "after the Commit"
    );

    let cases = [
        (
            &s.alice.session,
            formed.group_info.clone(),
            "alice's of epoch 1",
        ),
        (
            &s.dave.session,
            foreign::group_info(&daves, true, false),
            "dave's without external_pub",
        ),
        (
            &s.dave.session,
            foreign::group_info(&daves, false, true),
            "dave's without ratchet_tree",
        ),
        (
            &s.dave.session,
            foreign::group_info(&daves, false, false),
            "dave's without either",
        ),
    ];
    for (session, group_info, what) in &cases {
        assert_upload_refused(session, f, group_info, what);
        assert_eq!(latest_of(&s.carol.session, f), (true, None), "{what}");
    }

    let whole = foreign::group_info(&daves, true, true);
    assert_uploaded(&upload(&s.dave.session, f, &whole), "dave's with both");
    assert_latest(&s.carol.session, f, &whole, "dave's");
}

#[test]
fn a_group_info_not_listing_its_uploader_or_from_a_removed_device_is_refused() {
    let (mut s, formed) = Scene::formed(3_121_470_211);
    let f = &s.fixture;
    let carol = s.carol.id;
    let removal = s.alice.commits(f, removing(&[carol]));
    let second = s.alice.group_info(f);
    s.bob.process(f, &removal);
    let removed = (s
        .carol
        .mls
        .process_commit(f.general, &removal, now(), |_| true))
    .expect("carol processes her Remove");
    assert_eq!(removed, skald_crypto::Processed::Removed);

    // carol's account is still a member of the channel, so her session gets the GroupInfo; her
    // device is no member of the group, and dave's never was.
    assert_latest(&s.carol.session, f, &second, "the removed device's account");
    assert_upload_refused(&s.carol.session, f, &second, "from the removed device");
    assert_upload_refused(
        &s.carol.session,
        f,
        &formed.group_info,
        "the removed device's of epoch 1",
    );
    assert_upload_refused(&s.dave.session, f, &second, "not listing its uploader");
    assert_upload_refused(&s.eve.session, f, &second, "from outside the space");
    assert_latest(&s.bob.session, f, &second, "after the refusals");
}

#[test]
fn a_group_info_of_a_fork_is_refused() {
    let (s, formed) = Scene::formed(3_121_470_221);
    let f = &s.fixture;
    // carol, a member, forms a group of her own with the channel's ID, at epoch 1 too: signed by
    // her, listing her, with both extensions, but of another GroupContext.
    let mut fork = s.carol.twin();
    fork.create_group(f.general, now())
        .expect("a group of her own");
    empty(&mut fork, f.general).expect("a Commit of epoch 0");
    fork.commit_stored(f.general).expect("it merges");
    let forked = fork.group_info(f.general).expect("the fork's GroupInfo");

    assert_upload_refused(&s.carol.session, f, &forked, "a fork's");
    assert_latest(&s.bob.session, f, &formed.group_info, "after the fork's");
    // A member's of the channel's group is taken from carol.
    let carols = s.carol.group_info(f);
    assert_uploaded(&upload(&s.carol.session, f, &carols), "carol's");
    assert_latest(&s.bob.session, f, &carols, "carol's");
}

#[test]
fn after_a_commit_none_is_given_until_a_member_uploads_one_of_the_new_epoch() {
    let (mut s, _) = Scene::formed(3_121_470_231);
    let f = &s.fixture;
    let bobs_first = s.bob.group_info(f);
    let commit = empty(&mut s.bob.mls, f.general).expect("an empty Commit");
    s.bob.stores(f, &commit, &[]);
    s.bob.mls.commit_stored(f.general).expect("it merges");
    assert_eq!(
        latest_of(&s.alice.session, f),
        (true, None),
        "after the Commit"
    );
    assert_upload_refused(&s.bob.session, f, &bobs_first, "bob's of the epoch before");
    assert_eq!(
        latest_of(&s.alice.session, f),
        (true, None),
        "after the refusal"
    );

    // Any member of the new epoch uploads one; the server keeps the latest.
    s.carol.process(f, &commit);
    let carols = s.carol.group_info(f);
    assert_uploaded(&upload(&s.carol.session, f, &carols), "carol's");
    assert_latest(&s.alice.session, f, &carols, "carol's");
    let bobs = s.bob.group_info(f);
    assert_uploaded(&upload(&s.bob.session, f, &bobs), "bob's");
    assert_latest(&s.alice.session, f, &bobs, "bob's, the latest");
}

#[test]
fn the_group_info_is_given_only_to_sessions_of_the_channels_members() {
    let (mut s, formed) = Scene::formed(3_121_470_241);
    for peer in [&s.alice, &s.bob, &s.carol, &s.dave] {
        assert_latest(
            &peer.session,
            &s.fixture,
            &formed.group_info,
            "a channel member's",
        );
    }
    assert_refused(
        &latest(&s.eve.session, &s.fixture),
        "eve, outside the space",
    );
    let none = latest(&Session::none(&s.fixture.team.flow), &s.fixture);
    assert_eq!(none.status, 401, "without a session");

    // A revision without carol in the channel.
    let t = &s.fixture.team;
    let accounts = [t.alice.account, t.bob.account, t.dave.account];
    let general = s.fixture.general;
    s.fixture
        .revise(|body| channel_members(body, general, &accounts));
    assert_refused(
        &latest(&s.carol.session, &s.fixture),
        "carol, out of the channel",
    );
    assert_latest(&s.dave.session, &s.fixture, &formed.group_info, "dave's");
}
