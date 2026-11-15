//! Creating a channel's group (`docs/protocol/membership.md`, "Creating a channel"): the server
//! keeps the channel's first Commit of epoch 0 and refuses later ones with `epoch-mismatch`; it
//! takes the GroupInfo of epoch 1 only from that Commit's sender, and refuses the channel's other
//! `mls` envelopes with `not-a-member` until it has it. Each added device gets the Welcome.

use skald_proto::DeviceId;

use crate::common::devices::random;
use crate::scene::{
    EPOCH_MISMATCH, Scene, assert_latest, assert_upload_refused, assert_uploaded, latest_of,
    payloads, upload,
};
use crate::wire::{NOT_A_MEMBER, now};

#[test]
fn a_creators_commit_group_info_and_welcome_form_the_channels_group() {
    let mut s = Scene::new(3_121_470_001);
    let channel = s.fixture.general;
    assert_eq!(
        latest_of(&s.bob.session, &s.fixture),
        (false, None),
        "no group yet"
    );

    let commit = s.creates();
    let f = &s.fixture;
    s.alice.stores(f, &commit, &[]);
    let found = latest_of(&s.bob.session, f);
    assert_eq!(found, (true, None), "a group without a GroupInfo yet");

    let (welcome, group_info) = s.alice.merges_and_uploads(f);
    let welcome = welcome.expect("a Welcome");
    for peer in [&s.alice, &s.bob, &s.carol, &s.dave] {
        assert_latest(&peer.session, f, &group_info, "a channel member's");
    }
    s.alice.stores(f, &welcome, &[s.bob.id, s.carol.id]);

    // The first Commit goes to no one: its epoch had no member but its sender.
    for id in [s.alice.id, s.dave.id] {
        assert_eq!(payloads(f.db(), id), Vec::<Vec<u8>>::new());
    }
    for peer in [&mut s.bob, &mut s.carol] {
        peer.joins(f, &welcome);
        assert_eq!(peer.mls.epoch(channel).expect("the group"), 1);
    }
    s.assert_formed();

    // The members read each other.
    let message = s.bob.mls.encrypt(channel, b"hello").expect("a message");
    s.bob.stores(&s.fixture, &message, &[]);
    let read = (s.alice.mls.decrypt(channel, &message)).expect("alice reads it");
    assert_eq!((read.sender, read.plaintext), (s.bob.id, b"hello".to_vec()));
}

#[test]
fn of_two_creators_the_first_commit_is_kept_and_the_other_joins_its_group() {
    let mut s = Scene::new(3_121_470_011);
    // alice creates the group with bob and carol, then bob with alice and carol.
    let alices = s.creates();
    let packages = [s.alice.key_package(), s.carol.key_package()];
    let admitted = s.admitted();
    let f = &s.fixture;
    let bobs = s.bob.creates(f, &packages, &admitted);

    s.alice.stores(f, &alices, &[]);
    let problem = s.bob.refusal(f, &bobs, &[]);
    assert_eq!(problem, EPOCH_MISMATCH, "the later Commit of epoch 0");

    // bob's group of the refused Commit, a fork: its GroupInfo is refused, before alice's and
    // after it.
    let mut fork = s.bob.copy();
    fork.commit_stored(f.general).expect("the fork merges");
    let forked = fork.group_info(f.general).expect("the fork's GroupInfo");
    assert_upload_refused(&s.bob.session, f, &forked, "the fork's, first");
    let (welcome, group_info) = s.alice.merges_and_uploads(f);
    let welcome = welcome.expect("a Welcome");
    assert_upload_refused(&s.bob.session, f, &forked, "the fork's, then");
    assert_latest(&s.carol.session, f, &group_info, "the kept group's");

    // bob drops his group and joins the kept one, as carol does.
    (s.bob.mls.commit_refused(f.general)).expect("bob drops his group");
    s.alice.stores(f, &welcome, &[s.bob.id, s.carol.id]);
    for peer in [&mut s.bob, &mut s.carol] {
        peer.joins(f, &welcome);
    }
    s.assert_formed();
}

#[test]
fn before_the_first_group_info_the_channels_other_mls_envelopes_are_refused() {
    let mut s = Scene::new(3_121_470_021);
    let commit = s.creates();
    let f = &s.fixture;
    let channel = f.general;
    s.alice.stores(f, &commit, &[]);
    let welcome = s.alice.merges(f).expect("a Welcome");
    let group_info = s.alice.group_info(f);
    let message = (s.alice.mls.encrypt(channel, b"too early")).expect("a message");
    let mut later = s.alice.copy();
    let next_commit = (later.commit_empty(channel, b"", now())).expect("a Commit of epoch 1");
    let recipients = [s.bob.id, s.carol.id];

    let cases: [(&[u8], &[DeviceId], &str); 4] = [
        (&welcome, &recipients, "the Welcome"),
        (&message, &[], "an application message"),
        (&next_commit, &[], "a Commit of epoch 1"),
        (&random(), &[], "bytes that are no MLS message"),
    ];
    for (payload, to, what) in cases {
        s.alice.assert_refused(f, payload, to, NOT_A_MEMBER, what);
    }
    // Only the first Commit's sender gives the first GroupInfo.
    let what = "from another member's session";
    assert_upload_refused(&s.bob.session, f, &group_info, what);
    assert_eq!(latest_of(&s.carol.session, f), (true, None));
    for peer in [&s.alice, &s.bob, &s.carol, &s.dave] {
        assert_eq!(payloads(f.db(), peer.id), Vec::<Vec<u8>>::new());
    }

    assert_uploaded(&upload(&s.alice.session, f, &group_info), "the creator's");
    s.alice.stores(f, &welcome, &recipients);
    s.alice.stores(f, &message, &[]);
    for peer in [&s.bob, &s.carol] {
        let expected = [welcome.clone(), message.clone()];
        assert_eq!(payloads(f.db(), peer.id), expected);
    }
}
