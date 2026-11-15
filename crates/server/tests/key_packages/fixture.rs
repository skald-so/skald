//! What the tests share: the team of `acme`, where alice, bob and carol are members and dave is in
//! no space; the KeyPackages a device makes with the client's own code; and the requests of the
//! KeyPackages API and of revocations.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use skald_crypto::{MlsDevice, SigningKey, StateKey};
use skald_proto::{AccountId, DeviceId, DeviceKind, Role};

use crate::answers::{handed_of, status_of};
use crate::api::{Person, Session, Team};
use crate::common::Response;
use crate::common::cbor::{array_of, byte_item, head, uint};
use crate::common::device_flow::{json, text};
use crate::common::devices::{Device, assert_taken, b64, cert_of, write_devices};
use crate::common::times::DAY;
use crate::common::{TestDb, hex};
use crate::rosters::{member, next};

/// Hands out KeyPackages: `POST`.
pub const HAND_OUT: &str = "/key-packages";
/// Stores a revocation of a device of the session's account: `POST`.
pub const REVOCATIONS: &str = "/account/revocations";
/// The label a revocation is signed over.
const REVOCATION_LABEL: &str = "skald revocation";

/// The table of the KeyPackages the server holds.
pub const TABLE: &str = "key_packages";

/// The KeyPackages of one device: `POST` uploads, `GET` counts.
pub fn path(device: DeviceId) -> String {
    format!("/devices/{device}/key-packages")
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}

/// `acme` with revisions 1 and 2 stored: alice and bob admins, carol a member; dave, signed in,
/// shares no space with them.
pub struct Stand {
    pub team: Team,
    /// The revisions stored so far, in order.
    pub revisions: Vec<Vec<u8>>,
}

impl Stand {
    /// The stand on a server of its own; GitHub user IDs from `github` on.
    pub fn new(github: u64) -> Self {
        let team = Team::new(github);
        let revisions = team.with_members().to_vec();
        Stand { team, revisions }
    }

    pub fn db(&self) -> &TestDb {
        &self.team.flow.stand.db
    }

    /// Stores the next revision of `acme`, signed by alice's laptop, which adds dave as a member.
    pub fn add_dave(&mut self) {
        let last = self.revisions.last().expect("a revision");
        let mut body = next(last, &self.team.alice.laptop);
        body.members.push(member(&self.team.dave, Role::Member));
        let revision = self.team.alice.sign(&body);
        self.team.stored_by(&self.team.alice, &revision);
        self.revisions.push(revision);
    }

    /// Writes to the database a second `client` device of `person`'s account, certified by its
    /// master key, as linking will register it.
    pub fn phone(&self, person: &Person) -> Device {
        let phone = Device::new("Phone");
        let cert = phone.cert_as(person.account, DeviceKind::Client, &person.master);
        write_devices(
            self.db(),
            person.account,
            DeviceKind::Client,
            &[(&phone, Some(&cert))],
        );
        phone
    }

    /// Writes to the database a `browser` device of `person`'s account with a valid certificate,
    /// issued by its laptop.
    pub fn browser(&self, person: &Person) -> Device {
        let browser = Device::new("Browser");
        let mut body = browser.body_as(person.account, DeviceKind::Browser);
        body.issuer = Some(person.laptop.id);
        let cert = cert_of(&body, &person.laptop.signing.key, &browser.signing.key);
        write_devices(
            self.db(),
            person.account,
            DeviceKind::Browser,
            &[(&browser, Some(&cert))],
        );
        browser
    }

    /// Whether the database holds `bytes`, such as a KeyPackage's.
    pub fn holds(&self, bytes: &[u8]) -> bool {
        self.db().rows().concat().contains(&hex(bytes))
    }

    /// Moves the times of every KeyPackage the server holds back by `interval`, as if that much
    /// time had passed.
    pub fn age(&self, interval: &str) {
        self.db()
            .execute(&crate::common::times::moved_back(&[TABLE], interval));
    }
}

/// `device`'s MLS client, as the client's own code makes KeyPackages with it.
pub fn mls(device: &Device) -> MlsDevice {
    MlsDevice::new(
        &SigningKey::from_bytes(&device.signing.secret),
        &StateKey::generate(),
    )
}

/// `count` KeyPackages of `device` made at `at`, in seconds since the Unix epoch.
pub fn made_at(device: &mut MlsDevice, count: usize, at: u64) -> Vec<Vec<u8>> {
    (0..count)
        .map(|_| device.key_package(at).expect("a KeyPackage").message)
        .collect()
}

