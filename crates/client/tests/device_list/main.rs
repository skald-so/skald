//! `skald devices`: the account's devices, as the server gives them, checked against the master key
//! the machine pinned in its local state (`docs/protocol/membership.md`, "Device certificate" and
//! "Device checks"). Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on
//! PostgreSQL, behind the recording relay, which also rewrites what the server gives, as a
//! compromised server would; a test plays the person on the verification page in headless Chrome.
//! Each machine keeps its state in a directory of its own and its secrets in a file there.
//!
//! The CLI, as these tests use it:
//! - `skald devices`: lists the devices of the account the machine signed in to, from the server of
//!   its local state;
//! - on its output, one line per device, holding the device's ID: first the devices with a valid
//!   certificate, each line with its certificate's `name`, its `kind` (`client`, `browser` or
//!   `service`) and the date of its `issued` as `YYYY-MM-DD`, and the current device's line saying
//!   `this device`; then a line saying `not linked`, followed by the other devices, by ID only;
//! - the warning for a device with two certificates of different bodies: a line saying `warning`
//!   with the device's ID, on either output;
//! - on a device without a certificate, it says the device is `not linked` yet and lists no device
//!   as linked.

#[allow(dead_code)]
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[allow(dead_code)]
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "../device_keys/keys.rs"]
mod keys;
#[allow(dead_code)]
#[path = "../sign_in/machine.rs"]
mod machine;
#[allow(dead_code)]
#[path = "../sign_in/relay.rs"]
mod relay;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use browser::{Browser, Page};
use cloud::{ASK_MASTER, Cloud};
use machine::{DEVICES, FIRST_DEVICE, Finished, Machine, NO, NOT_LINKED, THIS_DEVICE};
use serde_json::{Value, json};
use skald_crypto::{SigningKey, issue};
use skald_github_stand_in::User;
use skald_proto::{AccountId, Cbor, Cert, CertBody, DeviceId, DeviceKind, Unknown};

/// What the warning for two bodies says.
const WARNING: &str = "warning";

/// The request for the device list, whose answers the proxy rewrites, as its line starts; it also
/// rewrites the answers to [`ASK_MASTER`].
const LIST: &str = "GET /devices";

/// When the certificates the tests make up were issued: 2001-02-03, at noon UTC. No date of that
/// month may show.
const MADE_UP_ISSUED: u64 = 981_201_600;
const MADE_UP_MONTH: &str = "2001-02";

/// The account's first machine, signed in with a certificate by the account's master key.
struct Ada {
    user: User,
    machine: Machine,
    /// The name its certificate holds.
    name: &'static str,
    account: AccountId,
    cert: Cert,
    /// Its signing key, as its secrets file holds it: with it, a test issues certificates as a
    /// linked device would.
    key: SigningKey,
}

impl Ada {
    fn id(&self) -> DeviceId {
        self.cert.body().device
    }
}

/// A server whose codes live for 2 minutes and poll every second, a browser for the person and the
/// account's first machine, signed in.
fn setup(github_id: u64, login: &str) -> (Cloud, Browser, Ada) {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let user = cloud.user(github_id, login);
    let name = "ada-workstation";
    let (machine, finished) = cloud.first_machine(&browser.page(), &user, name);
    assert!(finished.success, "{finished:?}");

    let token = cloud.last_token();
    let account = cloud.account_of(&token);
    let mut certs: Vec<Cert> = served(&cloud, &token)
        .iter()
        .filter_map(|entry| entry["cert"].as_str().map(cert_of))
        .collect();
    assert_eq!(certs.len(), 1, "the first device's certificate: {certs:?}");
    let cert = certs.remove(0);
    let key = keys::signing_key(&machine.files_holding(&token), &cert.body().signing);
    let ada = Ada {
        user,
        machine,
        name,
        account,
        cert,
        key,
    };
    (cloud, browser, ada)
}

/// Signs `machine` in as `user`, the person approving at `page`, answering `first` whether it is
/// the account's first device.
fn sign_in(cloud: &Cloud, page: &Page, machine: &Machine, user: &User, first: &str) -> Finished {
    cloud.signed_in(page, machine, user, &[FIRST_DEVICE, first])
}

/// A second machine of Ada's account, registered and waiting for linking, without a certificate;
/// its host name is its registered `name`.
fn second_machine(cloud: &Cloud, browser: &Browser, ada: &Ada, host: &str) -> Machine {
    let machine = Machine::new().with_host_name(host);
    sign_in(cloud, &browser.page(), &machine, &ada.user, NO);
    machine
}

