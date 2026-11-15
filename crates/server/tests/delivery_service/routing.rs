//! The mailboxes of a channel's `mls` envelopes (`docs/protocol/README.md`, "Envelope";
//! `docs/protocol/delivery.md`, "Send"): an application message goes to each member device of the
//! current epoch but its sender, which must be one, otherwise `not-a-member`; a Commit to each
//! member device of the epoch it commits from but its sender, so a removed device learns of its
//! removal; a Welcome to its `recipients`, each a member device of the current epoch, otherwise
//! `unknown-recipient`, as for a Welcome without `recipients` and any other `mls` envelope with
//! them; none to a revoked device.

use crate::scene::{Scene, empty, payloads, removing, revoke};
use crate::wire::{NOT_A_MEMBER, UNKNOWN_RECIPIENT, now};

#[test]
fn an_application_message_goes_to_each_member_device_of_the_current_epoch_but_its_sender() {
    let (mut s, formed) = Scene::formed(3_121_470_301);
    let f = &s.fixture;
    let message = s
        .alice
        .mls
        .encrypt(f.general, b"to the group")
        .expect("a message");
    s.alice.stores(f, &message, &[]);

    let db = f.db();
    for peer in [&mut s.bob, &mut s.carol] {
        assert_eq!(
            payloads(db, peer.id),
            [formed.welcome.clone(), message.clone()]
        );
        let read = peer
            .mls
            .decrypt(f.general, &message)
            .expect("a member reads it");
        assert_eq!(read.sender, s.alice.id);
    }
    // dave, a member of the channel but not of its group, gets none, nor does its sender.
    assert_eq!(payloads(db, s.dave.id), Vec::<Vec<u8>>::new());
    assert_eq!(payloads(db, s.alice.id), Vec::<Vec<u8>>::new());
    s.dave.assert_refused(
        f,
        &message,
        &[],
        NOT_A_MEMBER,
        "sent by a device outside the group",
    );
}

#[test]
fn a_message_sent_in_the_epoch_before_the_current_one_reaches_the_current_members() {
    let (mut s, _) = Scene::formed(3_121_470_311);
    let f = &s.fixture;
    let late = s
        .bob
        .mls
        .encrypt(f.general, b"of epoch 1")
        .expect("a message");
    let carol = s.carol.id;
    let removal = s.alice.commits(f, removing(&[carol]));

    s.bob.stores(f, &late, &[]);
    let db = f.db();
    assert_eq!(
        payloads(db, s.alice.id),
        std::slice::from_ref(&late),
        "alice, a member of epoch 2"
    );
    assert_eq!(
        payloads(db, carol).last(),
        Some(&removal),
        "nothing after the Remove"
    );
    let read = s
        .alice
        .mls
        .decrypt(f.general, &late)
        .expect("alice reads it");
    assert_eq!(read.sender, s.bob.id);
}

#[test]
fn a_remove_reaches_the_removed_device_whose_messages_are_then_refused() {
    let (mut s, formed) = Scene::formed(3_121_470_321);
    let f = &s.fixture;
    let before = s
        .carol
        .mls
        .encrypt(f.general, b"from epoch 1")
        .expect("a message");
    let carol = s.carol.id;
    let removal = s.alice.commits(f, removing(&[carol]));

    let db = f.db();
    for id in [s.bob.id, carol] {
        assert_eq!(
            payloads(db, id),
            [formed.welcome.clone(), removal.clone()],
            "the Remove"
        );
    }
    assert_eq!(payloads(db, s.alice.id), Vec::<Vec<u8>>::new());
    let processed = (s
        .carol
        .mls
        .process_commit(f.general, &removal, now(), |_| true))
    .expect("carol processes her Remove");
    assert_eq!(
        processed,
        skald_crypto::Processed::Removed,
        "carol learns of it"
    );

    s.carol.assert_refused(
        f,
        &before,
        &[],
        NOT_A_MEMBER,
        "the removed device's message",
    );
    assert_eq!(payloads(db, s.bob.id).len(), 2, "nothing for bob");
}

#[test]
fn a_revoked_devices_mailbox_gets_no_entry() {
    let (mut s, _) = Scene::formed(3_121_470_331);
    revoke(&s.fixture.team.carol);
    let f = &s.fixture;
    let db = f.db();

    let message = s
        .alice
        .mls
        .encrypt(f.general, b"after the revocation")
        .expect("a message");
    s.alice.stores(f, &message, &[]);
    let carol = s.carol.id;
    let removal = s.alice.commits(f, removing(&[carol]));
    let late = s
        .bob
        .mls
        .encrypt(f.general, b"of epoch 1, late")
        .expect("a message");
    s.bob.stores(f, &late, &[]);

    let bob = payloads(db, s.bob.id);
    assert_eq!(
        bob[bob.len() - 2..],
        [message.clone(), removal.clone()],
        "bob gets both"
    );
    let held = payloads(db, carol);
    for (payload, what) in [
        (message, "a message"),
        (removal, "her Remove"),
        (late, "bob's"),
    ] {
        assert!(!held.contains(&payload), "the revoked device got {what}");
    }
}

#[test]
fn unknown_recipient_for_a_welcome_without_recipients_or_outside_the_group_and_other_envelopes_with_them()
 {
    let (mut s, formed) = Scene::formed(3_121_470_341);
    let f = &s.fixture;
    let t = &f.team;
    // dave, a member of the channel, is outside the group; bob's other device has no certificate.
    let cases = [
        (vec![s.dave.id], "to dave, outside the group"),
        (vec![s.bob.id, s.dave.id], "to bob and dave"),
        (
            vec![t.bob.uncertified.id],
            "to bob's device without a certificate",
        ),
        (vec![s.eve.id], "to eve, outside the space"),
        (Vec::new(), "without recipients"),
    ];
    for (recipients, what) in &cases {
        s.alice
            .assert_refused(f, &formed.welcome, recipients, UNKNOWN_RECIPIENT, what);
    }
    let message = s
        .alice
        .mls
        .encrypt(f.general, b"to bob alone")
        .expect("a message");
    s.alice.assert_refused(
        f,
        &message,
        &[s.bob.id],
        UNKNOWN_RECIPIENT,
        "a message with them",
    );
    let commit = empty(&mut s.alice.copy(), f.general).expect("an empty Commit");
    s.alice.assert_refused(
        f,
        &commit,
        &[s.bob.id],
        UNKNOWN_RECIPIENT,
        "a Commit with them",
    );
    assert_eq!(
        payloads(f.db(), s.bob.id),
        std::slice::from_ref(&formed.welcome),
        "nothing for bob"
    );

    // Once alice adds dave, the Welcome goes to him alone, and the Commit to the members of the
    // epoch it commits from but alice.
    let (add, welcome) = s.add_dave();
    let f = &s.fixture;
    let db = f.db();
    assert_eq!(payloads(db, s.dave.id), [welcome]);
    for id in [s.bob.id, s.carol.id] {
        assert_eq!(payloads(db, id), [formed.welcome.clone(), add.clone()]);
    }
    assert_eq!(payloads(db, s.alice.id), Vec::<Vec<u8>>::new());
}
