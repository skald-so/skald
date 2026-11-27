//! The channel history through the recovery key (`docs/protocol/membership.md`, "Account",
//! "Device checks", "Key chain", "Wraps", "Uploads" and "Access").
//!
//! - Member devices check each member account's record with that account's `master`, the one they
//!   pinned for their own and the roster's for others, take the valid one with the highest
//!   `revision`, and warn when an account's `recovery` changes.
//! - They wrap each clean epoch's key to the recovery key of each account among the channel's
//!   members, sealed to the recovery HPKE key of its record and signed by a member device; an
//!   account without a record gets none, and no epoch judged unclean is wrapped.
//! - A device linked with the recovery key opens the wraps to it of each channel of the spaces it
//!   follows before it deletes the key, and keeps their keys only sealed. It reads the history
//!   before it is in the channels, back along the key chain, then joins them by itself and removes
//!   the revoked devices there, also when it replaced every device of its account.
//!
//! As the tests of the history on new devices (`history_devices/main.rs`): the CLI runs on Ada's
//! machines; Bob's and Carol's devices are played by the tests with an MLS state of their own, and
//! say each message as their own client would, uploading its entry. Ada's first machine signs up at
//! a terminal, which shows her recovery key, as in the tests of linking with it
//! (`recovery_linking/main.rs`); her other machines that do not use the key are linked by hand. A
//! wiped machine is one that never runs again; Ada's new machine, linked with the recovery key,
//! names each wiped device lost.
//!
//! The server's API, as these tests use it, beside that of the tests of history wraps: `GET
//! /spaces/{space}/devices` gives, beside `devices` and `revocations`, `records`: the account record
//! of each member account of the space that has one, as its CBOR bytes in base64url without
//! padding, each once. The relay rewrites it, as a compromised server would. The table `accounts`,
//! whose `record` and `backup` a test sets.
//!
//! The CLI, as these tests use it: `skald sign-in --with-recovery-key`, `skald
//! recv`, `skald send`, `skald history` and `skald add-rule`, as the tests of linking with the
//! recovery key, channel messages, reading the history and handing messages to agents use them;
//! `SKALD_CLOCK_OFFSET`, as the tests of scheduled rotation use it. A warning is a line saying
//! `warning`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod everything;
mod forged;
mod hidden;
mod records;
mod sealed;
mod without_history;
mod wrapped;

// The modules the tests of the channel history share (`history_common/mod.rs`); the helpers of the
// tests of the history on new devices, scheduled rotation, reading the history, history entries
// and history wraps; the terminal and signing in there of the tests of the recovery key, and
// linking with it of the tests of linking with the recovery key; the recovery key's derivation.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_rotation/aged.rs"]
mod aged;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_entries/entries.rs"]
mod entries;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../../../crypto/tests/common/master_backup.rs"]
mod master_backup;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../recovery_linking/recovering.rs"]
mod recovering;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../history_wraps/wraps.rs"]
mod wraps;

use serde_json::Value;
use skald_crypto::{HistoryKey, HpkeKey};
use skald_proto::{
    AccountId, Cbor as _, ChannelId, DeviceId, HistoryWrap, MasterBackup, Role, SpaceId,
    WrapRecipient,
};

use cloud::Cloud;
use fixtures::{fronted, hex};
use kit::{b64, latest};
use machine::Machine;
use recovering::{Ada, LOST, NEW_HOST, new_device, recover, sign_up};
use talk::{ADA_HOST, Talk};
use world::{Person, World, devices_request, handle_of, space_id};

/// The host name of Ada's second machine, linked by hand before her devices are wiped, and of the
/// laptop her new machine links by hand after.
pub const SECOND_HOST: &str = "ada-tablet";
pub const LAPTOP_HOST: &str = "ada-new-laptop";

