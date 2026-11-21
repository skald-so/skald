//! A deleted account's devices are served as revoked ones: their mailbox entries expire, their
//! senders getting `expired` for those sent with `report` true; no later envelope adds them an
//! entry; their KeyPackages are deleted and handed out no more; the server takes no upload from
//! them and no wrap to the account's recovery key or browsers.

use std::thread;

use skald_crypto::{HistoryKey, HpkeKey, SigningKey};
use skald_proto::{Cbor, ChannelId, DeviceId, DeviceKind, ServerStatus, WrapRecipient};

use crate::api::Person;
use crate::common::devices::{Device, assert_taken, random};
use crate::deletion::deletes;
use crate::fixture::Fixture;
use crate::history::{
    Answer, Item, assert_refused_upload, attempt, bob_rotates, formed, stores, upload, wrap,
};
use crate::link::{
    Entry, Link, PUSH, QUIET, Sent, assert_status, await_seqs, await_seqs_within, seqs, store_to,
};
use crate::packages::{fresh, hand_out, hand_out_one, last_resort, mls, upload as upload_packages};
use crate::sessions::Api;
use crate::wire::{Socket, UNKNOWN_RECIPIENT, assert_nowhere, envelope, now};

/// Checks that `entries` are the `expired` of each message of `sent` to `recipient`, in any order,
/// each issued at `before` or later.
#[track_caller]
fn assert_expired_for(entries: &[Entry], sent: &[Sent], recipient: DeviceId, before: u64) {
    assert_eq!(entries.len(), sent.len(), "{entries:?}");
    for (header, _) in sent {
        let found = entries.iter().find(
            |entry| matches!(entry, Entry::Status(_, status) if status.msg_id == header.msg_id),
        );
        let entry =
            found.unwrap_or_else(|| panic!("no status of {:?} in {entries:?}", header.msg_id));
        assert_status(entry, ServerStatus::Expired, header, recipient, before);
    }
}

/// alice, connected to another instance than the one that deletes the account, sends carol's
/// laptop, offline, two envelopes with `report` true and one without. Once carol deletes her
/// account, alice gets `expired` for the two and nothing for the third, and the laptop's mailbox is
/// empty.
#[test]
fn deleted_accounts_entries_expire_and_their_senders_get_expired_for_report_true_only() {
    let f = Fixture::new(3_303_040_001);
    let t = &f.team;
    let laptop = t.carol.laptop.id;
    let (_other, mut alice) = Link::elsewhere(&t.flow, &t.alice);
    let reported = store_to(&mut alice, &f, laptop, 2, true);
    let unreported = store_to(&mut alice, &f, laptop, 1, false);
    assert_eq!(unreported.len(), 1);
    await_seqs(f.db(), laptop, &[1, 2, 3]);

    let before = now();
    deletes(&Api::of(&t.flow), &t.carol);

    assert_expired_for(&alice.entries(2), &reported, laptop, before);
    alice.assert_quiet();
    await_seqs_within(f.db(), laptop, &[], PUSH);
}

/// carol's laptop sends alice's laptop an envelope with `report` true, then carol deletes her
/// account. Neither alice's ack of the envelope nor an envelope to carol's laptop, which is refused
/// with `unknown-recipient` and stored nowhere, adds an entry to its mailbox.
#[test]
fn no_later_entry_is_added_to_a_deleted_accounts_device() {
    let f = Fixture::new(3_303_040_011);
    let t = &f.team;
    let db = f.db();
    let laptop = t.carol.laptop.id;
    let alices = t.alice.laptop.id;
    let to_alice = f.to(alices);
    let sent = envelope(&to_alice);
    Link::synced(&t.flow, &t.carol, 0, &[]).store(&to_alice, &sent, true);

    deletes(&Api::of(&t.flow), &t.carol);

    let mut reader = Link::synced(&t.flow, &t.alice, 0, &[Entry::Message(1, sent)]);
    reader.ack(1);
    await_seqs(db, alices, &[]);
    let to_carol = f.to(laptop);
    let refused = envelope(&to_carol);
    let answer = Socket::of(&t.flow, &t.alice).refusal(to_carol.msg_id, &refused);
    assert_eq!(answer.1, UNKNOWN_RECIPIENT, "an envelope to carol's laptop");
    assert_nowhere(db, &refused);
    thread::sleep(QUIET);
    assert_eq!(
        seqs(db, laptop),
        Vec::<i64>::new(),
        "carol's laptop's mailbox"
    );
}