/// The devices the server gives a session with `token`, as JSON, straight from the server.
fn served(cloud: &Cloud, token: &str) -> Vec<Value> {
    let (status, body) = cloud.get_as("/devices", token);
    assert_eq!(status, 200, "GET /devices: {body}");
    body["devices"].as_array().cloned().expect("a device list")
}

fn cert_of(text: &str) -> Cert {
    let bytes = URL_SAFE_NO_PAD.decode(text).expect("base64url");
    Cert::from_cbor(&bytes).expect("a certificate")
}

/// A device a test makes up, with keys of its own.
struct MadeUp {
    key: SigningKey,
}

impl MadeUp {
    fn new() -> Self {
        MadeUp {
            key: SigningKey::generate(),
        }
    }

    fn id(&self) -> DeviceId {
        DeviceId::from_signing_key(&self.key.public_key())
    }

    /// Its certificate body on `account`, by `issuer` or the master key, named `name`.
    fn body(&self, account: AccountId, issuer: Option<DeviceId>, name: &str) -> CertBody {
        CertBody {
            account,
            device: self.id(),
            kind: DeviceKind::Client,
            signing: self.key.public_key(),
            hpke: SigningKey::generate().public_key(),
            issuer,
            issued: MADE_UP_ISSUED,
            ssh: None,
            name: name.to_owned(),
            unknown: Unknown::default(),
        }
    }

    /// Its certificate of `body`, signed by `signer` and proved by its own key.
    fn cert(&self, body: &CertBody, signer: &SigningKey) -> Cert {
        issue(body, signer, &self.key).expect("a certificate")
    }
}

/// `cert` with its `proof` broken.
fn broken_proof(cert: &Cert) -> Cert {
    let mut proof = *cert.proof();
    proof[0] ^= 1;
    Cert::from_parts(cert.body_bytes().to_vec(), *cert.signature(), proof).expect("a certificate")
}

/// `cert` with its `signature` broken.
fn broken_signature(cert: &Cert) -> Cert {
    let mut signature = *cert.signature();
    signature[0] ^= 1;
    Cert::from_parts(cert.body_bytes().to_vec(), signature, *cert.proof()).expect("a certificate")
}

/// A device entry of the device list, as the server gives it.
fn entry(device: DeviceId, cert: &Cert) -> Value {
    json!({ "device": device.to_string(), "cert": URL_SAFE_NO_PAD.encode(cert.to_cbor()) })
}

/// From now on, the proxy gives the machines the device list `change` makes of the server's.
fn tamper(cloud: &Cloud, change: impl Fn(&mut Vec<Value>) + Send + Sync + 'static) {
    cloud.relay.rewrite(LIST, move |body| {
        let mut list: Value = serde_json::from_str(body).expect("the server's device list");
        let devices = list["devices"].as_array_mut().expect("a device list");
        change(devices);
        list.to_string()
    });
}

/// From now on, the proxy adds `before` to the start of the server's device list and `after` to
/// its end.
fn add(cloud: &Cloud, before: Vec<Value>, after: Vec<Value>) {
    tamper(cloud, move |devices| {
        let given = std::mem::take(devices);
        devices.extend(before.iter().cloned());
        devices.extend(given);
        devices.extend(after.iter().cloned());
    });
}

/// What `skald devices` printed, its device lines split as the list shows them.
struct Listing {
    finished: Finished,
    /// The lines of the devices with a valid certificate.
    linked: Vec<String>,
    /// The lines of the devices listed apart as not linked.
    apart: Vec<String>,
}

impl Listing {
    fn linked_lines(&self, device: DeviceId) -> Vec<&String> {
        lines_of(&self.linked, device)
    }

    fn apart_lines(&self, device: DeviceId) -> Vec<&String> {
        lines_of(&self.apart, device)
    }

    fn shown(&self) -> String {
        self.finished.shown()
    }

    /// The warning lines that name `device`.
    fn warnings(&self, device: DeviceId) -> Vec<String> {
        let id = device.to_string();
        self.shown()
            .lines()
            .filter(|line| line.to_lowercase().contains(WARNING) && line.contains(&id))
            .map(str::to_owned)
            .collect()
    }

    /// Fails unless `device` is listed once, with a valid certificate.
    fn assert_linked(&self, device: DeviceId) -> &String {
        assert_eq!(
            self.apart_lines(device).len(),
            0,
            "{device} apart: {self:?}"
        );
        let lines = self.linked_lines(device);
        assert_eq!(lines.len(), 1, "{device} not linked once: {self:?}");
        lines[0]
    }

