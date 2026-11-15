//! Commit ordering (`docs/protocol/membership.md`, "Commit ordering"): once the channel's group has
//! its first GroupInfo, the server accepts a Commit only of the current epoch, otherwise
//! `epoch-mismatch`, and only from the device of its sender leaf or, for an external Commit, its
//! joiner, otherwise `not-a-member`, as it is for a Commit it cannot process and a PublicMessage
//! proposal. A refused Commit leaves the epoch and the GroupInfo as they were. The refusals of
//! storing sent envelopes come first, and a retry gets the answer of the first send.

use skald_proto::{PROTOCOL_VERSION, Stored};

use crate::common::devices::random;
use crate::foreign;
use crate::scene::{
    EPOCH_MISMATCH, Scene, assert_latest, assert_uploaded, empty, latest_of, mls_header, payloads,
    upload,
};
use crate::wire::{Answer, NOT_A_MEMBER, VERSION_MISMATCH, now, version};

/// `commit` with one byte of its signature flipped: a member's Commit in a PublicMessage ends with
/// its signature, its confirmation tag and its membership tag, each an `opaque<V>`, of 64, 32 and 32
/// bytes.
fn with_broken_signature(commit: &[u8]) -> Vec<u8> {
    let len = commit.len();
    let prefixes = [commit[len - 131], commit[len - 66], commit[len - 33]];
    assert_eq!(
        prefixes,
        [64, 32, 32],
        "a Commit ending with its signature and tags"
    );
    let mut broken = commit.to_vec();
    broken[len - 67] ^= 1;
    broken
}

#[test]
fn a_commit_of_an_earlier_epoch_or_one_accepted_before_gets_epoch_mismatch_and_changes_nothing() {
    let (mut s, formed) = Scene::formed(3_121_470_101);
    let f = &s.fixture;
    let carols = (empty(&mut s.carol.copy(), f.general)).expect("carol's Commit of epoch 1");
    let bobs = s.bob.commits(f, empty);
    let second = s.bob.group_info(f);
    s.alice.process(f, &bobs);

    let cases = [
        (
            &mut s.carol,
            &carols,
            "a Commit of epoch 1, made before carol processed bob's",
        ),
        (
            &mut s.bob,
            &bobs,
            "bob's Commit, which the server accepted, again",
        ),
        (
            &mut s.alice,
            &formed.commit,
            "the channel's first Commit again",
        ),
    ];
    for (peer, commit, what) in cases {
        peer.assert_refused(f, commit, &[], EPOCH_MISMATCH, what);
        assert_latest(&s.dave.session, f, &second, what);
    }
    // bob's Commit went to alice and carol once.
    let db = f.db();
    assert_eq!(payloads(db, s.alice.id), std::slice::from_ref(&bobs));
    assert_eq!(
        payloads(db, s.carol.id),
        [formed.welcome.clone(), bobs.clone()]
    );
    assert_eq!(
        payloads(db, s.bob.id),
        std::slice::from_ref(&formed.welcome)
    );

    // The epoch is bob's: carol processes his Commit, and hers of epoch 2 is stored.
    assert_eq!(s.carol.process(f, &bobs), 2);
    let third = s.carol.commits(f, empty);
    assert_eq!(payloads(db, s.bob.id), [formed.welcome.clone(), third]);
}

#[test]
fn a_retried_commit_gets_the_stored_of_its_first_send() {
    let (mut s, _) = Scene::formed(3_121_470_111);
    let f = &s.fixture;
    let commit = empty(&mut s.bob.mls, f.general).expect("an empty Commit");
    let header = mls_header(f, &[]);
    let (envelope, first) = s.bob.send_with(&header, &commit);
    let Answer::Stored(first) = first else {
        panic!("stored, not {first:?}");
    };
    let (again, retried) = s.bob.send_with(&header, &commit);
    assert_eq!(again, envelope, "the same bytes");
    let Answer::Stored(Stored { send_key, .. }) = retried else {
        panic!("the retry's stored, not {retried:?}");
    };
    assert_eq!(send_key, first.send_key);
    for peer in [&s.alice, &s.carol] {
        let found = payloads(f.db(), peer.id);
        let held: Vec<_> = found.iter().filter(|payload| **payload == commit).collect();
        assert_eq!(held.len(), 1, "the Commit once in each member's mailbox");
    }
}

#[test]
fn a_commit_from_a_device_outside_the_group_gets_not_a_member() {
    let (mut s, formed) = Scene::formed(3_121_470_121);
    let f = &s.fixture;
    // dave, a member of the channel but not of its group, forms a group of his own with the
    // channel's ID: his Commit of epoch 1 names leaf 0, alice's in the channel's group.
    let mut fork = s.dave.twin();
    fork.create_group(f.general, now())
        .expect("a group of his own");
    empty(&mut fork, f.general).expect("a Commit of epoch 0");
    fork.commit_stored(f.general).expect("it merges");
    let forked = empty(&mut fork, f.general).expect("a Commit of epoch 1");

    s.dave
        .assert_refused(f, &forked, &[], NOT_A_MEMBER, "sent by dave");
    s.alice
        .assert_refused(f, &forked, &[], NOT_A_MEMBER, "sent by alice, of leaf 0");
    assert_latest(
        &s.bob.session,
        f,
        &formed.group_info,
        "after the fork's Commit",
    );
    s.bob.commits(f, empty);
}

