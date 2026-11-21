//! What the tests share: the revocation command, revocations the tests sign, store and read from
//! the server, the `[+ revocation]` a Commit carries, what a compromised server hides, adds or
//! serves, a copy of a machine's disk, a fixture account whose master key the test holds, and what
//! the CLI shows of devices.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use skald_crypto::{SigningKey, sign_revocation};
use skald_proto::{
    AccountId, Cbor as _, Cert, ChannelId, DeviceId, Revocation, RevocationBody, SpaceId, Unknown,
};

use crate::browser::Browser;
use crate::cloud::{Cloud, GIVE_MASTER, unb64};
use crate::fixtures::{Issuer, Keyed, add_device, hex};
use crate::kit::b64;
use crate::machine::{Finished, Machine};
use crate::played::Played;
use crate::world::{Person, devices_request, handle_of, lines_naming, now, post_as, run};

/// The command that revokes a device, and what the CLI says of a revoked device.
pub const REVOKE_DEVICE: &str = "revoke-device";
pub const REVOKED: &str = "revoked";

/// The request that stores a revocation, and the one for the account's devices, as their lines
/// start.
pub const STORE_REVOCATION: &str = "POST /account/revocations ";
pub const ACCOUNT_DEVICES: &str = "GET /devices ";

/// Runs `skald revoke-device <device>` on `machine`.
pub fn revoke_device(machine: &Machine, device: DeviceId) -> Finished {
    run(machine, &[REVOKE_DEVICE, &device.to_string()])
}

/// As [`revoke_device`]; fails unless the run succeeds.
#[track_caller]
pub fn revoked_by_cli(machine: &Machine, device: DeviceId) -> Finished {
    let finished = revoke_device(machine, device);
    assert!(finished.success, "{finished:?}");
    finished
}

/// The revocation of `device` on `account`, by `signer`, the master key if `None`, keeping nothing.
pub fn body(account: AccountId, device: DeviceId, signer: Option<DeviceId>) -> RevocationBody {
    RevocationBody {
        account,
        device,
        keep: None,
        signer,
        timestamp: now(),
        also: None,
        unknown: Unknown::default(),
    }
}

/// `body` signed by `key`.
pub fn signed(body: &RevocationBody, key: &SigningKey) -> Revocation {
    sign_revocation(body, key).expect("a revocation body the spec allows")
}

/// `revocation` with its signature broken.
pub fn broken(revocation: &Revocation) -> Revocation {
    let mut signature = *revocation.signature();
    signature[0] ^= 1;
    Revocation::from_parts(revocation.body_bytes().to_vec(), signature).expect("a revocation")
}

/// Stores `revocation` as the session `token`, through the API; fails unless the server stores it
/// as new.
#[track_caller]
pub fn store_revocation(cloud: &Cloud, token: &str, revocation: &Revocation) {
    let request = json!({ "revocation": b64(&revocation.to_cbor()) });
    let (status, body) = post_as(cloud, token, "/account/revocations", Some(&request));
    assert_eq!(status, 201, "the revocation is not stored: {body}");
}

/// `person`'s device revokes `device`, one of its account's, as the CLI would: the revocation,
/// signed by the device and stored from its session.
#[track_caller]
pub fn revoked_by(cloud: &Cloud, person: &Person, device: DeviceId) -> Revocation {
    let revocation = signed(
        &body(person.account, device, Some(person.device)),
        &person.key,
    );
    store_revocation(cloud, &person.token, &revocation);
    revocation
}

/// The revocations in `answer`, a `Devices` answer of the server.
fn revocations_in(answer: &Value) -> Vec<Revocation> {
    answer["revocations"]
        .as_array()
        .unwrap_or_else(|| panic!("no revocations in {answer}"))
        .iter()
        .map(|text| {
            let bytes = unb64(text.as_str().expect("a revocation in base64url"));
            Revocation::from_cbor(&bytes).expect("a revocation")
        })
        .collect()
}

/// The revocations of the account of the session `token`, straight from the server.
pub fn account_revocations(cloud: &Cloud, token: &str) -> Vec<Revocation> {
    let (status, answer) = cloud.get_as("/devices", token);
    assert_eq!(status, 200, "GET /devices: {answer}");
    revocations_in(&answer)
}

/// The revocations of `space`'s accounts the session `token` gets, straight from the server.
pub fn space_revocations(cloud: &Cloud, token: &str, space: SpaceId) -> Vec<Revocation> {
    let path = format!("/spaces/{space}/devices");
    let (status, answer) = cloud.get_as(&path, token);
    assert_eq!(status, 200, "GET {path}: {answer}");
    revocations_in(&answer)
}

