//! `skald sign-out`: the machine ends its session on the server and deletes its session token, and
//! its device stays, with its keys, its certificate and its local state, for its next sign-in.
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay, with a test playing the person on the verification page in headless
//! Chrome. Each machine keeps its state in a directory of its own and its secrets in a file
//! there, never in the system's key storage.
//!
//! The CLI, as these tests use it:
//! - `skald sign-out`: ends the session of the machine, on the server of its local state, and
//!   says `signed out`;
//! - signed out, `skald devices` fails and asks to sign in, naming `skald sign-in`;
//! - `skald sign-in` again resumes the machine's device on the account it belongs to; approved by
//!   another GitHub user, it fails, ends the session it got and keeps the device as it was.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

use std::fs;
use std::path::PathBuf;

use browser::Browser;
use cloud::{Cloud, Device, GIVE_MASTER, REGISTER, certified, ids};
use machine::{
    DEVICES, FIRST_DEVICE, Finished, Machine, NO, NOT_LINKED, SERVER, SIGN_IN, SIGN_OUT,
    THIS_DEVICE, YES,
};
use skald_github_stand_in::User;
use skald_proto::{Cert, DeviceId};

/// What sign-out says once done.
const SIGNED_OUT: &str = "signed out";
/// The command the device list names when it asks to sign in.
const ASKS_TO_SIGN_IN: &str = "skald sign-in";

/// The paths a machine reads with its session.
const READS: [&str; 3] = ["/session", "/devices", "/account/master"];

/// The account's first machine, signed in with a certificate by the account's master key.
struct Ada {
    user: User,
    machine: Machine,
    /// Its session token.
    token: String,
    /// The account's master public key.
    master: [u8; 32],
    id: DeviceId,
    cert: Cert,
    /// Its signing and HPKE private keys, as its secrets hold them.
    signing: [u8; 32],
    hpke: [u8; 32],
}

impl Ada {
    /// The account's devices as the server gives them while only Ada's machine is on it.
    fn alone(&self) -> Vec<Device> {
        vec![Device {
            id: self.id,
            cert: Some(self.cert.clone()),
        }]
    }
}

/// A server whose codes live for 2 minutes and poll every second, a browser for the person and the
/// account's first machine, signed in as `user`, a GitHub ID and login.
fn setup(user: (u64, &str)) -> (Cloud, Browser, Ada) {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let ada = cloud.user(user.0, user.1);
    let (machine, signed_in) = cloud.first_machine(&browser.page(), &ada, "ada-workstation");
    assert!(signed_in.success, "{signed_in:?}");

    let token = token_of(&cloud, &machine).expect("a session token");
    let master = cloud.master(&token).expect("the account's master key");
    let mut devices = cloud.devices(&token);
    assert_eq!(devices.len(), 1, "{devices:?}");
    let first = devices.remove(0);
    let cert = first.cert.expect("the first device's certificate");
    skald_crypto::check_device(&master, std::slice::from_ref(&cert), first.id)
        .expect("a valid certificate");
    let (signing, hpke) = private_keys(&machine, &cert).expect("the device's private keys");
    let ada = Ada {
        user: ada,
        machine,
        token,
        master,
        id: first.id,
        cert,
        signing,
        hpke,
    };
    (cloud, browser, ada)
}

/// A second machine of Ada's account, signed in and waiting for linking, and its session token.
fn second_machine(cloud: &Cloud, browser: &Browser, ada: &Ada) -> (Machine, String) {
    let second = Machine::new().with_host_name("ada-second");
    cloud.signed_in(&browser.page(), &second, &ada.user, &[FIRST_DEVICE, NO]);
    let token = token_of(cloud, &second).expect("the second machine's session");
    assert_ne!(token, ada.token);
    (second, token)
}

/// Runs `skald sign-out` on `machine`; fails unless it succeeds and says it signed out.
fn sign_out(machine: &Machine) -> Finished {
    let finished = machine.start(&[SIGN_OUT]).finish();
    assert!(finished.success, "{finished:?}");
    assert!(
        finished.shown().to_lowercase().contains(SIGNED_OUT),
        "{finished:?}"
    );
    finished
}

/// Runs `skald devices` on `machine`.
fn list(machine: &Machine) -> Finished {
    machine.start(&[DEVICES]).finish()
}

/// Fails unless `skald devices` on `machine` fails and asks to sign in.
fn assert_asks_to_sign_in(machine: &Machine) {
    let listing = list(machine);
    assert!(!listing.success, "{listing:?}");
    assert!(listing.shown().contains(ASKS_TO_SIGN_IN), "{listing:?}");
}

/// Fails unless `skald devices` on `machine` succeeds and lists `device` as linked, marked as this
/// device.
fn assert_lists_itself_as_linked(machine: &Machine, device: DeviceId) {
    let listing = list(machine);
    assert!(listing.success, "{listing:?}");
    let id = device.to_string();
    let lines: Vec<&str> = listing
        .stdout
        .lines()
        .filter(|line| line.contains(&id))
        .collect();
    assert_eq!(lines.len(), 1, "{listing:?}");
    assert!(lines[0].to_lowercase().contains(THIS_DEVICE), "{listing:?}");
    let above = listing.stdout.split(lines[0]).next().unwrap_or_default();
    assert!(
        !above.to_lowercase().contains(NOT_LINKED),
        "listed as not linked: {listing:?}"
    );
}

