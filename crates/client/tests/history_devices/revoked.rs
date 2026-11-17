//! A full copy of a revoked device's disk, behind a compromised server that hides the revocation
//! and hands it every entry and link of the channel, shows the history written before the
//! revocation and none after, and no history key in its sealed state opens a later entry or link.

use std::fs;

use rusqlite::Connection;
use rusqlite::types::ValueRef;
use serde_json::json;
use skald_crypto::StateKey;
use skald_proto::{Cbor as _, ChannelId, DeviceId, HistoryEntry, SpaceId};

use crate::chain::{key_of, links, uploads_link};
use crate::cloud::Cloud;
use crate::devices::{BOB, CAROL, said_by};
use crate::keys;
use crate::kit::{group_info_request, received};
use crate::machine::Machine;
use crate::reading::{assert_history, rotates, sends, shown_history, without_key};
use crate::revocations::{
    carried, copy_of, hide_revocations, naming, revoked_by_cli, serve_as_not_revoked,
    space_revocations,
};
use crate::talk::{ADA_HOST, Talk};
use crate::world::now;

/// What Bob and Carol say after Ada's device is revoked; no text is anywhere else.
const AFTER: [&str; 3] = [
    "bob shares the new escrow account number",
    "carol confirms the escrow transfer date",
    "bob posts the rotated signing passphrase",
];

/// Carol, Bob and Ada each say a line in epoch 1 of the talk's channel; Bob's device commits an
/// empty Commit, epoch 2, which Ada's run enters, and Carol and Ada each say one more. A full copy
/// of the disk of Ada's machine is taken; then her machine revokes its own device. Bob's device
/// removes it by a Commit carrying the revocation: epoch 3, where Bob and Carol each say a line;
/// Bob's device commits an empty Commit: epoch 4, where Bob says one more. Bob's device uploads the
/// link of each epoch it makes. A compromised server serves the copy as a device that is not
/// revoked, hides the revocation and every GroupInfo from it, so it cannot rejoin, and gives it the
/// channel's entries and links as to any session of Ada's account.
///
/// The copy's `skald history` shows the five lines from before the revocation, oldest first, and
/// none of the three after; it holds no key for those three. The keys of epochs 3 and 4 open their
/// entries and links, and none of them is in the copy's sealed state or in the clear on its disk.
#[test]
fn copy_of_a_revoked_device_s_disk_reads_no_history_written_after_its_revocation() {
    let mut talk = Talk::new(
        &[4_169_003_001, 4_169_003_002, 4_169_003_003],
        &[
            "ada-copied-then-revoked",
            "bob-removes-the-copy",
            "carol-writes-after",
        ],
        false,
    );
    let (at, space, ada) = (talk.at(), talk.space.name.clone(), talk.ada.device);
    let mut before = vec![
        said_by(&mut talk, CAROL, (at, 1), "carol lists the vendors to pay").0,
        said_by(&mut talk, BOB, (at, 1), "bob checks the vendor invoices").0,
        sends(
            &talk.ada,
            (&space, at.1),
            "ada schedules the vendor payments",
        ),
    ];
    assert_eq!(
        rotates_by(&mut talk, BOB),
        2,
        "the epoch of Bob's empty Commit"
    );
    received(&talk.ada.machine);
    before.push(said_by(&mut talk, CAROL, (at, 2), "carol pays the first vendor").0);
    before.push(sends(
        &talk.ada,
        (&space, at.1),
        "ada reconciles the payment",
    ));
    let copy = copy_of(&talk.ada.machine, ADA_HOST);
    revoked_by_cli(&talk.ada.machine, ada);
    assert_eq!(
        bob_removes_revoked(&mut talk, ada),
        3,
        "the epoch of Bob's Remove"
    );
    let mut after = vec![
        said_by(&mut talk, BOB, (at, 2), AFTER[0]).2,
        said_by(&mut talk, CAROL, (at, 3), AFTER[1]).2,
    ];
    assert_eq!(
        rotates_by(&mut talk, BOB),
        4,
        "the epoch of Bob's empty Commit"
    );
    after.push(said_by(&mut talk, BOB, (at, 3), AFTER[2]).2);
    let cloud = talk.cloud();
    serve_as_not_revoked(cloud, ada);
    hide_revocations(cloud, at.0);
    withhold_group_infos(cloud, at.0, &[talk.space.default, at.1]);

    let shown = shown_history(&copy, at.1);

    assert_history(&shown, at.1, &before, &AFTER);
    assert_eq!(
        without_key(&shown),
        Some(3),
        "not the three later entries counted as under no key: {shown:?}"
    );
    let bob = &talk.played[BOB];
    let later: Vec<[u8; 16]> = [3, 4]
        .map(|epoch| *key_of(bob, at.1, epoch).to_bytes())
        .to_vec();
    for bytes in &after {
        let entry = HistoryEntry::from_cbor(bytes).expect("an entry");
        let key = key_of(bob, at.1, entry.epoch);
        assert!(
            key.open_entry(&entry).is_ok(),
            "a later entry does not open"
        );
    }
    let held = links(cloud, &talk.ada.token, at);
    for epoch in [3, 4] {
        let link = (held.iter())
            .find(|link| link.epoch == epoch)
            .unwrap_or_else(|| panic!("no link of epoch {epoch}: {held:?}"));
        assert!(
            key_of(bob, at.1, epoch).open_link(link).is_ok(),
            "the link of epoch {epoch} does not open"
        );
    }
    let sealed = unsealed(&copy, &talk.ada.token, ada);
    let earlier = *key_of(bob, at.1, 2).to_bytes();
    assert!(
        sealed
            .iter()
            .any(|plaintext| keys::holds(plaintext, &earlier)),
        "the copy's sealed state, as the test opens it, holds no key of epoch 2"
    );
    for (key, epoch) in later.iter().zip([3, 4]) {
        assert!(
            !sealed.iter().any(|plaintext| keys::holds(plaintext, key)),
            "the copy's sealed state holds the key of epoch {epoch}"
        );
        let holding: Vec<_> = (copy.files().into_iter())
            .filter(|file| fs::read(file).is_ok_and(|bytes| keys::holds(&bytes, key)))
            .collect();
        assert!(
            holding.is_empty(),
            "the key of epoch {epoch} in the clear: {holding:?}"
        );
    }
}