    /// Fails unless `device` is listed once, apart as not linked, by its ID only: no made-up
    /// certificate's name or `issued` shows anywhere.
    fn assert_apart(&self, device: DeviceId, names: &[&str]) {
        assert_eq!(
            self.linked_lines(device).len(),
            0,
            "{device} linked: {self:?}"
        );
        assert_eq!(
            self.apart_lines(device).len(),
            1,
            "{device} not apart: {self:?}"
        );
        self.assert_hidden(names);
    }

    /// Fails if anything shown holds one of `names` or a date the made-up certificates hold.
    fn assert_hidden(&self, names: &[&str]) {
        let shown = self.shown();
        for name in names {
            assert!(!shown.contains(name), "{name:?} shown: {self:?}");
        }
        assert!(
            !shown.contains(MADE_UP_MONTH),
            "a made-up date shown: {self:?}"
        );
    }
}

impl std::fmt::Debug for Listing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.finished)
    }
}

fn lines_of(lines: &[String], device: DeviceId) -> Vec<&String> {
    let id = device.to_string();
    lines.iter().filter(|line| line.contains(&id)).collect()
}

/// Whether `line` holds a device ID.
fn holds_id(line: &str) -> bool {
    line.split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .any(|word| word.parse::<DeviceId>().is_ok())
}

/// Runs `skald devices` on `machine`.
fn list(machine: &Machine) -> Listing {
    let finished = machine.start(&[DEVICES]).finish();
    let mut linked = Vec::new();
    let mut apart = Vec::new();
    let mut below = false;
    for line in finished.stdout.lines() {
        let lower = line.to_lowercase();
        if lower.contains(WARNING) {
            continue;
        }
        if !holds_id(line) {
            below |= lower.contains(NOT_LINKED);
        } else if below || lower.contains(NOT_LINKED) {
            apart.push(line.to_owned());
        } else {
            linked.push(line.to_owned());
        }
    }
    Listing {
        finished,
        linked,
        apart,
    }
}

/// Runs `skald devices` on Ada's machine; fails unless it succeeds and lists her device as linked,
/// marked as this one, with no warning about it.
fn list_on_ada(ada: &Ada) -> Listing {
    let listing = list(&ada.machine);
    assert!(listing.finished.success, "{listing:?}");
    let line = listing.assert_linked(ada.id());
    assert!(line.contains(ada.name), "{line:?}");
    assert!(line.to_lowercase().contains(THIS_DEVICE), "{line:?}");
    assert_eq!(listing.warnings(ada.id()), Vec::<String>::new());
    listing
}

/// The date of `seconds` since the Unix epoch, as `YYYY-MM-DD` in UTC.
fn date(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).expect("a date");
    // Howard Hinnant's days_from_civil, reversed.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Whether `line` shows the date of `issued`, in any time zone.
fn shows_issued(line: &str, issued: u64) -> bool {
    [issued - 86_400, issued, issued + 86_400]
        .iter()
        .any(|at| line.contains(&date(*at)))
}

/// The account's first device lists itself as linked, by its certificate's `name`, `kind` and
/// `issued` and its ID, marked as this device; a second device of the account, registered and
/// waiting for linking, is listed apart as not linked, by its ID only.
#[test]
fn lists_the_linked_device_by_its_certificate_and_others_apart_by_id() {
    let (cloud, browser, ada) = setup(4_120_667_001, "ada-lists-standin");
    let second = second_machine(&cloud, &browser, &ada, "ada-second-host");
    let token = cloud.last_token();
    let others: Vec<DeviceId> = served(&cloud, &token)
        .iter()
        .filter_map(|entry| entry["device"].as_str()?.parse().ok())
        .filter(|id| *id != ada.id())
        .collect();
    assert_eq!(others.len(), 1, "the second device is not registered");
    drop(second);

    let listing = list_on_ada(&ada);

    let line = listing.assert_linked(ada.id());
    assert!(line.to_lowercase().contains("client"), "the kind: {line:?}");
    assert!(
        shows_issued(line, ada.cert.body().issued),
        "the date issued: {line:?}"
    );
    listing.assert_apart(others[0], &["ada-second-host"]);
    assert!(
        !listing.apart_lines(others[0])[0]
            .to_lowercase()
            .contains(THIS_DEVICE)
    );
    assert_eq!(listing.warnings(others[0]), Vec::<String>::new());
}