/// The certificates of the account of the session `token`, straight from the server.
pub fn account_certs(cloud: &Cloud, token: &str) -> Vec<Cert> {
    cloud
        .devices(token)
        .into_iter()
        .filter_map(|device| device.cert)
        .collect()
}

/// The revocations of `revocations` whose `device` is `device`.
pub fn naming(revocations: &[Revocation], device: DeviceId) -> Vec<Revocation> {
    revocations
        .iter()
        .filter(|revocation| revocation.body().device == device)
        .cloned()
        .collect()
}

/// Whether `held` holds `revocation`, byte for byte.
pub fn holds(held: &[Revocation], revocation: &Revocation) -> bool {
    let bytes = revocation.to_cbor();
    held.iter().any(|one| one.to_cbor() == bytes)
}

/// The encoded `[+ revocation]` of `revocations`, as a Commit's `authenticated_data` carries them.
pub fn carried(revocations: &[Revocation]) -> Vec<u8> {
    let count = u8::try_from(revocations.len())
        .ok()
        .filter(|count| (1..24).contains(count))
        .expect("1 to 23 revocations");
    let mut bytes = vec![0x80 | count];
    for revocation in revocations {
        bytes.extend(revocation.to_cbor());
    }
    bytes
}

/// The head of the CBOR data item at `at` in `bytes`: its major type and argument; moves `at` past
/// it. Fails on an indefinite length, which the deterministic encoding never uses.
#[track_caller]
fn head(bytes: &[u8], at: &mut usize) -> (u8, u64) {
    let first = *bytes.get(*at).expect("a CBOR head");
    *at += 1;
    let (major, info) = (first >> 5, first & 0x1f);
    let argument = match info {
        0..=23 => u64::from(info),
        24..=27 => {
            let length = 1 << (info - 24);
            let held = bytes.get(*at..*at + length).expect("a CBOR argument");
            *at += length;
            held.iter()
                .fold(0, |value, byte| (value << 8) | u64::from(*byte))
        }
        _ => panic!("not a definite CBOR head: {first:#04x}"),
    };
    (major, argument)
}

/// The revocations `authenticated_data` carries, an encoded `[+ revocation]`; fails unless it is
/// one.
#[track_caller]
pub fn carried_in(authenticated_data: &[u8]) -> Vec<Revocation> {
    let mut at = 0;
    let (major, count) = head(authenticated_data, &mut at);
    assert_eq!(major, 4, "not an array: {authenticated_data:?}");
    let mut found = Vec::new();
    for _ in 0..count {
        let start = at;
        assert_eq!(
            head(authenticated_data, &mut at),
            (4, 2),
            "not a signed object"
        );
        for _ in 0..2 {
            let (major, length) = head(authenticated_data, &mut at);
            assert_eq!(major, 2, "not a byte string");
            at += usize::try_from(length).expect("a length");
        }
        let item = authenticated_data
            .get(start..at)
            .expect("a whole revocation");
        found.push(Revocation::from_cbor(item).expect("a revocation"));
    }
    assert_eq!(at, authenticated_data.len(), "bytes after the revocations");
    found
}

/// From now on, the relay gives the machines the revocations `change` makes of those of each
/// answer for the account's devices and for `space`'s, as a compromised server would.
pub fn change_revocations(
    cloud: &Cloud,
    space: SpaceId,
    change: impl Fn(&mut Vec<Value>) + Clone + Send + Sync + 'static,
) {
    for request in [ACCOUNT_DEVICES.to_owned(), devices_request(space)] {
        let change = change.clone();
        cloud.relay.rewrite(&request, move |answer| {
            let mut held: Value = serde_json::from_str(answer).expect("the server's devices");
            let mut list = held["revocations"].as_array().cloned().unwrap_or_default();
            change(&mut list);
            held["revocations"] = Value::Array(list);
            held.to_string()
        });
    }
}

/// From now on, the relay gives the machines no revocation with the certificates.
pub fn hide_revocations(cloud: &Cloud, space: SpaceId) {
    change_revocations(cloud, space, Vec::clear);
}

/// From now on, the relay gives the machines `added` beside the revocations the server gives.
pub fn add_revocations(cloud: &Cloud, space: SpaceId, added: &[Revocation]) {
    let added: Vec<Value> = added
        .iter()
        .map(|revocation| json!(b64(&revocation.to_cbor())))
        .collect();
    change_revocations(cloud, space, move |list| list.extend(added.iter().cloned()));
}

