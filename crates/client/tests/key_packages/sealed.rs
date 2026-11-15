//! The MLS state on disk: only sealed, and no older sealed state after a save. The state holds the
//! private keys of the device's KeyPackages; a run that uploads KeyPackages saves it.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use rusqlite::Connection;
use rusqlite::types::ValueRef;

use crate::fixtures::keyed;
use crate::keys;
use crate::kit::{Pair, hpke_keys};
use crate::machine::Machine;

/// The least length of a value the tests look for on disk: shorter ones, such as counters, are
/// found by chance.
const LEAST: usize = 32;

/// The SQLite databases of `machine`'s local state, by the first bytes of their files.
fn databases(machine: &Machine) -> Vec<PathBuf> {
    machine
        .sqlite_files()
        .into_iter()
        .filter(|file| fs::read(file).is_ok_and(|bytes| bytes.starts_with(b"SQLite format 3\0")))
        .collect()
}

/// Every text and blob of at least [`LEAST`] bytes that the tables of `machine`'s local state
/// hold, as SQLite reads them.
fn values(machine: &Machine) -> BTreeSet<Vec<u8>> {
    let mut found = BTreeSet::new();
    for database in databases(machine) {
        let db = Connection::open(&database).expect("open the local state");
        let tables: Vec<String> = db
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .and_then(|mut names| {
                names
                    .query_map([], |row| row.get(0))
                    .and_then(Iterator::collect)
            })
            .expect("the local state's tables");
        for table in tables {
            let sql = format!("SELECT * FROM \"{}\"", table.replace('"', "\"\""));
            let mut select = db.prepare(&sql).expect("select a table");
            let columns = select.column_count();
            let mut rows = select.query([]).expect("read a table");
            while let Some(row) = rows.next().expect("a row") {
                for column in 0..columns {
                    let bytes = match row.get_ref(column).expect("a value") {
                        ValueRef::Blob(bytes) | ValueRef::Text(bytes) => bytes,
                        _ => continue,
                    };
                    if bytes.len() >= LEAST {
                        found.insert(bytes.to_vec());
                    }
                }
            }
        }
    }
    found
}

/// The identity, to list every 32 bytes that look like a key.
fn same(bytes: &[u8; 32]) -> [u8; 32] {
    *bytes
}

/// The bytes of the secrets file of `pair`'s Ada.
fn secrets(pair: &Pair) -> Vec<u8> {
    let holders = pair.ada.machine.files_holding(&pair.ada.token);
    assert_eq!(holders.len(), 1, "the secrets file: {holders:?}");
    fs::read(&holders[0]).expect("read the secrets file")
}

/// The 32-byte keys the secrets file of `pair`'s Ada holds beside its session token and its
/// device's signing and HPKE private keys.
fn other_secrets(pair: &Pair) -> BTreeSet<[u8; 32]> {
    let device = keyed(pair.cloud(), &pair.ada);
    let known = [*device.signing.to_bytes(), *device.hpke.to_bytes()];
    let token: BTreeSet<[u8; 32]> = keys::public_keys(pair.ada.token.as_bytes(), same)
        .into_iter()
        .collect();
    keys::public_keys(&secrets(pair), same)
        .into_iter()
        .filter(|key| !known.contains(key) && !token.contains(key))
        .collect()
}

/// The bytes of each file of `machine`'s local state, its databases and their journals and WALs.
fn state_files(machine: &Machine) -> Vec<(PathBuf, Vec<u8>)> {
    machine
        .sqlite_files()
        .into_iter()
        .map(|file| {
            let bytes = fs::read(&file).expect("read the local state");
            (file, bytes)
        })
        .collect()
}

/// Ada's device saves its MLS state when a run uploads 11 KeyPackages, Bob having taken 11 after
/// an earlier run accepted what was new. Every value the save removed from the local state, the
/// previous sealed state among them, is then in neither the database file nor its journal or WAL.
/// The state key is in key storage, the secrets file, beside the device keys; none of the files
/// holds in the clear a private key of the KeyPackages the device made or any key of its secrets
/// file: its signing and HPKE private keys and the state key. No channel group exists yet, so the
/// state holds no history key.
#[test]
fn save_leaves_no_older_sealed_state_on_disk_and_no_key_in_the_clear() {
    let pair = Pair::start(
        [4_130_020_001, 4_130_020_002],
        ["ada-sealed-state-standin", "bob-sealed-state-standin"],
    );
    let machine = &pair.ada.machine;
    pair.recv();
    let taken = pair.take_others(11);
    let before = values(machine);

    pair.recv();

    let files = state_files(machine);
    let after = values(machine);
    let removed: Vec<&Vec<u8>> = before.difference(&after).collect();
    let removed_bytes: usize = removed.iter().map(|value| value.len()).sum();
    // The private keys of 21 KeyPackages, two of 32 bytes each, sealed again.
    assert!(
        removed_bytes >= 21 * 64,
        "the run did not replace the sealed state: {removed_bytes} bytes removed"
    );
    for (file, bytes) in &files {
        for value in &removed {
            assert!(
                !bytes
                    .windows(value.len())
                    .any(|window| window == value.as_slice()),
                "{file:?} keeps {} bytes the save removed",
                value.len()
            );
        }
    }

    let state_key = other_secrets(&pair);
    assert!(
        !state_key.is_empty(),
        "the secrets file holds no key beside the device keys and the session token"
    );
    let (others, last) = pair.take_all();
    let kp_keys = hpke_keys(taken.iter().chain(&others).chain([&last]));
    let secret_keys: BTreeSet<[u8; 32]> = keys::public_keys(&secrets(&pair), same)
        .into_iter()
        .collect();
    for (file, bytes) in &files {
        let found = keys::public_keys(bytes, keys::x25519);
        assert!(
            found.iter().all(|public| !kp_keys.contains(public)),
            "{file:?} holds a KeyPackage's private key"
        );
        let in_file: BTreeSet<[u8; 32]> = keys::public_keys(bytes, same).into_iter().collect();
        assert!(
            in_file.is_disjoint(&secret_keys),
            "{file:?} holds a key of the secrets file"
        );
    }
}