/// `count` KeyPackages of `device` made now, which expire in 60 days.
pub fn fresh(device: &mut MlsDevice, count: usize) -> Vec<Vec<u8>> {
    made_at(device, count, now())
}

/// `count` KeyPackages of `device` that hold now and expire within 7 days: made 54 days ago.
pub fn expiring(device: &mut MlsDevice, count: usize) -> Vec<Vec<u8>> {
    made_at(device, count, now() - 54 * DAY)
}

/// A last-resort KeyPackage of `device` made now.
pub fn last_resort(device: &mut MlsDevice) -> Vec<u8> {
    device
        .last_resort_key_package(now())
        .expect("a last-resort KeyPackage")
        .message
}

/// The upload of `key_packages` and, if given, the last-resort `last`.
pub fn upload_body(key_packages: &[Vec<u8>], last: Option<&[u8]>) -> Value {
    let mut body = json!({});
    if !key_packages.is_empty() {
        let list: Vec<String> = key_packages.iter().map(|bytes| b64(bytes)).collect();
        body["key_packages"] = json!(list);
    }
    if let Some(last) = last {
        body["last_resort"] = json!(b64(last));
    }
    body
}

/// `POST /devices/<device>/key-packages` of `key_packages` and the last-resort `last`, from
/// `session`.
pub fn upload(
    session: &Session,
    device: DeviceId,
    key_packages: &[Vec<u8>],
    last: Option<&[u8]>,
) -> Response {
    let body = upload_body(key_packages, last);
    session.call("post", &path(device), Some(&body))
}

/// As [`upload`]; fails unless the server takes them.
#[track_caller]
pub fn uploaded(
    session: &Session,
    device: DeviceId,
    key_packages: &[Vec<u8>],
    last: Option<&[u8]>,
) {
    assert_taken(&upload(session, device, key_packages, last));
}

/// What `GET /devices/<device>/key-packages` gives `session`: how many unused KeyPackages the
/// device has that expire in more than 7 days, and whether its last-resort one is to be replaced:
/// it has none, it was handed out or it expires within 7 days.
#[track_caller]
pub fn status(session: &Session, device: DeviceId) -> (u64, bool) {
    let response = session.call("get", &path(device), None);
    assert_eq!(response.status, 200, "{}", text(&response));
    status_of(&json(&response))
}

/// `POST /key-packages` for `devices`, from `session`.
pub fn hand_out_response(session: &Session, devices: &[DeviceId]) -> Response {
    let devices: Vec<String> = devices.iter().map(ToString::to_string).collect();
    session.call("post", HAND_OUT, Some(&json!({ "devices": devices })))
}

/// The KeyPackages the server hands `session` for `devices`: at most one by each device's ID.
#[track_caller]
pub fn hand_out(session: &Session, devices: &[DeviceId]) -> BTreeMap<DeviceId, Vec<u8>> {
    let response = hand_out_response(session, devices);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    let mut handed = BTreeMap::new();
    for (device, bytes) in handed_of(&body) {
        assert!(
            devices.contains(&device),
            "a KeyPackage of {device}, not asked for: {body}"
        );
        assert!(
            handed.insert(device, bytes).is_none(),
            "two KeyPackages of {device}: {body}"
        );
    }
    handed
}

/// The one KeyPackage the server hands `session` for `device`, if any.
#[track_caller]
pub fn hand_out_one(session: &Session, device: DeviceId) -> Option<Vec<u8>> {
    hand_out(session, &[device]).remove(&device)
}

/// The revocation of `device` of `account` by the account's master key `master`, encoded as
/// `docs/protocol/membership.md`, "Revocation", says: `[body, signature]`, its body
/// `{1: account, 2: device, 5: timestamp}`.
pub fn master_revocation(account: AccountId, device: DeviceId, master: &SigningKey) -> Vec<u8> {
    let mut body = head(5, 3);
    body.extend(uint(1));
    body.extend(byte_item(account.as_bytes()));
    body.extend(uint(2));
    body.extend(byte_item(device.as_bytes()));
    body.extend(uint(5));
    body.extend(uint(now()));
    let signature = master.sign(REVOCATION_LABEL, &[&body]);
    array_of(&[byte_item(&body), byte_item(&signature)])
}

/// `person`'s session stores the revocation of its account's `device` by its master key; fails
/// unless the server takes it.
#[track_caller]
pub fn revoke(person: &Person, device: DeviceId) {
    let revocation = master_revocation(person.account, device, &person.master.key);
    let body = json!({ "revocation": b64(&revocation) });
    let response = person.session.call("post", REVOCATIONS, Some(&body));
    assert_taken(&response);
}