/// From now on, the server's database serves `device` as a device that is not revoked, as a
/// compromised server would.
pub fn serve_as_not_revoked(cloud: &Cloud, device: DeviceId) {
    cloud.execute(&format!(
        "UPDATE devices SET revoked = false WHERE id = '{device}'::uuid"
    ));
}

/// A new machine with the host name `host` and a copy of every file `machine` has on disk.
pub fn copy_of(machine: &Machine, host: &str) -> Machine {
    let copy = Machine::new().with_host_name(host);
    copy_dir(machine.root(), copy.root());
    copy
}

fn copy_dir(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).expect("read a machine's directory") {
        let entry = entry.expect("a machine's file");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file's type").is_dir() {
            fs::create_dir_all(&target).expect("copy a machine's directory");
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy a machine's file");
        }
    }
}

/// Fails unless `shown`, what `skald devices` or `skald members` showed, lists the device named
/// `name`, and says it is revoked exactly when `revoked`.
#[track_caller]
pub fn assert_listed(shown: &Finished, name: &str, revoked: bool) {
    let lines = lines_naming(shown, name);
    assert!(!lines.is_empty(), "{name} is not listed: {shown:?}");
    let marked = lines
        .iter()
        .any(|line| line.to_lowercase().contains(REVOKED));
    assert_eq!(marked, revoked, "{name} listed as revoked: {shown:?}");
}

/// A fixture account whose master key the test holds, with two `client` devices it certified,
/// which the test writes to the server's database, and the session of a machine that signed in to
/// it.
pub struct Fixture {
    pub master: SigningKey,
    /// Its first device, as a person whose machine holds that device's keys: the machine is the
    /// one that signed in, without a certificate.
    pub person: Person,
    /// Its second device.
    pub second: Keyed,
}

impl Fixture {
    /// GitHub user `id` with `login` signs in on a new machine, which creates no master key; the
    /// test sets the account's master key and writes the devices `names` to the server's
    /// database, each certified by it.
    pub fn new(
        cloud: &Cloud,
        browser: &Browser,
        (id, login): (u64, &str),
        names: [&'static str; 2],
    ) -> Self {
        let user = cloud.user(id, login);
        let machine = Machine::new().with_host_name(&format!("{login}-machine"));
        // The machine signs in and, as the server refuses its master key as another device's taken
        // first, keeps none and waits for linking.
        cloud.relay.refuse(GIVE_MASTER, "409 Conflict");
        let _ = cloud.signed_in(&browser.page(), &machine, &user, &[]);
        cloud.relay.stop_refusing();
        let token = cloud.last_token();
        let (account, handle) = (cloud.account_of(&token), handle_of(cloud, &token));
        let master = SigningKey::generate();
        cloud.execute(&format!(
            "UPDATE accounts SET master = '\\x{}'::bytea WHERE id = '{account}'::uuid",
            hex(&master.public_key())
        ));
        let first = add_device(cloud, account, names[0], &Issuer::Master(&master));
        let second = add_device(cloud, account, names[1], &Issuer::Master(&master));
        let person = Person {
            user,
            machine,
            token,
            account,
            master: master.public_key(),
            handle,
            device: first.id,
            host: names[0],
            key: first.signing,
        };
        Fixture {
            master,
            person,
            second,
        }
    }
}

/// Fails unless `played` entered, from a Commit of `sender`'s device, an epoch of `channel`'s group
/// without `removed`, and that Commit carries `revocation`, byte for byte, and only revocations of
/// `removed`.
#[track_caller]
pub fn assert_removed_carrying(
    played: &Played,
    channel: ChannelId,
    (sender, removed): (DeviceId, DeviceId),
    revocation: &Revocation,
) {
    let changes = played.changes_of(channel);
    let change = changes
        .iter()
        .find(|change| change.removed.iter().any(|member| member.device == removed))
        .unwrap_or_else(|| panic!("no Commit removed {removed} from {channel}: {changes:?}"));
    assert_eq!(change.sender, sender, "the Commit's sender: {change:?}");
    let revocations = carried_in(&change.authenticated_data);
    assert!(
        holds(&revocations, revocation),
        "the Commit does not carry the revocation: {revocations:?}"
    );
    assert!(
        revocations
            .iter()
            .all(|carried| carried.body().device == removed),
        "the Commit carries another device's revocation: {revocations:?}"
    );
}
