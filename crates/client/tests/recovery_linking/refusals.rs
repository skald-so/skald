//! What a device linked with the recovery key refuses: a key that does not open the master backup,
//! and a master key the server gives without a record it signed and a backup that opens in PSK mode
//! with the recovery key. It signs nothing then.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use skald_crypto::{HpkeKey, RecoveryKey, SigningKey, sign_account};
use skald_proto::{AccountRecord, Cbor as _, MasterBackup};

use crate::cloud::ASK_MASTER;
use crate::keys;
use crate::machine::Machine;
use crate::revocations::Fixture;
use crate::world::World;
use crate::{
    ADA_HOST, Ada, LOST, NEW_HOST, NO_RECOVERY_KEY, assert_links_later, assert_signed_nothing,
    recover, sign_up,
};

/// The master backup's HPKE `info`, as the spec fixes it.
const BACKUP_INFO: &[u8] = b"skald master backup";

/// Ada's new machine links with `key` naming her first device lost; fails unless the run fails and
/// no machine signed anything. Gives the machine.
#[track_caller]
fn refused(world: &World, ada: &Ada, key: &str) -> Machine {
    let cloud = &world.cloud;
    let machine = Machine::new().with_host_name(NEW_HOST);
    let since = cloud.relay.mark();
    let first = ada.person.device;

    let shown = recover(world, ada.user(), &machine, &[(LOST, first)], Some(key));

    assert!(!shown.success, "{shown:?}");
    assert_signed_nothing(cloud, &ada.person.token, (since, &[first]), &shown.text);
    machine
}

/// From now on the relay gives the account's master key, record and backup as the server does.
fn stop_forging(world: &World) {
    world
        .cloud
        .relay
        .rewrite(ASK_MASTER, |answer| answer.to_owned());
}

/// From now on the relay gives, in place of the account's master key, record and backup, those
/// `forge` makes of the record the server holds, as a compromised server would.
fn forge_master(world: &World, forge: impl Fn(&AccountRecord) -> Value + Send + Sync + 'static) {
    world.cloud.relay.rewrite(ASK_MASTER, move |answer| {
        let answer: Value = serde_json::from_str(answer).unwrap_or(Value::Null);
        let record = URL_SAFE_NO_PAD
            .decode(answer["record"].as_str().unwrap_or_default())
            .unwrap_or_default();
        match AccountRecord::from_cbor(&record) {
            Ok(record) => forge(&record).to_string(),
            Err(_) => answer.to_string(),
        }
    });
}

/// `bytes` in base64url without padding, as a JSON string.
fn b64(bytes: &[u8]) -> Value {
    Value::String(URL_SAFE_NO_PAD.encode(bytes))
}

/// Ada types a recovery key that is not her account's: the run fails and signs nothing. With the
/// right key, a later run on that machine links it.
#[test]
fn a_wrong_recovery_key_is_refused() {
    let world = World::start();
    let ada = sign_up(&world, (4_176_600_001, "ada-recovery-wrong"), ADA_HOST);
    let wrong = RecoveryKey::generate();

    let machine = refused(&world, &ada, &wrong.shown());

    assert_links_later(&world, &ada, &machine);
}

/// The server gives another master key, with a record it signed naming the account's recovery HPKE
/// key and a backup of its private key sealed to that key in base mode, which needs no recovery
/// key to make. The run fails, signs nothing and keeps none of it. Once the server's answer is
/// given as it is, a later run on that machine links it.
#[test]
fn a_master_backup_sealed_without_the_recovery_key_is_refused() {
    let world = World::start();
    let ada = sign_up(&world, (4_176_600_101, "ada-recovery-base-mode"), ADA_HOST);
    let forged = SigningKey::generate();
    let forged_public = forged.public_key();
    let seed = *forged.to_bytes();
    forge_master(&world, move |record| {
        let mut body = record.body().clone();
        body.master = forged_public;
        let master = SigningKey::from_bytes(&seed);
        let record = sign_account(&body, &master).expect("an account record");
        let account = body.account.as_bytes();
        let (enc, ct) =
            HpkeKey::seal(&body.recovery, BACKUP_INFO, account, &seed).expect("a sealed backup");
        let backup = MasterBackup { enc, ct };
        serde_json::json!({
            "master": b64(&forged_public),
            "record": b64(&record.to_cbor()),
            "backup": b64(&backup.to_cbor()),
        })
    });

    let machine = refused(&world, &ada, &ada.key);

    for file in machine.files() {
        let bytes = std::fs::read(&file).unwrap_or_default();
        let kept = keys::holds(&bytes, &forged_public);
        assert!(!kept, "{} keeps the forged master key", file.display());
    }
    stop_forging(&world);
    assert_links_later(&world, &ada, &machine);
}

/// The server gives the account's master key and backup with its record re-signed by another key.
/// The run fails and signs nothing; once the server's answer is given as it is, a later run on that
/// machine links it.
#[test]
fn an_account_record_the_master_key_did_not_sign_is_refused() {
    let world = World::start();
    let ada = sign_up(&world, (4_176_600_201, "ada-recovery-record"), ADA_HOST);
    let (status, held) = world.cloud.get_as("/account/master", &ada.person.token);
    assert_eq!(status, 200, "{held}");
    let other = SigningKey::generate().to_bytes();
    forge_master(&world, move |record| {
        let signer = SigningKey::from_bytes(&other);
        let resigned = sign_account(record.body(), &signer).expect("an account record");
        let mut answer = held.clone();
        answer["record"] = b64(&resigned.to_cbor());
        answer
    });

    let machine = refused(&world, &ada, &ada.key);

    stop_forging(&world);
    assert_links_later(&world, &ada, &machine);
}

/// An account whose master key the server holds without a record or a backup, with two devices the
/// master key certified. Ada's new machine, linking with the recovery key naming one of them lost,
/// says the account has no recovery key and signs nothing.
#[test]
fn on_an_account_without_a_master_backup_the_new_device_says_so_and_signs_nothing() {
    let world = World::start();
    let cloud = &world.cloud;
    let fixture = Fixture::new(
        cloud,
        &world.browser,
        (4_176_600_301, "ada-recovery-no-backup"),
        ["ada-desk", "ada-laptop"],
    );
    let person = &fixture.person;
    let machine = Machine::new().with_host_name(NEW_HOST);
    let since = cloud.relay.mark();
    let typed = RecoveryKey::generate().shown();
    let named = [(LOST, person.device)];

    let shown = recover(&world, &person.user, &machine, &named, Some(&typed));

    assert!(shown.text.contains(NO_RECOVERY_KEY), "{shown:?}");
    let certified = [person.device, fixture.second.id];
    assert_signed_nothing(cloud, &person.token, (since, &certified), &shown.text);
}
