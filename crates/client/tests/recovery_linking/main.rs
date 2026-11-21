//! Linking a new device with the recovery key (`docs/protocol/membership.md`, "Linking a device",
//! "Anchors", "Revocation" and "Device checks"). The new device, signed in at a terminal to an
//! account with a master key, reads the recovery key there without echo and pins the account's
//! master key only from an account record and a master backup that opens in PSK mode with that key.
//! It issues its own certificate with the master key, shows the account's devices, revoked or not,
//! then revokes with the master key the devices its user names stolen, stores its certificate,
//! starts from the anchors of the devices no master-key revocation marks as stolen, revokes with
//! its own key the devices its user names lost, and keeps neither key. Against a `skald-server`
//! with GitHub sign-in through the stand-in, behind the recording relay of the sign-in tests, which
//! the tests use as a proxy that sees and changes what goes through; the person's terminal is a
//! pseudo-terminal, or a pseudo-console on Windows, and a test plays the person in headless Chrome.
//!
//! Ada's account is made by her first machine at a terminal, which shows its recovery key; her
//! other machines are linked by hand, each certified with the keys of the machine that links it, as
//! in the tests of the history on new devices (`history_devices/main.rs`). Bob's account is one of
//! a member of her spaces, whose session checks her devices as the tests of revoking linked devices
//! do (`linked_revocation/main.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests of spaces and of revoking
//! devices:
//! - `skald sign-in --server <URL> --with-recovery-key`, on an account with a
//!   master key, links the device with the recovery key instead of a link code, and shows no code.
//!   `--stolen <device>`, `--lost <device>`, `--keep <device>` and `--keep-revoked <device>`, each
//!   once per device, by its ID, name the devices its user names stolen, lost or wiped, kept valid
//!   (a revocation's `keep`) and kept revoked (a master-key revocation's `also`); given any of them,
//!   it asks about no device.
//! - At a terminal, once signed in, it asks for the recovery key on the last line it shows, a line
//!   that holds `recovery key`, and reads the key there without echo, ignoring case, spaces and
//!   `-`. It shows each device of the account on a line that holds its certificate's `name`, saying
//!   `revoked` for a revoked one, before it revokes any. It exits with success once it is done.
//! - A wrong recovery key, or an account record or master backup it must reject, makes it exit
//!   failing, signing nothing. So does a refusal of its certificate over the devices limit, saying
//!   on a line that the account has as many devices as it may; a later run with the recovery key on
//!   the same device takes it up again.
//! - On an account without a master backup, it says on a line that the account has `no recovery
//!   key`, and signs nothing.
//! - Through a pipe, or run with `--agent <name>`, it refuses and signs nothing.
//! - A revocation it signs waits in the local state until the server takes it, and the next run
//!   that connects, such as `skald devices`, sends it again, with the same bytes; a run killed
//!   before it is done leaves what it did not sign to a later run with the recovery key on the same
//!   device.
//! - `skald spaces` lists the spaces the device follows, one line each, holding the space's name;
//!   the personal space's line says `personal`. A warning is a line saying `warning`: of a space it
//!   does not join as its anchors disagree, holding the space's name or ID; of anchors objects that
//!   name different personal spaces, holding `personal`.
//!
//! The server's API, as these tests use it besides that of revocations and anchors objects: `PUT
//! /devices/{device}/cert` of `{"cert": <bytes>}`, the new device's complete certificate, and `GET
//! /account/master`, which gives the account's master key, account record and master backup.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod anchoring;
mod certifying;
mod input;
mod limit;
mod naming;
mod recovering;
mod refusals;
mod resends;
mod secrets;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices, the helpers of the tests of reading the
// history and of revoking linked devices, the anchors objects of the tests of anchors, and the
// terminal, signing in there and the system's key storage of the tests of the recovery key.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../anchors/objects.rs"]
mod anchors;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../linked_revocation/linked.rs"]
mod linked;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../recovery_key/system.rs"]
mod system;
#[path = "../recovery_key/terminal.rs"]
mod terminal;

use std::time::Instant;

use skald_crypto::{DeviceCheckError, RecoveryKey, check_valid_device, open_master_backup};
use skald_proto::{Cbor as _, DeviceId, MasterBackup, Revocation, Role, Roster, SpaceId};

use anchors::ANCHORS_REQUEST;
use cloud::{Cloud, certified, unb64};
use machine::Machine;
use revocations::{STORE_REVOCATION, account_certs, account_revocations, naming};
use world::{ACME, Person, World, create, next_by, store, words};

pub use recovering::*;

/// How a device of `ada`'s account checks `device`: with her master key and the certificates and
/// revocations the server holds of her account.
pub fn checked(cloud: &Cloud, ada: &Person, device: DeviceId) -> Result<(), DeviceCheckError> {
    let certs = account_certs(cloud, &ada.token);
    let revocations = account_revocations(cloud, &ada.token);
    check_valid_device(&ada.master, &certs, &revocations, device).map(|_| ())
}

/// The revocations of `device` the server holds of `ada`'s account.
pub fn revocations_of(cloud: &Cloud, ada: &Person, device: DeviceId) -> Vec<Revocation> {
    naming(&account_revocations(cloud, &ada.token), device)
}