/// carol's laptop has two KeyPackages and a last-resort one; bob, who shares the space, gets one.
/// Once carol deletes her account, the database holds none of the others, bob gets none, and her
/// session's upload of more is refused.
#[test]
fn deleted_accounts_key_packages_are_not_handed_out_and_its_uploads_are_refused() {
    let f = Fixture::new(3_303_040_021);
    let t = &f.team;
    let laptop = t.carol.laptop.id;
    let mut device = mls(&t.carol.laptop);
    let others = fresh(&mut device, 2);
    let last = last_resort(&mut device);
    assert_taken(&upload_packages(
        &t.carol.session,
        laptop,
        &others,
        Some(&last),
    ));
    let handed = hand_out_one(&t.bob.session, laptop).expect("a KeyPackage before the deletion");
    assert!(others.contains(&handed), "one of carol's KeyPackages");

    deletes(&Api::of(&t.flow), &t.carol);

    let rows = f.db().rows().concat();
    for bytes in others.iter().chain([&last]) {
        let hexed = crate::common::hex(bytes);
        assert!(
            !rows.contains(&hexed),
            "the database holds a KeyPackage of carol's laptop"
        );
    }
    assert_eq!(hand_out(&t.bob.session, &[laptop]).get(&laptop), None);
    let more = fresh(&mut device, 1);
    let response = upload_packages(&t.carol.session, laptop, &more, None);
    crate::deletion::assert_refused(&response, "KeyPackages from carol's session");
    assert_eq!(hand_out_one(&t.bob.session, laptop), None);
}

/// A wrap of a key of its own of `channel`'s epoch `epoch` to the browser `browser`, signed by
/// `signer`'s laptop.
fn browser_wrap(channel: ChannelId, epoch: u64, browser: &Device, signer: &Person) -> Item {
    let key = HistoryKey::from_bytes(&random()[..16].try_into().expect("16 bytes"));
    let to = WrapRecipient::Browser(browser.id);
    let signing = SigningKey::from_bytes(&signer.laptop.signing.secret);
    let sealed = key.wrap(
        channel,
        epoch,
        to,
        &HpkeKey::generate().public_key(),
        &signing,
    );
    Item::Wrap(sealed.expect("a wrap").to_cbor())
}

/// In the channel's group, at epoch 2, bob's wraps of epoch 1 to carol's recovery key and to her
/// browser are stored. Once carol deletes her account, bob's wraps of epoch 2 to them are refused
/// and stored nowhere, while his wrap of epoch 2 to alice's recovery key is stored; an upload from
/// carol's laptop's connection is not stored.
#[test]
fn wraps_to_a_deleted_accounts_recovery_key_or_browsers_are_refused() {
    let (mut s, _) = formed(3_303_040_031);
    assert_eq!(bob_rotates(&mut s), 2, "bob's empty Commit starts epoch 2");
    let f = &s.fixture;
    let t = &f.team;
    let general = f.general;
    let browser = t
        .carol
        .certified_by(f.db(), &t.carol.laptop, DeviceKind::Browser, "Browser");
    let to_recovery = wrap(general, 1, &t.carol, &t.bob);
    stores(
        &mut s.bob,
        f,
        &to_recovery,
        "a wrap to carol's recovery key",
    );
    let to_browser = browser_wrap(general, 1, &browser, &t.bob);
    stores(&mut s.bob, f, &to_browser, "a wrap to carol's browser");

    deletes(&Api::of(&t.flow), &t.carol);

    let refused = [
        (
            wrap(general, 2, &t.carol, &t.bob),
            "to carol's recovery key",
        ),
        (
            browser_wrap(general, 2, &browser, &t.bob),
            "to carol's browser",
        ),
    ];
    for (item, what) in &refused {
        let answer = upload(&mut s.bob, f, item);
        assert_refused_upload(&answer, f, item, what);
    }
    let to_alice = wrap(general, 2, &t.alice, &t.bob);
    stores(&mut s.bob, f, &to_alice, "a wrap to alice's recovery key");
    let carols = wrap(general, 2, &t.alice, &t.carol);
    let answer = attempt(&mut s.carol.socket, t.space, general, &carols);
    assert_ne!(
        answer,
        Some(Answer::Stored),
        "an upload from carol's laptop"
    );
    assert_nowhere(f.db(), carols.bytes());
}
