//! A revoked device's mailbox: its entries expire at revocation, their senders get `expired` for
//! those sent with `report` true, and no later entry is added to it.

use std::thread;

use skald_proto::{DeviceId, ServerStatus};

use crate::common::devices::Device;
use crate::fixture::Fixture;
use crate::kit::{assert_device_refused, assert_open, by_bobs_laptop, connect};
use crate::link::{
    Entry, Link, PUSH, QUIET, Sent, assert_delivered, assert_status, await_seqs, await_seqs_within,
    bytes, messages, seqs, store_to,
};
use crate::wire::{Socket, UNKNOWN_RECIPIENT, envelope, now};

/// Checks that `entries` are the `expired` of each message of `sent` to `recipient`, in any order,
/// each issued at `before` or later.
#[track_caller]
fn assert_expired(entries: &[Entry], sent: &[Sent], recipient: DeviceId, before: u64) {
    assert_eq!(entries.len(), sent.len(), "{entries:?}");
    let mut reported: Vec<_> = sent.iter().map(|(header, _)| header.msg_id).collect();
    for entry in entries {
        let Entry::Status(_, status) = entry else {
            panic!("a status, not {entry:?}");
        };
        let at = reported
            .iter()
            .position(|&msg_id| msg_id == status.msg_id)
            .unwrap_or_else(|| panic!("a status of no message left: {status:?}"));
        let msg_id = reported.remove(at);
        let header = &sent
            .iter()
            .find(|(header, _)| header.msg_id == msg_id)
            .expect("a sent message")
            .0;
        assert_status(entry, ServerStatus::Expired, header, recipient, before);
    }
}

/// alice, connected to another instance than the one that stores the revocation, sends bob's
/// laptop four envelopes, three with `report` true; bob acks the first. Once bob's master key
/// revokes the laptop, alice gets `expired` for the two others with `report` true and nothing
/// for the one without; the laptop's mailbox is empty and its proof is refused.
#[test]
fn a_revoked_devices_entries_expire_and_their_senders_get_expired_for_report_true_only() {
    let f = Fixture::new(3_131_320_301);
    let t = &f.team;
    let laptop = t.bob.laptop.id;
    let (_other, mut alice) = Link::elsewhere(&t.flow, &t.alice);
    let acked = store_to(&mut alice, &f, laptop, 1, true);
    let reported = store_to(&mut alice, &f, laptop, 2, true);
    let unreported = store_to(&mut alice, &f, laptop, 1, false);
    let all: Vec<Sent> = [&acked, &reported, &unreported]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    let mut bob = Link::synced(&t.flow, &t.bob, 0, &messages(1, &bytes(&all)));
    let before = now();
    bob.ack(1);
    assert_delivered(&alice.entry(), &acked[0].0, laptop, before);
    await_seqs(f.db(), laptop, &[2, 3, 4]);
    drop(bob);

    let before = now();
    t.bob.revoke(&t.bob.by_master(laptop, &[]));
    let expired = alice.entries(reported.len());
    assert_expired(&expired, &reported, laptop, before);
    alice.assert_quiet();
    await_seqs_within(f.db(), laptop, &[], PUSH);
    let server = &t.flow.stand.server;
    assert_device_refused(&mut connect(server, &t.flow, &t.bob, &t.bob.laptop));
}

/// bob's laptop sends alice's laptop an envelope with `report` true, then bob's master key revokes
/// it. alice's ack of the envelope adds no `delivered` to the laptop's mailbox, and an envelope to
/// it is refused with `unknown-recipient`, adding no entry.
#[test]
fn no_later_entry_is_added_to_a_revoked_devices_mailbox() {
    let f = Fixture::new(3_131_320_311);
    let t = &f.team;
    let db = f.db();
    let laptop = t.bob.laptop.id;
    let mut bob = Link::synced(&t.flow, &t.bob, 0, &[]);
    let header = f.to(t.alice.laptop.id);
    let sent = envelope(&header);
    bob.store(&header, &sent, true);
    drop(bob);

    t.bob.revoke(&t.bob.by_master(laptop, &[]));
    let mut alice = Link::synced(&t.flow, &t.alice, 0, &[Entry::Message(1, sent)]);
    alice.ack(1);
    await_seqs(db, t.alice.laptop.id, &[]);
    thread::sleep(QUIET);
    assert_eq!(seqs(db, laptop), Vec::<i64>::new(), "after alice's ack");

    let mut sender = Socket::of(&t.flow, &t.alice);
    let header = f.to(laptop);
    let (_, problem) = sender.refusal(header.msg_id, &envelope(&header));
    assert_eq!(problem, UNKNOWN_RECIPIENT, "an envelope to the laptop");
    thread::sleep(QUIET);
    assert_eq!(seqs(db, laptop), Vec::<i64>::new(), "after a send to it");
}

/// bob's master key revokes bob's laptop, keeping its phone but not its tablet, both certified by
/// the laptop. The tablet's entry expires and alice gets `expired` for it; the phone keeps its
/// entry and connects, while the tablet and the laptop are refused.
#[test]
fn the_devices_a_revoked_device_certified_are_revoked_with_it_but_those_it_keeps() {
    let f = Fixture::new(3_131_320_321);
    let t = &f.team;
    let db = f.db();
    let bob = &t.bob;
    let phone = by_bobs_laptop(&f, "Phone");
    let tablet = by_bobs_laptop(&f, "Tablet");
    let mut alice = Link::synced(&t.flow, &t.alice, 0, &[]);
    store_to(&mut alice, &f, phone.id, 1, true);
    let to_tablet = store_to(&mut alice, &f, tablet.id, 1, true);

    let before = now();
    bob.revoke(&bob.by_master(bob.laptop.id, &[phone.id]));
    assert_expired(&[alice.entry()], &to_tablet, tablet.id, before);
    alice.assert_quiet();
    await_seqs_within(db, tablet.id, &[], PUSH);
    assert_eq!(seqs(db, phone.id), [1], "the phone's mailbox");

    let server = &t.flow.stand.server;
    let refused: [&Device; 2] = [&tablet, &bob.laptop];
    for device in refused {
        assert_device_refused(&mut connect(server, &t.flow, bob, device));
    }
    assert_open(&mut connect(server, &t.flow, bob, &phone));
}