/// The talk's played device `who` commits an empty Commit in the talk's channel and uploads the
/// new epoch's link; gives the new epoch.
#[track_caller]
fn rotates_by(talk: &mut Talk, who: usize) -> u64 {
    let at = talk.at();
    rotates(&talk.world.cloud, &mut talk.played[who], at)
}

/// Bob's device takes in its mailbox, learns from the server the revocations of `revoked`, and
/// removes that device from the talk's channel by a Commit carrying them; once the server stores
/// it, the device uploads the new epoch's GroupInfo and its link to the epoch before. Gives the
/// new epoch.
#[track_caller]
fn bob_removes_revoked(talk: &mut Talk, revoked: DeviceId) -> u64 {
    let at = talk.at();
    let cloud = &talk.world.cloud;
    let remover = &mut talk.played[BOB];
    remover.take_in(cloud);
    let revocations = space_revocations(cloud, &remover.person.token, at.0);
    let counting = naming(&revocations, revoked);
    assert!(
        !counting.is_empty(),
        "{revoked} is not revoked: {revocations:?}"
    );
    let removal = (remover.mls)
        .remove_devices(at.1, &[revoked], &carried(&counting), now())
        .expect("a Commit removing the revoked device");
    remover.commits(cloud, at, &removal);
    remover.uploads_group_info(cloud, at);
    let epoch = remover.mls.epoch(at.1).expect("the epoch of the Remove");
    uploads_link(cloud, remover, at, (epoch, epoch - 1));
    epoch
}

/// From now on, the relay says each of `channels` of `space` has a group but no GroupInfo of its
/// current epoch, so no device can join it by itself, as a compromised server can withhold it.
fn withhold_group_infos(cloud: &Cloud, space: SpaceId, channels: &[ChannelId]) {
    let none = json!({ "group_info": null, "group": true }).to_string();
    for channel in channels {
        let answer = none.clone();
        let request = group_info_request(space, *channel);
        cloud.relay.rewrite(&request, move |_| answer.clone());
    }
}

/// Every plaintext that a value of a row of `machine`'s local state opens to as an item sealed
/// under a state key of the device `device`: the state key one of the 32-byte keys of the secrets
/// file, which holds `token`, and the item's name another value of the same row.
fn unsealed(machine: &Machine, token: &str, device: DeviceId) -> Vec<Vec<u8>> {
    let holders = machine.files_holding(token);
    assert_eq!(holders.len(), 1, "the secrets file: {holders:?}");
    let secrets = fs::read(&holders[0]).expect("read the secrets file");
    let state_keys: Vec<StateKey> = (keys::public_keys(&secrets, as_is).iter())
        .map(StateKey::from_bytes)
        .collect();
    let mut opened = Vec::new();
    for row in state_rows(machine) {
        for (at, name) in row.iter().enumerate() {
            let sealed = (row.iter().enumerate()).filter(|(other, _)| *other != at);
            for (_, bytes) in sealed {
                for key in &state_keys {
                    if let Some(plaintext) = key.open(device, name, bytes) {
                        opened.push(plaintext.to_vec());
                    }
                }
            }
        }
    }
    opened
}

/// The 32 bytes as they are.
fn as_is(bytes: &[u8; 32]) -> [u8; 32] {
    *bytes
}

/// The texts and blobs of each row of each table of `machine`'s local state.
fn state_rows(machine: &Machine) -> Vec<Vec<Vec<u8>>> {
    let state = Connection::open(machine.state_dir().join("state.sqlite3"))
        .expect("open the copy's local state");
    let mut tables = state
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table'")
        .expect("list the tables");
    let names: Vec<String> = (tables.query_map([], |found| found.get(0)))
        .expect("the tables")
        .collect::<Result<_, _>>()
        .expect("a table's name");
    let mut all = Vec::new();
    for name in names {
        let mut select = state
            .prepare(&format!("SELECT * FROM \"{name}\""))
            .expect("select a table");
        let width = select.column_count();
        let mut found = select.query([]).expect("read a table");
        while let Some(row) = found.next().expect("a row") {
            let values: Vec<Vec<u8>> = (0..width)
                .filter_map(|column| match row.get_ref(column).expect("a value") {
                    ValueRef::Blob(bytes) | ValueRef::Text(bytes) => Some(bytes.to_vec()),
                    _ => None,
                })
                .collect();
            all.push(values);
        }
    }
    all
}