/// The session token the server gave that a file of `machine` holds; `None` if none does. Fails if
/// several files or tokens are found.
fn token_of(cloud: &Cloud, machine: &Machine) -> Option<String> {
    let mut found = cloud.token_files(machine);
    assert!(found.len() <= 1, "files holding a session token: {found:?}");
    found.pop().map(|(_, token)| token)
}

/// The files of `machine` but its local state, which holds no secret: where its secrets are.
fn secret_files(machine: &Machine) -> Vec<PathBuf> {
    let state = machine.sqlite_files();
    let mut files = machine.files();
    files.retain(|file| !state.contains(file));
    files
}

/// The signing and HPKE private keys of `cert` that the secrets of `machine` hold.
fn private_keys(machine: &Machine, cert: &Cert) -> Option<([u8; 32], [u8; 32])> {
    let body = cert.body();
    let mut signing = None;
    let mut hpke = None;
    for file in secret_files(machine) {
        let bytes = fs::read(&file).expect("read a machine's file");
        signing = signing.or_else(|| keys::private_key(&bytes, &body.signing, keys::ed25519));
        hpke = hpke.or_else(|| keys::private_key(&bytes, &body.hpke, keys::x25519));
    }
    Some((signing?, hpke?))
}

/// The device of `devices` whose signing private key the secrets of `machine` hold.
fn device_of(machine: &Machine, devices: &[Device]) -> DeviceId {
    let listed = ids(devices);
    let mut found = Vec::new();
    for file in secret_files(machine) {
        let bytes = fs::read(&file).expect("read a machine's file");
        for public in keys::public_keys(&bytes, keys::ed25519) {
            let id = DeviceId::from_signing_key(&public);
            if listed.contains(&id) && !found.contains(&id) {
                found.push(id);
            }
        }
    }
    assert_eq!(found.len(), 1, "the machine's device among {devices:?}");
    found[0]
}

/// Fails unless the local state of Ada's machine still pins the master key and holds the
/// certificate, and its secrets still hold the device's private keys.
fn assert_keeps_the_device(ada: &Ada) {
    let state = ada.machine.state_bytes();
    assert!(keys::holds(&state, &ada.master), "the pinned master key");
    for part in [
        ada.cert.body_bytes(),
        ada.cert.signature(),
        ada.cert.proof(),
    ] {
        assert!(keys::holds(&state, part), "the certificate is not kept");
    }
    assert_eq!(
        private_keys(&ada.machine, &ada.cert),
        Some((ada.signing, ada.hpke)),
        "the device's private keys"
    );
}

/// Fails unless the server refuses `token` on each request a machine reads with its session.
fn assert_refused(cloud: &Cloud, token: &str) {
    for path in READS {
        let (status, body) = cloud.get_as(path, token);
        assert_eq!(status, 401, "GET {path} with an ended session: {body}");
    }
}

/// Signs Ada's machine in again, signed out before, with her account; fails unless it resumes its
/// device: a new session of her account, her old one still refused, no master key sent, the
/// account's one device with its certificate as before, kept on the machine and listed as linked.
fn sign_in_again(cloud: &Cloud, browser: &Browser, ada: &Ada) {
    let since = cloud.relay.mark();
    let again = cloud.signed_in(
        &browser.page(),
        &ada.machine,
        &ada.user,
        &[FIRST_DEVICE, YES],
    );
    assert!(again.success, "{again:?}");
    assert!(again.shown().contains(&ada.user.login), "{again:?}");
    let token = token_of(cloud, &ada.machine).expect("a new session token");
    assert_ne!(token, ada.token);
    assert_eq!(cloud.login_of(&token), ada.user.login);
    assert_refused(cloud, &ada.token);
    assert_eq!(cloud.count_sent(since, GIVE_MASTER), 0, "a master key sent");
    assert_eq!(cloud.master(&token), Some(ada.master));
    let devices = cloud.devices(&token);
    assert_eq!(devices, ada.alone(), "a new device or certificate");
    assert_eq!(device_of(&ada.machine, &devices), ada.id);
    assert_keeps_the_device(ada);
    assert_lists_itself_as_linked(&ada.machine, ada.id);
}

/// After `skald sign-out`, the server refuses the old token at once, no file of the machine holds
/// it, and the device list asks to sign in; the device's keys, its certificate and the pinned
/// master key stay on the machine, and the device stays on the account.
#[test]
fn sign_out_ends_the_session_on_the_server_and_keeps_the_device() {
    let (cloud, browser, ada) = setup((4_120_668_001, "ada-signs-out-standin"));

    let finished = sign_out(&ada.machine);

    assert_refused(&cloud, &ada.token);
    assert_eq!(
        ada.machine.files_holding(&ada.token),
        Vec::<PathBuf>::new(),
        "the token is kept"
    );
    assert!(
        !finished.shown().contains(&ada.token),
        "the token was shown"
    );
    assert_keeps_the_device(&ada);
    assert_asks_to_sign_in(&ada.machine);
    let (_second, token) = second_machine(&cloud, &browser, &ada);
    let devices = cloud.devices(&token);
    assert_eq!(certified(&devices), vec![ada.id], "{devices:?}");
    assert_eq!(cloud.master(&token), Some(ada.master));
}

