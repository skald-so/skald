//! A channel's first Commit and first GroupInfo (`docs/protocol/membership.md`, "Creating a
//! channel" and "Commit ordering"): only a Commit of epoch 0 starts the group, and only a GroupInfo
//! of epoch 1 starts the view the server follows; and a GroupInfo is taken only from an account
//! with a listed device that is not revoked ("MLS profile").

use crate::scene::{
    EPOCH_MISMATCH, Scene, assert_latest, assert_upload_refused, assert_uploaded, empty, latest_of,
    revoke, upload,
};

#[test]
fn a_commit_of_epoch_1_to_a_channel_without_a_group_gets_epoch_mismatch_and_starts_none() {
    let mut s = Scene::new(3_121_470_801);
    let first = s.creates();
    let f = &s.fixture;
    let mut ahead = s.alice.copy();
    ahead.commit_stored(f.general).expect("it merges");
    let later = empty(&mut ahead, f.general).expect("a Commit of epoch 1");

    s.alice
        .assert_refused(f, &later, &[], EPOCH_MISMATCH, "a Commit of epoch 1 first");
    assert_eq!(latest_of(&s.bob.session, f), (false, None), "no group");
    s.alice.stores(f, &first, &[]);
    assert_eq!(latest_of(&s.bob.session, f), (true, None), "the group");
}

#[test]
fn the_creators_first_group_info_of_epoch_2_is_refused_and_its_one_of_epoch_1_taken() {
    let mut s = Scene::new(3_121_470_811);
    let commit = s.creates();
    let f = &s.fixture;
    s.alice.stores(f, &commit, &[]);
    let welcome = s.alice.merges(f).expect("a Welcome");
    let first = s.alice.group_info(f);
    let mut ahead = s.alice.copy();
    empty(&mut ahead, f.general).expect("a Commit of epoch 1");
    ahead.commit_stored(f.general).expect("it merges");
    let second = ahead.group_info(f.general).expect("a GroupInfo of epoch 2");

    assert_upload_refused(&s.alice.session, f, &second, "the creator's of epoch 2");
    assert_eq!(latest_of(&s.bob.session, f), (true, None), "none taken");
    assert_uploaded(
        &upload(&s.alice.session, f, &first),
        "the creator's of epoch 1",
    );
    assert_latest(&s.bob.session, f, &first, "the creator's of epoch 1");

    // The server follows epoch 1: a member's Commit of it is stored.
    s.alice.stores(f, &welcome, &[s.bob.id, s.carol.id]);
    s.bob.joins(f, &welcome);
    s.bob.commits(f, empty);
}

#[test]
fn a_group_info_from_an_account_whose_only_listed_device_is_revoked_is_refused() {
    let (s, formed) = Scene::formed(3_121_470_821);
    revoke(&s.fixture.team.carol);
    let f = &s.fixture;
    let carols = s.carol.group_info(f);

    assert_upload_refused(
        &s.carol.session,
        f,
        &carols,
        "from carol, her laptop revoked",
    );
    assert_latest(&s.bob.session, f, &formed.group_info, "after the refusal");
    assert_uploaded(&upload(&s.bob.session, f, &carols), "the same from bob");
}