/// A device whose certificate a device of the account issued, chained to the pinned master key, is
/// linked, by its certificate's name, and is not this device.
#[test]
fn device_issued_by_a_linked_device_is_linked() {
    let (cloud, _browser, ada) = setup(4_120_667_002, "ada-issues-standin");
    let linked = MadeUp::new();
    let body = linked.body(ada.account, Some(ada.id()), "linked-by-ada");
    let cert = linked.cert(&body, &ada.key);
    add(&cloud, vec![], vec![entry(linked.id(), &cert)]);

    let listing = list_on_ada(&ada);

    let line = listing.assert_linked(linked.id());
    assert!(line.contains("linked-by-ada"), "{line:?}");
    assert!(!line.to_lowercase().contains(THIS_DEVICE), "{line:?}");
}

/// A certificate signed only by the server, whose key is not the master key nor a linked device's,
/// is rejected: with no issuer and with Ada's device named as its issuer, its device is listed
/// apart, by ID only, and Ada's device stays linked.
#[test]
fn certificate_signed_only_by_the_server_is_rejected() {
    let (cloud, _browser, ada) = setup(4_120_667_003, "ada-server-signed-standin");
    let server = SigningKey::generate();
    let alone = MadeUp::new();
    let alone_cert = alone.cert(&alone.body(ada.account, None, "server-made-one"), &server);
    let posing = MadeUp::new();
    let posing_cert = posing.cert(
        &posing.body(ada.account, Some(ada.id()), "server-made-two"),
        &server,
    );
    add(
        &cloud,
        vec![entry(alone.id(), &alone_cert)],
        vec![entry(posing.id(), &posing_cert)],
    );

    let listing = list_on_ada(&ada);

    let names = ["server-made-one", "server-made-two"];
    listing.assert_apart(alone.id(), &names);
    listing.assert_apart(posing.id(), &names);
}

/// A certificate chained to another master key, directly or through a device it issued, leaves its
/// device off the linked ones.
#[test]
fn certificate_chained_to_another_master_key_is_not_linked() {
    let (cloud, _browser, ada) = setup(4_120_667_004, "ada-other-master-standin");
    let other_master = SigningKey::generate();
    let issuer = MadeUp::new();
    let issuer_cert = issuer.cert(
        &issuer.body(ada.account, None, "other-master-one"),
        &other_master,
    );
    let issued = MadeUp::new();
    let issued_cert = issued.cert(
        &issued.body(ada.account, Some(issuer.id()), "other-master-two"),
        &issuer.key,
    );
    add(
        &cloud,
        vec![],
        vec![
            entry(issued.id(), &issued_cert),
            entry(issuer.id(), &issuer_cert),
        ],
    );

    let listing = list_on_ada(&ada);

    let names = ["other-master-one", "other-master-two"];
    listing.assert_apart(issuer.id(), &names);
    listing.assert_apart(issued.id(), &names);
}

/// A certificate Ada's device signed fails the proof check, and leaves its device off the linked
/// ones, when its `proof` is broken or its `device` is not derived from its `signing` key.
#[test]
fn certificate_that_fails_the_proof_check_is_not_linked() {
    let (cloud, _browser, ada) = setup(4_120_667_005, "ada-broken-proof-standin");
    let broken = MadeUp::new();
    let broken_cert = broken_proof(&broken.cert(
        &broken.body(ada.account, Some(ada.id()), "broken-proof"),
        &ada.key,
    ));
    let underived = MadeUp::new();
    let stranger = MadeUp::new();
    let mut body = underived.body(ada.account, Some(ada.id()), "not-derived");
    body.device = stranger.id();
    let underived_cert = underived.cert(&body, &ada.key);
    add(
        &cloud,
        vec![],
        vec![
            entry(broken.id(), &broken_cert),
            entry(stranger.id(), &underived_cert),
        ],
    );

    let listing = list_on_ada(&ada);

    let names = ["broken-proof", "not-derived"];
    listing.assert_apart(broken.id(), &names);
    listing.assert_apart(stranger.id(), &names);
}

/// A device with two certificates of different bodies, each valid alone, has no valid one: it is
/// listed apart, and the list warns about it.
#[test]
fn device_with_two_bodies_is_not_linked_and_the_list_warns() {
    let (cloud, _browser, ada) = setup(4_120_667_006, "ada-two-bodies-standin");
    let twice = MadeUp::new();
    let one = twice.cert(
        &twice.body(ada.account, Some(ada.id()), "first-body"),
        &ada.key,
    );
    let two = twice.cert(
        &twice.body(ada.account, Some(ada.id()), "second-body"),
        &ada.key,
    );
    add(
        &cloud,
        vec![],
        vec![entry(twice.id(), &one), entry(twice.id(), &two)],
    );

    let listing = list_on_ada(&ada);

    listing.assert_apart(twice.id(), &["first-body", "second-body"]);
    assert_eq!(listing.warnings(twice.id()).len(), 1, "{listing:?}");
}

