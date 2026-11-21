//! A device linked with the recovery key keeps neither the recovery key nor the master private
//! key, and sends neither.

use crate::keys;
use crate::machine::{KEY_STORAGE, Machine};
use crate::system::Entry;
use crate::world::World;
use crate::{ADA_HOST, Ada, LOST, NEW_HOST, master_private_key, recovered, shows_key, sign_up};

/// Whether `bytes` hold `ada`'s recovery key, as its characters or in its shown form, or `master`,
/// the master private key.
fn holds_a_key(bytes: &[u8], ada: &Ada, master: &[u8; 32]) -> bool {
    let key: String = ada.key.split('-').collect();
    keys::holds(bytes, key.as_bytes())
        || keys::holds(bytes, master)
        || shows_key(&String::from_utf8_lossy(bytes), &ada.key)
}

/// Fails unless no file of `machine` holds `ada`'s recovery key or `master`.
#[track_caller]
fn assert_no_file_holds_a_key(machine: &Machine, ada: &Ada, master: &[u8; 32]) {
    for file in machine.files() {
        let bytes = std::fs::read(&file).unwrap_or_default();
        let holds = holds_a_key(&bytes, ada, master);
        assert!(!holds, "{} holds a key", file.display());
    }
}

/// Once Ada's new machine is linked with the recovery key, naming her first device lost, no file of
/// it and no request any machine sent holds the recovery key or the master private key.
#[test]
fn the_new_device_keeps_neither_the_recovery_key_nor_the_master_private_key() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = sign_up(&world, (4_176_800_001, "ada-recovery-secrets"), ADA_HOST);
    let master = master_private_key(cloud, &ada);
    let machine = Machine::new().with_host_name(NEW_HOST);

    recovered(&world, &ada, &machine, &[(LOST, ada.person.device)]);

    assert_no_file_holds_a_key(&machine, &ada, &master);
    assert!(
        !holds_a_key(&cloud.relay.sent_bytes(), &ada, &master),
        "a request holds a key"
    );
}

/// With the system's key storage, only with `SKALD_TEST_REAL_KEY_STORAGE=1`: once Ada's new
/// machine is linked with the recovery key, its key storage's entry holds neither the recovery key
/// nor the master private key, and no file of it does. The test deletes the entry.
#[test]
fn with_the_systems_key_storage_the_new_device_keeps_neither_key() {
    if std::env::var("SKALD_TEST_REAL_KEY_STORAGE").as_deref() != Ok("1") {
        return;
    }
    let world = World::start();
    let cloud = &world.cloud;
    let ada = sign_up(&world, (4_176_800_101, "ada-recovery-keyring"), ADA_HOST);
    let master = master_private_key(cloud, &ada);
    let machine = Machine::new()
        .with_host_name(NEW_HOST)
        .with_env(KEY_STORAGE, "auto");
    let entry = Entry::of(&machine);

    recovered(&world, &ada, &machine, &[(LOST, ada.person.device)]);

    let held = entry.value().expect("the entry holds the secrets");
    assert!(
        !holds_a_key(held.as_bytes(), &ada, &master),
        "the entry holds a key"
    );
    assert_no_file_holds_a_key(&machine, &ada, &master);
}
