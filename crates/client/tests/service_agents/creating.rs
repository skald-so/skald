//! An admin creates a service agent: the other members see its entry and its device's certificate
//! as valid, its token reads its grant, the key secret holds what the service needs, and the
//! admin's machine keeps nothing of either secret.

use std::fs;

use skald_crypto::accept_next;
use skald_proto::Roster;

use crate::fixtures::fronted;
use crate::keys::{self, ed25519, x25519};
use crate::kit::{
    Acme, BOT, DEFAULT_DAYS, RECEIVE, SEND, Secrets, assert_certified_by, assert_grant,
    assert_record, create_agent, device_session_names, entry_of, grant_of, own_record, rosters,
    secrets_of, service_of, sign_up_on,
};
use crate::world::{ACME, Person, space_certs};

/// Fails unless no file of `person`'s machine, its local state, key storage and logs, holds the key
/// secret, the token or the service device's private keys, `seeds`, in any encoding.
#[track_caller]
fn assert_nothing_kept(person: &Person, secrets: &Secrets, seeds: &[[u8; 32]]) {
    for (what, secret) in [("key secret", &secrets.key), ("token", &secrets.token)] {
        assert_eq!(
            person.machine.files_holding(secret),
            Vec::<std::path::PathBuf>::new(),
            "files that hold the {what}"
        );
    }
    for file in person.machine.files() {
        let Ok(bytes) = fs::read(&file) else { continue };
        for seed in seeds {
            assert!(
                !keys::holds(&bytes, seed),
                "{} holds a private key of the service device",
                file.display()
            );
        }
    }
}

/// The service device's private keys the key secret holds, its signing key and its HPKE key, whose
/// public keys are those of `cert`; fails unless it holds both.
#[track_caller]
fn private_keys(secrets: &Secrets, signing: &[u8; 32], hpke: &[u8; 32]) -> [[u8; 32]; 2] {
    let bytes = secrets.key.as_bytes();
    let signing = keys::private_key(bytes, signing, ed25519)
        .unwrap_or_else(|| panic!("the key secret does not hold the service device's signing key"));
    let hpke = keys::private_key(bytes, hpke, x25519)
        .unwrap_or_else(|| panic!("the key secret does not hold the service device's HPKE key"));
    [signing, hpke]
}

/// Fails unless the key secret pins `admin`'s master key, the space `acme`, `revision`, the one
/// with the entry, by its hash, and the server's public address `public`.
#[track_caller]
fn assert_pins(secrets: &Secrets, admin: &Person, acme: &Acme, revision: &Roster, public: &str) {
    let bytes = secrets.key.as_bytes();
    assert!(
        keys::holds(bytes, &admin.master),
        "the key secret lacks the master key"
    );
    let space = acme.id.to_string();
    assert!(
        keys::holds(bytes, acme.id.as_bytes()) || keys::holds(bytes, space.as_bytes()),
        "the key secret lacks the space"
    );
    assert!(
        keys::holds(bytes, &revision.hash()),
        "the key secret lacks the revision's hash"
    );
    assert!(
        keys::holds(bytes, public.as_bytes()),
        "the key secret lacks the server's public address {public}"
    );
}

/// The master key of `person` as `roster` lists it.
#[track_caller]
fn master_in(roster: &Roster, person: &Person) -> [u8; 32] {
    roster
        .body()
        .members
        .iter()
        .find(|member| member.account == person.account)
        .unwrap_or_else(|| panic!("{} is no member", person.handle))
        .master
}

/// Ada, an admin of `acme` with Bob as a member, creates `ci/deploy-bot` for the default channel.
/// Stdout is only its key secret and its token. Bob's device accepts the revision with its entry,
/// which Ada's device signed, and finds the service device's certificate valid against Ada's master
/// key in that revision. The token reads its grant: of the service device, for the default
/// channel, signed by Ada's device, expiring in 90 days, and its record has `send` and `receive`.
/// The key secret holds the service device's private keys and pins Ada's master key, `acme`, the
/// revision and the server; no file of Ada's machine holds either secret or those keys.
#[test]
fn admin_creates_a_service_agent_and_gets_its_key_secret_and_token_once() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_310_001, "ada-create-standin", "ada-workstation");
    let bob = world.sign_up(3_135_310_002, "bob-create-standin", "bob-laptop");
    let acme = Acme::with_member(cloud, &ada, &bob);

    let created = create_agent(&ada.machine, ACME, BOT, &[acme.default], &[]);

    let secrets = secrets_of(&created);
    for secret in [&secrets.key, &secrets.token] {
        assert!(
            !created.stderr.contains(secret.as_str()),
            "stderr holds a secret: {created:?}"
        );
    }
    let held = rosters(cloud, &bob.token, acme.id);
    assert_eq!(held.len(), 4, "the space's revisions: {held:?}");
    let (third, fourth) = (&held[2], &held[3]);
    assert_eq!(third, acme.latest(), "Bob's revision is not kept");
    let certs = space_certs(cloud, &bob.token, acme.id);
    accept_next(third, fourth, &certs, bob.account, &bob.master)
        .unwrap_or_else(|err| panic!("Bob's device refuses the revision: {err:?}"));
    assert_eq!(fourth.body().signer, ada.device, "{fourth:?}");
    let (device, cert) = service_of(cloud, &ada);
    assert_eq!(entry_of(fourth, BOT), Some(device), "{fourth:?}");
    assert_eq!(
        master_in(fourth, &ada),
        ada.master,
        "Ada's master key in the revision"
    );
    assert_certified_by(&ada, &certs, device);

    let record = own_record(cloud, &secrets.token).expect("the server refuses the new token");
    assert_record(&record, device, &[SEND, RECEIVE], &[acme.default]);
    assert_grant(
        &grant_of(&record),
        device,
        &[acme.default],
        &ada,
        DEFAULT_DAYS,
    );

    let body = cert.body();
    let seeds = private_keys(&secrets, &body.signing, &body.hpke);
    assert_pins(&secrets, &ada, &acme, fourth, &cloud.public);
    assert_nothing_kept(&ada, &secrets, &seeds);
}

/// Ada's CLI session registered her workstation; creating `ci/deploy-bot` from it registers the
/// service device, and Access still lists every device session of hers under the workstation.
#[test]
fn creating_a_service_agent_leaves_the_cli_session_under_the_admin_s_device() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let page = world.browser.page();
    let host = "ada-workstation";
    let ada = sign_up_on(&world, &page, (3_135_310_101, "ada-access-standin"), host);
    let acme = Acme::create(cloud, &ada);
    let workstation = Some(host.to_owned());
    assert_eq!(
        device_session_names(cloud, &page),
        std::slice::from_ref(&workstation)
    );

    let created = create_agent(&ada.machine, ACME, BOT, &[acme.default], &[]);

    secrets_of(&created);
    let (device, _) = service_of(cloud, &ada);
    let names = device_session_names(cloud, &page);
    assert!(
        !names.is_empty() && names.iter().all(|name| *name == workstation),
        "the device sessions are not all under {host}: {names:?}; the service device is {device}"
    );
}