/// Beside Ada's genuine certificate, and before it, a copy of it with a broken `signature` and a
/// second body that fails the proof check count for nothing: her device stays linked, listed once,
/// by its genuine name, with no warning.
#[test]
fn broken_copies_beside_the_genuine_certificate_leave_it_linked() {
    let (cloud, _browser, ada) = setup(4_120_667_007, "ada-broken-copies-standin");
    let copy = broken_signature(&ada.cert);
    let mut body = ada.cert.body().clone();
    body.name = "unproved-body".to_owned();
    body.issued = MADE_UP_ISSUED;
    let unproved = broken_proof(&issue(&body, &SigningKey::generate(), &ada.key).expect("a cert"));
    add(
        &cloud,
        vec![entry(ada.id(), &copy), entry(ada.id(), &unproved)],
        vec![],
    );

    let listing = list_on_ada(&ada);

    listing.assert_hidden(&["unproved-body"]);
}

/// The server gives another master key and a device that key issued: the list checks against the
/// master key the machine pinned, so Ada's device stays linked and the other one is listed apart.
#[test]
fn master_key_the_server_swaps_changes_nothing() {
    let (cloud, _browser, ada) = setup(4_120_667_008, "ada-swapped-master-standin");
    let swapped = SigningKey::generate();
    let master = URL_SAFE_NO_PAD.encode(swapped.public_key());
    cloud
        .relay
        .rewrite(ASK_MASTER, move |_| json!({ "master": master }).to_string());
    let device = MadeUp::new();
    let cert = device.cert(&device.body(ada.account, None, "swapped-master"), &swapped);
    add(&cloud, vec![], vec![entry(device.id(), &cert)]);

    let listing = list_on_ada(&ada);

    listing.assert_apart(device.id(), &["swapped-master"]);
}

/// Names come only from valid certificates: names the server gives with its devices never show,
/// neither for Ada's device nor for one without a certificate, nor the name its registration had.
#[test]
fn names_come_only_from_certificates() {
    let (cloud, browser, ada) = setup(4_120_667_009, "ada-names-standin");
    drop(second_machine(
        &cloud,
        &browser,
        &ada,
        "ada-registered-name",
    ));
    let token = cloud.last_token();
    let second: DeviceId = served(&cloud, &token)
        .iter()
        .filter_map(|entry| entry["device"].as_str()?.parse().ok())
        .find(|id| *id != ada.id())
        .expect("the second device");
    tamper(&cloud, |devices| {
        for (at, device) in devices.iter_mut().enumerate() {
            device["name"] = json!(format!("server-says-{at}"));
        }
    });

    let listing = list_on_ada(&ada);

    listing.assert_apart(second, &["server-says-", "ada-registered-name"]);
}

/// On a device without a certificate, the list says it is not linked yet and lists no device as
/// linked, not even the account's first one.
#[test]
fn device_without_a_certificate_says_it_is_not_linked_yet() {
    let (cloud, browser, ada) = setup(4_120_667_010, "ada-not-linked-standin");
    let second = second_machine(&cloud, &browser, &ada, "ada-waiting");

    let listing = list(&second);

    assert!(
        listing.shown().to_lowercase().contains(NOT_LINKED),
        "{listing:?}"
    );
    assert_eq!(listing.linked, Vec::<String>::new(), "{listing:?}");
    assert!(!listing.shown().contains(ada.name), "{listing:?}");
}

/// Two linked devices with one name, Ada's and one she issued: both are listed as linked, and the
/// list warns once, naming both.
#[test]
fn linked_devices_with_the_same_name_get_a_warning() {
    let (cloud, _browser, ada) = setup(4_120_667_011, "ada-same-name-standin");
    let namesake = MadeUp::new();
    let cert = namesake.cert(
        &namesake.body(ada.account, Some(ada.id()), ada.name),
        &ada.key,
    );
    add(&cloud, vec![], vec![entry(namesake.id(), &cert)]);

    let listing = list(&ada.machine);

    assert!(listing.finished.success, "{listing:?}");
    listing.assert_linked(ada.id());
    listing.assert_linked(namesake.id());
    let warnings = listing.warnings(ada.id());
    assert_eq!(warnings.len(), 1, "{listing:?}");
    assert!(
        warnings[0].contains(&namesake.id().to_string()) && warnings[0].contains(ada.name),
        "{listing:?}"
    );
}
