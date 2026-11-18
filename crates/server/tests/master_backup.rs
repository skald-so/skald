//! The account's master key on the server comes with its account record and master backup
//! (`docs/protocol/membership.md`, "Account"): the server takes a master key only with a master
//! backup and an account record of the session's account that the key signs and names; the same
//! ones sent again count as stored; the account's sessions get the record and the backup as they
//! were sent. Every response is checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The API, each request stating the protocol version, as `common/devices.rs` describes it, and:
//! - `PUT /account/master` of `{"master": <bytes>, "cert": <bytes>, "record": <bytes>, "backup":
//!   <bytes>}`: with the master public key and the first device's certificate, the CBOR bytes of
//!   the account record, `account`, and of the master backup, `master-backup`;
//! - `GET /account/master`: `{"master": <bytes> | null, "record": <bytes> | null, "backup": <bytes> |
//!   null}`.
//!
//! The records are encoded byte by byte and signed with the tests' keys; the server cannot open a
//! backup, so the tests' backups are random bytes of its form.

mod common;

use std::sync::Barrier;
use std::thread;

use common::cbor::{array_of, byte_item, head, signed, uint};
use common::device_flow::{DeviceFlow, json, text};
use common::devices::{
    Device, Key, MASTER, Machine, account_id, assert_refused, assert_taken, b64, machine, random,
    unb64,
};
use common::sign_in::Account;
use common::{Response, hex};
use serde_json::{Value, json};
use skald_crypto::SigningKey;
use skald_proto::AccountId;

/// The label of an account record's signature.
const LABEL: &str = "skald account";

/// What an account's first device gives: its master key, its certificate, the account record and
/// the master backup, each as its bytes.
struct Given {
    master: Key,
    cert: Vec<u8>,
    record: Vec<u8>,
    backup: Vec<u8>,
}

impl Given {
    /// The request body of `PUT /account/master`.
    fn request(&self) -> Value {
        json!({
            "master": b64(&self.master.public()),
            "cert": b64(&self.cert),
            "record": b64(&self.record),
            "backup": b64(&self.backup),
        })
    }

    /// The request body without `field`.
    fn without(&self, field: &str) -> Value {
        let mut request = self.request();
        request
            .as_object_mut()
            .expect("a JSON object")
            .remove(field);
        request
    }

    /// The request body with `field` set to `bytes`.
    fn with(&self, field: &str, bytes: &[u8]) -> Value {
        let mut request = self.request();
        request[field] = json!(b64(bytes));
        request
    }
}

/// The encoded `account-body` of `account`, `master` and `recovery`, at `revision`, with the encoded
/// fields `extra` after them, already sorted.
fn body(
    account: AccountId,
    master: &[u8; 32],
    recovery: &[u8; 32],
    revision: u64,
    extra: &[Vec<u8>],
) -> Vec<u8> {
    let count = 4 + u64::try_from(extra.len()).expect("a few fields");
    let mut body = head(5, count);
    for (key, value) in [
        (1, byte_item(account.as_bytes())),
        (2, byte_item(master)),
        (3, byte_item(recovery)),
        (4, uint(revision)),
    ] {
        body.extend(uint(key));
        body.extend(value);
    }
    for field in extra {
        body.extend(field);
    }
    body
}

/// A master backup of the form `[enc, ct]`: random bytes, as the server cannot tell.
fn backup() -> Vec<u8> {
    let mut ct = random().to_vec();
    ct.extend_from_slice(&random()[..16]);
    array_of(&[byte_item(&random()), byte_item(&ct)])
}

/// What `device`, registered on `machine`'s account, gives as the account's first device: a new
/// master key, its certificate, a record the key signs and names, revision 1, and a backup.
fn first(machine: &Machine, device: &Device) -> Given {
    let master = Key::generate();
    let cert = device.cert(&machine.account, &master);
    let record = signed(
        &body(
            account_id(&machine.account),
            &master.public(),
            &random(),
            1,
            &[],
        ),
        &master.key,
        LABEL,
    );
    Given {
        master,
        cert,
        record,
        backup: backup(),
    }
}

/// The master key, record and backup of the account as `GET /account/master` gives them to the
/// session with the request `headers`.
fn held(flow: &DeviceFlow, headers: &[(&str, &str)]) -> [Option<Vec<u8>>; 3] {
    let response = flow.request("get", MASTER, headers, None, true);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    ["master", "record", "backup"].map(|field| match body.get(field).unwrap_or(&Value::Null) {
        Value::Null => None,
        Value::String(bytes) => Some(unb64(bytes)),
        other => panic!("{field} in {body}: {other}"),
    })
}