/// Fails unless the server holds one revocation of `device`, by `signer`, the master key if
/// `None`, which counts: `device` is revoked for `ada`'s devices. Gives it.
#[track_caller]
pub fn revoked_once_by(
    cloud: &Cloud,
    ada: &Person,
    device: DeviceId,
    signer: Option<DeviceId>,
) -> Revocation {
    let held = revocations_of(cloud, ada, device);
    assert_eq!(held.len(), 1, "not one revocation of {device}: {held:?}");
    assert_eq!(held[0].body().signer, signer, "its signer: {held:?}");
    assert!(
        matches!(
            checked(cloud, ada, device),
            Err(DeviceCheckError::Revoked { .. })
        ),
        "{device} is not revoked: {held:?}"
    );
    held[0].clone()
}

/// Fails unless the server holds a certificate of `device` that the master key issued, without
/// `issuer`, valid for `ada`'s devices.
#[track_caller]
pub fn assert_certified_by_the_master_key(cloud: &Cloud, ada: &Person, device: DeviceId) {
    let cert = (cloud.cert(&ada.token, device)).expect("the new device's certificate");
    assert_eq!(cert.body().issuer, None, "its issuer: {:?}", cert.body());
    assert!(
        checked(cloud, ada, device).is_ok(),
        "the new device is not valid: {:?}",
        checked(cloud, ada, device)
    );
}

/// The devices with a certificate of the account of the session `token`, as the server lists them.
pub fn certified_now(cloud: &Cloud, token: &str) -> Vec<DeviceId> {
    certified(&cloud.devices(token))
}

/// After a run on `machine` that was refused, Ada links it with her recovery key at a terminal,
/// naming her first device lost; fails unless the run succeeds and the server then holds the new
/// device's certificate, issued by the master key: the refusal was the run's own.
#[track_caller]
pub fn assert_links_later(world: &World, ada: &Ada, machine: &Machine) {
    let first = ada.person.device;
    recovered(world, ada, machine, &[(LOST, first)]);
    let new = new_device(&world.cloud, &ada.person, &[first]);
    assert_certified_by_the_master_key(&world.cloud, &ada.person, new);
}

/// Fails unless, since `since`, no machine sent a certificate, a revocation or an anchors object,
/// and the server holds no revocation of the account of the session `token`, and certificates only
/// of the devices `certified_before`; `shown` is what the new device's run showed.
#[track_caller]
pub fn assert_signed_nothing(
    cloud: &Cloud,
    token: &str,
    (since, certified_before): (Instant, &[DeviceId]),
    shown: &str,
) {
    for request in [STORE_CERT, STORE_REVOCATION, ANCHORS_REQUEST] {
        assert_eq!(
            cloud.count_sent(since, request),
            0,
            "sent {request}: {shown}"
        );
    }
    assert!(
        account_revocations(cloud, token).is_empty(),
        "a revocation stored: {shown}"
    );
    let mut before = certified_before.to_vec();
    before.sort();
    let mut now = certified_now(cloud, token);
    now.sort();
    assert_eq!(now, before, "a certificate stored: {shown}");
}

/// Whether `text` shows the recovery key `key`, in any case, with or without spaces and `-`.
pub fn shows_key(text: &str, key: &str) -> bool {
    let squeezed = |text: &str| -> String {
        text.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_uppercase()
    };
    squeezed(text).contains(&squeezed(key))
}

/// `key` as a person may type it: in lowercase, with spaces in place of `-`.
pub fn typed(key: &str) -> String {
    key.to_lowercase().replace('-', " ")
}

/// The master private key that `ada`'s account's master backup holds, opened with her recovery key.
#[track_caller]
pub fn master_private_key(cloud: &Cloud, ada: &Ada) -> [u8; 32] {
    let person = &ada.person;
    let (status, body) = cloud.get_as("/account/master", &person.token);
    assert_eq!(status, 200, "GET /account/master: {body}");
    let backup = body["backup"].as_str().expect("a master backup");
    let backup = MasterBackup::from_cbor(&unb64(backup)).expect("a master backup that decodes");
    let key = RecoveryKey::parse(&ada.key).expect("a recovery key");
    let master = open_master_backup(&backup, &key, person.account, &person.master)
        .expect("the backup opens with the recovery key");
    *master.to_bytes()
}

/// Bob, GitHub user `id` with `login`, signs up; Ada's first machine creates `acme` and signs its
/// revision 3, which adds Bob as an admin, and accepts it. Gives Bob, `acme` and revision 3.
#[track_caller]
pub fn with_bob(
    world: &World,
    ada: &Person,
    (id, login): (u64, &str),
) -> (Person, SpaceId, Roster) {
    let cloud = &world.cloud;
    let bob = world.sign_up(id, login, BOB_HOST);
    let created = create(cloud, ada, ACME);
    let third = next_by(&created, ada, |members| {
        members.push(bob.member(Role::Admin));
    });
    store(cloud, &ada.token, &third);
    world::members(&ada.machine, ACME);
    (bob, created.body().space, third)
}

/// The first line of `text` that is no warning and holds `name` as a word, if any.
pub fn first_line_naming<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    text.lines()
        .filter(|line| !line.to_lowercase().contains("warning"))
        .find(|line| words(line).contains(&name))
}

/// The warning lines of `text`, in lowercase.
pub fn warnings_in(text: &str) -> Vec<String> {
    text.lines()
        .map(str::to_lowercase)
        .filter(|line| line.contains("warning"))
        .collect()
}