#[test]
fn a_members_commit_sent_by_another_device_gets_not_a_member() {
    let (mut s, formed) = Scene::formed(3_121_470_131);
    let f = &s.fixture;
    let bobs = empty(&mut s.bob.mls, f.general).expect("an empty Commit");

    s.carol
        .assert_refused(f, &bobs, &[], NOT_A_MEMBER, "sent by carol, a member");
    s.dave.assert_refused(
        f,
        &bobs,
        &[],
        NOT_A_MEMBER,
        "sent by dave, outside the group",
    );
    assert_latest(&s.bob.session, f, &formed.group_info, "after the refusals");
    s.bob.stores(f, &bobs, &[]);
    assert_eq!(latest_of(&s.bob.session, f), (true, None), "a new epoch");
}

#[test]
fn an_external_commit_sent_by_another_device_than_its_joiner_gets_not_a_member() {
    let (mut s, formed) = Scene::formed(3_121_470_141);
    let f = &s.fixture;
    let dave = &f.team.dave.laptop;
    let (_, external) = foreign::joined(dave, &formed.group_info);

    s.carol
        .assert_refused(f, &external, &[], NOT_A_MEMBER, "sent by carol");
    s.alice
        .assert_refused(f, &external, &[], NOT_A_MEMBER, "sent by alice");
    assert_latest(&s.bob.session, f, &formed.group_info, "after the refusals");

    s.dave.stores(f, &external, &[]);
    // It goes to the members of epoch 1, who enter epoch 2 with dave.
    for peer in [&mut s.alice, &mut s.bob, &mut s.carol] {
        assert_eq!(payloads(f.db(), peer.id).last(), Some(&external));
        assert_eq!(peer.process(f, &external), 2);
        let members = (peer.mls.members(f.general)).expect("the group");
        assert!(
            members.iter().any(|member| member.device == dave.id),
            "dave in the group"
        );
    }
}

#[test]
fn a_broken_signature_bytes_that_are_no_mls_message_and_a_proposal_get_not_a_member() {
    let (mut s, formed) = Scene::formed(3_121_470_151);
    let f = &s.fixture;
    // dave joins by himself, on a client of the tests' own.
    let (mut daves, external) = foreign::joined(&f.team.dave.laptop, &formed.group_info);
    s.dave.stores(f, &external, &[]);
    for peer in [&mut s.alice, &mut s.bob, &mut s.carol] {
        peer.process(f, &external);
    }
    let group_info = s.bob.group_info(f);
    assert_uploaded(
        &upload(&s.bob.session, f, &group_info),
        "a member's of epoch 2",
    );

    let bobs = empty(&mut s.bob.mls, f.general).expect("an empty Commit");
    let cases = [
        (
            with_broken_signature(&bobs),
            "bob's Commit with a broken signature",
        ),
        (bobs[..bobs.len() - 1].to_vec(), "bob's Commit cut short"),
        (random().to_vec(), "bytes that are no MLS message"),
    ];
    for (payload, what) in cases {
        s.bob.assert_refused(f, &payload, &[], NOT_A_MEMBER, what);
    }
    let proposal = foreign::proposal(&mut daves);
    s.dave
        .assert_refused(f, &proposal, &[], NOT_A_MEMBER, "a member's proposal");
    assert_latest(&s.carol.session, f, &group_info, "after the refusals");

    s.bob.stores(f, &bobs, &[]);
}

#[test]
fn the_refusals_of_storing_sent_envelopes_come_first() {
    let (mut s, formed) = Scene::formed(3_121_470_161);
    let f = &s.fixture;
    let bobs = empty(&mut s.bob.mls, f.general).expect("an empty Commit");

    // eve, outside the space, sends the channel's first Commit again.
    let problem = s.eve.refusal(f, &formed.commit, &[]);
    assert_eq!(problem, NOT_A_MEMBER, "a sender outside the channel");
    let mut header = mls_header(f, &[]);
    header.v = version(PROTOCOL_VERSION.get() + 1);
    let (_, answer) = s.bob.send_with(&header, &bobs);
    let Answer::Refused(_, problem) = answer else {
        panic!("send-refused, not {answer:?}");
    };
    assert_eq!(problem, VERSION_MISMATCH, "a Commit of another version");
    assert_latest(
        &s.carol.session,
        f,
        &formed.group_info,
        "after the refusals",
    );

    s.bob.stores(f, &bobs, &[]);
}