/// What `machine`'s session gets from `GET /account/master`.
fn held_by(flow: &DeviceFlow, machine: &Machine) -> [Option<Vec<u8>>; 3] {
    let bearer = format!("Bearer {}", machine.token);
    held(flow, &[("authorization", bearer.as_str())])
}

/// What `GET /account/master` gives while the account holds `given`.
fn holding(given: &Given) -> [Option<Vec<u8>>; 3] {
    [
        Some(given.master.public().to_vec()),
        Some(given.record.clone()),
        Some(given.backup.clone()),
    ]
}

/// Checks that `response` refuses as an invalid request.
#[track_caller]
fn assert_invalid(what: &str, response: &Response) {
    assert_eq!(response.status, 400, "{what}: {}", text(response));
    assert_refused(response, 400);
}

#[test]
fn the_master_key_with_its_record_and_backup_is_stored_once_also_when_sent_twice() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_170_101_001, "ada-backup-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    machine.register_device(&device);
    assert_eq!(held_by(&flow, &machine), [None, None, None]);
    let given = first(&machine, &device);

    assert_taken(&machine.give(&given.request()));
    assert_eq!(held_by(&flow, &machine), holding(&given));

    // A retry after a lost answer.
    assert_taken(&machine.give(&given.request()));
    assert_eq!(held_by(&flow, &machine), holding(&given));
    assert_eq!(
        machine.devices().get(&device.id.to_string()),
        Some(&Some(given.cert.clone()))
    );

    // Another record or backup with the same master key and certificate is not the one taken.
    let other_record = signed(
        &body(
            account_id(&machine.account),
            &given.master.public(),
            &random(),
            1,
            &[],
        ),
        &given.master.key,
        LABEL,
    );
    for (what, sent) in [
        ("another record", given.with("record", &other_record)),
        ("another backup", given.with("backup", &backup())),
    ] {
        let response = machine.give(&sent);
        assert_eq!(response.status, 409, "{what}: {}", text(&response));
        assert_refused(&response, 409);
    }
    assert_eq!(held_by(&flow, &machine), holding(&given));
}

#[test]
fn a_record_with_fields_of_later_versions_is_given_as_sent() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_170_101_002, "ada-later-record-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    machine.register_device(&device);
    let mut given = first(&machine, &device);
    // 10 => "later", after the four known fields.
    let later = [uint(10), [vec![0x65], b"later".to_vec()].concat()].concat();
    let account = account_id(&machine.account);
    given.record = signed(
        &body(account, &given.master.public(), &random(), 1, &[later]),
        &given.master.key,
        LABEL,
    );

    assert_taken(&machine.give(&given.request()));

    assert_eq!(held_by(&flow, &machine), holding(&given));
}

#[test]
fn a_master_key_without_a_valid_record_and_backup_is_refused() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_170_101_003, "ada-no-backup-standin");
    let bob = flow.stand.user(3_170_101_004, "bob-no-backup-standin");
    let other = machine(&flow, &bob);
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    machine.register_device(&device);
    let given = first(&machine, &device);
    let account = account_id(&machine.account);
    let master = given.master.public();
    let stranger = Key::generate();
    let recovery = random();
    let record_of = |account: AccountId, named: &[u8; 32], key: &SigningKey, label: &str| {
        signed(&body(account, named, &recovery, 1, &[]), key, label)
    };
    let mut broken = record_of(account, &master, &given.master.key, LABEL);
    let last = broken.len() - 1;
    broken[last] ^= 0x01;
    let enc_too_short = array_of(&[byte_item(&[7; 31]), byte_item(&[8; 48])]);
    let mut neither = given.without("record");
    neither
        .as_object_mut()
        .expect("a JSON object")
        .remove("backup");
    let mut not_base64 = given.request();
    not_base64["record"] = json!("not base64url!");

    let refused: Vec<(&str, Value)> = vec![
        ("without a record", given.without("record")),
        ("without a backup", given.without("backup")),
        ("with neither", neither),
        (
            "a record another key signs",
            given.with("record", &record_of(account, &master, &stranger.key, LABEL)),
        ),
        (
            "a record signed over another label",
            given.with(
                "record",
                &record_of(account, &master, &given.master.key, "skald cert"),
            ),
        ),
        (
            "a record with a broken signature",
            given.with("record", &broken),
        ),
        (
            "a record of another account",
            given.with(
                "record",
                &record_of(
                    account_id(&other.account),
                    &master,
                    &given.master.key,
                    LABEL,
                ),
            ),
        ),
        (
            "a record naming another master key",
            given.with(
                "record",
                &record_of(account, &stranger.public(), &given.master.key, LABEL),
            ),
        ),
        (
            "a record of another master key, signed by it",
            given.with(
                "record",
                &record_of(account, &stranger.public(), &stranger.key, LABEL),
            ),
        ),
        (
            "a record that does not decode",
            given.with("record", &given.record[..given.record.len() - 1]),
        ),
        (
            "a certificate as the record",
            given.with("record", &given.cert),
        ),
        (
            "a backup that does not decode",
            given.with("backup", &given.backup[1..]),
        ),
        (
            "a backup with a short enc",
            given.with("backup", &enc_too_short),
        ),
        ("an empty backup", given.with("backup", &[])),
        ("a record not in base64url", not_base64),
    ];
    for (what, sent) in &refused {
        assert_invalid(what, &machine.give(sent));
        assert_eq!(held_by(&flow, &machine), [None, None, None], "{what}");
    }
    // The account's genuine ones, from another account's session.
    assert_invalid("another account's session", &other.give(&given.request()));
    assert_eq!(held_by(&flow, &other), [None, None, None]);

    // None of them held the account back.
    assert_taken(&machine.give(&given.request()));
    assert_eq!(held_by(&flow, &machine), holding(&given));
}