/// Signing one machine out leaves the account's other machine signed in: the server takes its
/// session, it keeps its token, and run again it finds its session.
#[test]
fn signing_one_machine_out_leaves_the_accounts_other_sessions_working() {
    let (cloud, browser, ada) = setup((4_120_668_002, "ada-other-sessions-standin"));
    let (second, second_token) = second_machine(&cloud, &browser, &ada);

    sign_out(&ada.machine);

    assert_refused(&cloud, &ada.token);
    assert_eq!(cloud.login_of(&second_token), ada.user.login);
    for path in READS {
        let (status, body) = cloud.get_as(path, &second_token);
        assert_eq!(status, 200, "GET {path}: {body}");
    }
    assert_eq!(token_of(&cloud, &second), Some(second_token.clone()));

    let grants = cloud.grants().len();
    let mark = cloud.relay.mark();
    let again = second
        .start(&[SIGN_IN, SERVER, &cloud.url, FIRST_DEVICE, NO])
        .finish();
    assert!(again.shown().contains(&ada.user.login), "{again:?}");
    assert_eq!(cloud.grants().len(), grants, "a new code: {again:?}");
    let bearer = format!("\nauthorization: bearer {}", second_token.to_lowercase());
    assert!(
        cloud
            .relay
            .sent_since(mark)
            .iter()
            .any(|request| request.head.contains(&bearer)),
        "its session token was not sent"
    );
}

/// Signing in again with the same account resumes the device: the same device ID and certificate,
/// no new device or master key on the server, and the device list shows it linked.
#[test]
fn signing_in_again_resumes_the_device_with_its_certificate() {
    let (cloud, browser, ada) = setup((4_120_668_003, "ada-signs-in-again-standin"));
    sign_out(&ada.machine);

    sign_in_again(&cloud, &browser, &ada);
}

/// A device waiting for linking, signed out and in again with the same account, stays the same
/// device without a certificate: the server gets no new device or master key.
#[test]
fn device_waiting_for_linking_resumes_without_a_certificate() {
    let (cloud, browser, ada) = setup((4_120_668_004, "ada-waiting-again-standin"));
    let (second, second_token) = second_machine(&cloud, &browser, &ada);
    let before = cloud.devices(&ada.token);
    assert_eq!(before.len(), 2, "{before:?}");
    let id = device_of(&second, &before);

    sign_out(&second);
    assert_refused(&cloud, &second_token);
    let mark = cloud.relay.mark();
    cloud.signed_in(&browser.page(), &second, &ada.user, &[FIRST_DEVICE, NO]);

    let token = token_of(&cloud, &second).expect("a new session token");
    assert_ne!(token, second_token);
    assert_eq!(cloud.login_of(&token), ada.user.login);
    assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "a master key sent");
    assert_eq!(cloud.master(&ada.token), Some(ada.master));
    let after = cloud.devices(&ada.token);
    assert_eq!(after, before, "a new device or certificate");
    assert_eq!(device_of(&second, &after), id);
    assert_eq!(certified(&after), vec![ada.id]);
}

/// Signed out and then approved by another GitHub user, the sign-in is refused: the server refuses
/// the session it got, the machine keeps no token of it, and the device is neither registered nor
/// certified on the other account. The device keeps its account and certificate: signed in again
/// with its own account, it resumes as before.
#[test]
fn signing_in_with_another_account_is_refused_and_the_device_keeps_its_own() {
    let (cloud, browser, ada) = setup((4_120_668_005, "ada-keeps-account-standin"));
    let bob = cloud.user(4_120_668_006, "bob-other-account-standin");
    sign_out(&ada.machine);
    let mark = cloud.relay.mark();
    let tokens = cloud.tokens().len();

    let refused = cloud.signed_in(&browser.page(), &ada.machine, &bob, &[FIRST_DEVICE, YES]);

    assert!(!refused.success, "{refused:?}");
    let given = cloud.tokens();
    assert_eq!(given.len(), tokens + 1, "the session the machine got");
    let bob_token = given.last().cloned().expect("a session token");
    assert_refused(&cloud, &bob_token);
    assert_eq!(token_of(&cloud, &ada.machine), None, "a token is kept");
    assert!(!refused.shown().contains(&bob_token), "the token was shown");
    assert_eq!(cloud.count_sent(mark, REGISTER), 0, "registered on Bob's");
    assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "a master key sent");
    assert_keeps_the_device(&ada);
    assert_asks_to_sign_in(&ada.machine);

    sign_in_again(&cloud, &browser, &ada);
}