/// The talk of the tests of channel messages in `acme`, revision 3 giving Bob and, if any, Carol
/// `role`, but with Ada, GitHub user `ids[0]` with `logins[0]`, signed up at a terminal; gives it
/// and Ada's recovery key, in its shown form.
pub fn talk_with_key(ids: &[u64], logins: &[&str], role: Role) -> (Talk, String) {
    let (world, front) = fronted();
    let Ada { person, key } = sign_up(&world, (ids[0], logins[0]), ADA_HOST);
    let talk = Talk::of(
        (world, front, person),
        world::ACME,
        (&ids[1..], &logins[1..]),
        (false, role),
    );
    (talk, key)
}

/// The recovery HPKE key pair the recovery key `shown`, in its shown form, derives on `account`, as
/// "Account" says at the cost of this test build.
pub fn recovery_hpke(shown: &str, account: AccountId) -> HpkeKey {
    let digits = master_backup::digits(shown).expect("a recovery key in its shown form");
    let (_, ikm) = master_backup::derive_as_built(&digits, account.as_bytes());
    HpkeKey::from_ikm(&ikm)
}

/// Ada, with her recovery key `key`, links a new machine with it at a terminal, naming each of
/// `wiped` lost; fails unless the run succeeds. Gives the person on the new machine.
#[track_caller]
pub fn recovers(talk: &Talk, key: &str, wiped: &[DeviceId]) -> Person {
    let machine = Machine::new().with_host_name(NEW_HOST);
    let named: Vec<(&str, DeviceId)> = wiped.iter().map(|device| (LOST, *device)).collect();
    let shown = recover(&talk.world, &talk.ada.user, &machine, &named, Some(key));
    assert!(
        shown.success,
        "linking with the recovery key failed: {shown:?}"
    );
    person_on(&talk.world, &talk.ada, (machine, NEW_HOST), wiped)
}