#[test]
fn of_master_keys_with_their_records_sent_at_once_one_is_taken_with_its_own() {
    const AT_ONCE: usize = 4;
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_170_101_005, "ada-backup-race-standin");
    let machine = machine(&flow, &ada);
    let given: Vec<Given> = (0..AT_ONCE)
        .map(|at| {
            let device = Device::new(&format!("Racer {at}"));
            machine.register_device(&device);
            first(&machine, &device)
        })
        .collect();
    let barrier = Barrier::new(AT_ONCE);

    let statuses: Vec<u16> = thread::scope(|scope| {
        let sent: Vec<_> = given
            .iter()
            .map(|given| {
                let (barrier, machine) = (&barrier, &machine);
                scope.spawn(move || {
                    barrier.wait();
                    machine.give(&given.request()).status
                })
            })
            .collect();
        sent.into_iter()
            .map(|sent| sent.join().expect("a request"))
            .collect()
    });

    let taken: Vec<&Given> = (given.iter().zip(&statuses))
        .filter_map(|(given, status)| matches!(status, 200 | 201).then_some(given))
        .collect();
    assert_eq!(taken.len(), 1, "taken of {statuses:?}");
    assert_eq!(held_by(&flow, &machine), holding(taken[0]));
}

#[test]
fn the_record_and_backup_go_only_to_the_account_sessions() {
    let flow = DeviceFlow::start();
    let ada = flow
        .stand
        .user(3_170_101_006, "ada-backup-sessions-standin");
    let bob = flow
        .stand
        .user(3_170_101_007, "bob-backup-sessions-standin");
    let first_machine = machine(&flow, &ada);
    let second = machine(&flow, &ada);
    let other = machine(&flow, &bob);
    let device = Device::new("Laptop");
    first_machine.register_device(&device);
    let given = first(&first_machine, &device);
    assert_taken(&first_machine.give(&given.request()));

    assert_eq!(held_by(&flow, &second), holding(&given));
    let web = [("cookie", first_machine.cookie.as_str())];
    assert_eq!(
        held(&flow, &web),
        holding(&given),
        "the person's web session"
    );

    assert_eq!(held_by(&flow, &other), [None, None, None]);
    let web = [("cookie", other.cookie.as_str())];
    assert_eq!(held(&flow, &web), [None, None, None]);
    let rows = flow.stand.db.rows().join("\n");
    assert!(
        rows.contains(&hex(&given.backup)) || rows.contains(&b64(&given.backup)),
        "the database holds the backup as sent"
    );
}

#[test]
fn an_account_whose_master_key_has_no_record_gives_none() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_170_101_008, "ada-old-account-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    machine.register_device(&device);
    // An account whose first device gave its master key before records existed.
    let master = Key::generate();
    let cert = device.cert(&machine.account, &master);
    let Account { id, .. } = &machine.account;
    flow.stand.db.execute(&format!(
        "UPDATE accounts SET master = '\\x{}'::bytea WHERE id = '{id}'::uuid;
         UPDATE devices SET cert = '\\x{}'::bytea WHERE id = '{}'::uuid",
        hex(&master.public()),
        hex(&cert),
        device.id
    ));

    assert_eq!(
        held_by(&flow, &machine),
        [Some(master.public().to_vec()), None, None]
    );
    // The account has a master key: another one with a record is refused.
    let given = first(&machine, &device);
    let response = machine.give(&given.request());
    assert_eq!(response.status, 409, "{}", text(&response));
    assert_eq!(
        held_by(&flow, &machine),
        [Some(master.public().to_vec()), None, None]
    );
}