/// The person of `ada`'s account on `machine`, with the host name `host`, whose device is the one
/// of the account not among `known` and holds a certificate, and whose files hold its session.
#[track_caller]
pub fn person_on(
    world: &World,
    ada: &Person,
    (machine, host): (Machine, &'static str),
    known: &[DeviceId],
) -> Person {
    let cloud = &world.cloud;
    let device = new_device(cloud, ada, known);
    let tokens = cloud.token_files(&machine);
    assert_eq!(tokens.len(), 1, "the new machine's sessions: {tokens:?}");
    let token = tokens[0].1.clone();
    let cert = (cloud.cert(&token, device)).expect("the new device's certificate");
    let key = keys::signing_key(&machine.files_holding(&token), &cert.body().signing);
    Person {
        user: ada.user.clone(),
        handle: handle_of(cloud, &token),
        machine,
        token,
        account: ada.account,
        master: ada.master,
        device,
        host,
        key,
    }
}

/// The default channel of `person`'s personal space: the space's ID and name, and the channel.
pub struct Personal {
    pub space: SpaceId,
    pub name: String,
    pub channel: ChannelId,
}

impl Personal {
    pub fn of(cloud: &Cloud, person: &Person) -> Self {
        let name = person.handle.to_string();
        let space = space_id(cloud, &person.token, &name);
        let revision = latest(cloud, &person.token, space);
        let channel = (revision.body().channels.first())
            .unwrap_or_else(|| panic!("no default channel in {space}"))
            .id;
        Personal {
            space,
            name,
            channel,
        }
    }

    pub fn at(&self) -> (SpaceId, ChannelId) {
        (self.space, self.channel)
    }
}

/// The wraps of `at`'s channel the server gives the session `token` to the recovery key of
/// `account`.
#[track_caller]
pub fn to_recovery_of(
    cloud: &Cloud,
    token: &str,
    at: (SpaceId, ChannelId),
    account: AccountId,
) -> Vec<HistoryWrap> {
    (wraps::wraps(cloud, token, at).into_iter())
        .filter(|wrap| wrap.body().recipient == WrapRecipient::Recovery(account))
        .collect()
}

/// The epochs of the wraps of the talk's channel the server holds to the recovery key of
/// `account`, each as often as it comes.
#[track_caller]
pub fn recovery_epochs(talk: &Talk, account: AccountId) -> Vec<u64> {
    let held = to_recovery_of(talk.cloud(), &talk.ada.token, talk.at(), account);
    held.iter().map(|wrap| wrap.body().epoch).collect()
}

/// The key of the one wrap of the talk's channel and `epoch` that the server holds to the recovery
/// key of `account`: fails unless it opens with `recovery`, that key's HPKE key pair, is signed by
/// Ada's device, as its `signer` says, and gives the key of `epoch` Bob's device derives. Bob's
/// device takes in its mailbox first.
#[track_caller]
pub fn assert_wrapped_to_recovery(
    talk: &mut Talk,
    (account, recovery): (AccountId, &HpkeKey),
    epoch: u64,
) -> HistoryKey {
    let recipient = WrapRecipient::Recovery(account);
    wraps::assert_wrapped_as(talk, (recipient, recovery), epoch)
}

/// The server's database holds no account record or master backup of `account`, as for an account
/// whose master key came before account records.
pub fn without_record(cloud: &Cloud, account: AccountId) {
    cloud.execute(&format!(
        "UPDATE accounts SET record = NULL, backup = NULL WHERE id = '{account}'::uuid"
    ));
}

/// The server's database holds `record`, an account record's bytes, as the record of `account`,
/// with a master backup of the spec's form.
pub fn with_record(cloud: &Cloud, account: AccountId, record: &[u8]) {
    let backup = MasterBackup {
        nonce: [0xe0; 12],
        ct: [0xc0; 48],
    };
    cloud.execute(&format!(
        "UPDATE accounts SET record = '\\x{}'::bytea, backup = '\\x{}'::bytea
         WHERE id = '{account}'::uuid",
        hex(record),
        hex(&backup.to_cbor())
    ));
}

/// From now on, the relay gives the machines the devices the server gives for `space` with the
/// account records `added`, each an account record's bytes, after those it gives, as a compromised
/// server would.
pub fn add_records(cloud: &Cloud, space: SpaceId, added: Vec<Vec<u8>>) {
    cloud.relay.rewrite(&devices_request(space), move |body| {
        let mut given: Value = serde_json::from_str(body).expect("the server's device list");
        let mut records: Vec<Value> = (given["records"].as_array().cloned()).unwrap_or_default();
        records.extend(added.iter().map(|record| Value::String(b64(record))));
        given["records"] = Value::Array(records);
        given.to_string()
    });
}

/// The talk of Ada, GitHub user `ids[0]` with `logins[0]`, signed up at a terminal, and Bob, the
/// next: in epoch 1 of the channel Bob and Ada each say a line; Bob's device commits an empty
/// Commit and uploads the link of the new epoch, epoch 2, where Bob says one more; Ada's machine
/// runs once more. Gives the talk, Ada's recovery key and the lines, oldest first.
pub fn talked(ids: &[u64; 2], logins: &[&str; 2]) -> (Talk, String, Vec<reading::Said>) {
    let (mut talk, key) = talk_with_key(ids, logins, Role::Member);
    let at = talk.at();
    let space = talk.space.name.clone();
    let mut said = vec![
        devices::said_by(
            &mut talk,
            devices::BOB,
            (at, 1),
            "bob asks for the audit dates",
        )
        .0,
        reading::sends(&talk.ada, (&space, at.1), "ada gives the audit dates"),
    ];
    let cloud = &talk.world.cloud;
    assert_eq!(
        reading::rotates(cloud, &mut talk.played[devices::BOB], at),
        2,
        "the epoch of Bob's empty Commit"
    );
    said.push(devices::said_by(&mut talk, devices::BOB, (at, 2), "bob books the auditors").0);
    kit::received(&talk.ada.machine);
    (talk, key, said)
}

/// The request for `at`'s channel's history wraps, as its line starts.
pub fn wraps_request((space, channel): (SpaceId, ChannelId)) -> String {
    format!("GET /spaces/{space}/channels/{channel}/history/wraps")
}
