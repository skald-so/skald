//! Accepting roster revisions in the client: before a command uses a space, the device fetches the
//! space's revisions and the devices of its accounts and accepts the new revisions in order with
//! checks 1 to 4 of `docs/protocol/membership.md`, "Roster". A revision that fails, and every
//! revision built on it, has no effect: the device keeps the last revision it accepted, warns and
//! keeps working with it. Another valid revision with a number it accepted is a forked roster, and
//! an accepted revision that changes a member's `master` is warned of ("Device checks").
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay, which rewrites the revisions and devices the server gives, as a compromised
//! server would; a test plays the person on the verification page in headless Chrome. Each machine
//! keeps its state in a directory of its own and its secrets in a file there. The tests sign
//! fixture revisions with the keys of the machines' devices, read from their secrets files, and
//! store the valid ones on the server through the API, as an admin's device would.
//!
//! The CLI, as these tests use it, beside `skald create-space <name>` and `skald spaces`:
//! - `skald members <space>`: shows the members of the space named `<space>` as its last accepted
//!   revision lists them, after accepting the revisions the server has: one line per member holding
//!   its handle and its role (`admin` or `member`), and, for each member, a line per device of its
//!   account with a valid certificate, holding the device's ID and its certificate's `name`; a device
//!   without one is not shown by its certificate's name. It succeeds while the device keeps working
//!   with the space, also after a revision fails;
//! - a warning is a line saying `warning`, on either output, of any command that uses the space: for
//!   a revision that fails; for a forked roster, a line that also says `fork`; for a changed
//!   `master`, a line that also says `master` and holds the member's handle.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;

#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
mod world;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cloud::Cloud;
use machine::{Finished, Machine};
use serde_json::{Value, json};
use skald_crypto::{RosterError, SigningKey, accept_next, check_roster, issue};
use skald_proto::{
    AccountId, Cbor, Cert, CertBody, DeviceId, DeviceKind, Member, Name, Role, Roster, SpaceId,
    Unknown,
};
use world::{
    ACME, ADMIN, MEMBER, MEMBERS, Person, SPACES, World, assert_role, create, holds_word, members,
    next, next_by, now, plain_lines, revisions_request, run, set_role, space_certs, store,
    warnings, words,
};

/// What the warnings of a forked roster and of a changed `master` also say.
const FORK: &str = "fork";
const MASTER: &str = "master";

/// The handles of the accounts that forged revisions add; no account of a test has them.
const MALLORY: &str = "mallory";
const TRUDY: &str = "trudy";

/// A space beside [`ACME`].
const GLOBEX: &str = "globex";

/// A member a forged revision adds: a made-up account with a master key of its own.
fn made_up_member(handle: &str) -> Member {
    Member {
        account: AccountId::generate(),
        handle: handle.parse().expect("a valid handle"),
        role: Role::Admin,
        master: SigningKey::generate().public_key(),
        invite: None,
        excluded: None,
        unknown: Unknown::default(),
    }
}

/// `roster` with its signature broken.
fn broken_signature(roster: &Roster) -> Roster {
    let mut signature = *roster.signature();
    signature[0] ^= 1;
    Roster::from_parts(roster.body_bytes().to_vec(), signature).expect("a revision")
}

/// The request for the devices of `space`'s accounts, as its line starts.
fn devices_request(space: SpaceId) -> String {
    format!("GET /spaces/{space}/devices")
}

/// From now on, the proxy gives the machines `revisions` as `space`'s revisions, whatever the server
/// holds.
fn serve_revisions(cloud: &Cloud, space: SpaceId, revisions: &[&Roster]) {
    let list: Vec<String> = revisions
        .iter()
        .map(|roster| URL_SAFE_NO_PAD.encode(roster.to_cbor()))
        .collect();
    cloud.relay.rewrite(&revisions_request(space), move |_| {
        json!({ "revisions": list }).to_string()
    });
}

/// From now on, the proxy gives the machines `space`'s revisions as the server holds them.
fn serve_stored_revisions(cloud: &Cloud, space: SpaceId) {
    cloud
        .relay
        .rewrite(&revisions_request(space), |body| body.to_owned());
}

/// From now on, the proxy adds `entries` to the devices the server gives for `space`.
fn add_devices(cloud: &Cloud, space: SpaceId, entries: Vec<Value>) {
    cloud.relay.rewrite(&devices_request(space), move |body| {
        let mut list: Value = serde_json::from_str(body).expect("the server's device list");
        let devices = list["devices"].as_array_mut().expect("a device list");
        devices.extend(entries.iter().cloned());
        list.to_string()
    });
}

/// A device entry of a device list, as the server gives it.
fn entry(device: DeviceId, cert: &Cert) -> Value {
    json!({ "device": device.to_string(), "cert": URL_SAFE_NO_PAD.encode(cert.to_cbor()) })
}

/// A device a test makes up on `account`, named `name`, with a certificate signed by a key of its
/// own instead of the account's master key, as a server would sign it: the device, its signing key
/// and its certificate.
fn certified_by_the_server(account: AccountId, name: &str) -> (DeviceId, SigningKey, Cert) {
    let key = SigningKey::generate();
    let device = DeviceId::from_signing_key(&key.public_key());
    let body = CertBody {
        account,
        device,
        kind: DeviceKind::Client,
        signing: key.public_key(),
        hpke: SigningKey::generate().public_key(),
        issuer: None,
        issued: now(),
        ssh: None,
        name: name.to_owned(),
        unknown: Unknown::default(),
    };
    let cert = issue(&body, &SigningKey::generate(), &key).expect("a certificate");
    (device, key, cert)
}

/// Fails unless `shown` shows `device` on a line with its certificate's name `name`.
fn assert_device(shown: &Finished, device: DeviceId, name: &str) {
    let id = device.to_string();
    assert!(
        plain_lines(shown)
            .iter()
            .any(|line| line.contains(&id) && words(line).contains(&name)),
        "{name} ({id}) is not shown as a device with a valid certificate: {shown:?}"
    );
}

/// Fails if a line of `shown` but its warnings holds one of `words`.
fn assert_hidden(shown: &Finished, hidden: &[&str]) {
    let lines = plain_lines(shown);
    for word in hidden {
        assert!(!holds_word(&lines, word), "{word} is shown: {shown:?}");
    }
}

/// Fails unless `machine`'s space list shows the space `name` with the role `role` and `handle`.
fn assert_listed(machine: &Machine, name: &str, role: &str, handle: &Name) {
    let listed = run(machine, &[SPACES]);
    assert!(listed.success, "{listed:?}");
    let lines: Vec<String> = plain_lines(&listed)
        .into_iter()
        .filter(|line| words(line).contains(&name))
        .collect();
    assert_eq!(lines.len(), 1, "{name} is not listed once: {listed:?}");
    let line = words(&lines[0]);
    assert!(line.contains(&role), "{name} not as {role}: {listed:?}");
    assert!(
        line.contains(&handle.as_str()),
        "{name} not with the handle {handle}: {listed:?}"
    );
}

/// A space whose revision 1 Ada's machine created and whose revision 2, signed by Ada's device and
/// stored on the server, adds Bob as an admin and Carol, if any, as a member.
struct Space {
    first: Roster,
    second: Roster,
}

impl Space {
    fn id(&self) -> SpaceId {
        self.first.body().space
    }
}

/// Ada's machine creates `acme`, and revision 2 adds Bob as an admin and `carol` as a member.
fn space(cloud: &Cloud, ada: &Person, bob: &Person, carol: Option<&Person>) -> Space {
    let first = create(cloud, ada, ACME);
    let second = next_by(&first, ada, |members| {
        members.push(bob.member(Role::Admin));
        if let Some(carol) = carol {
            members.push(carol.member(Role::Member));
        }
    });
    store(cloud, &ada.token, &second);
    Space { first, second }
}

/// A revision by a device of another admin account is accepted after the one that made the account
/// an admin, both new to the device in one run: the members follow the latest revision, each with
/// its handle and role and its account's device with a valid certificate, and so does the space
/// list, while another space stays at its own revision.
#[test]
fn revision_by_a_device_of_another_admin_account_is_accepted() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_681_001, "ada-follows", "ada-workstation");
    let bob = world.sign_up(4_120_681_002, "bob-signs", "bob-workstation");
    let carol = world.sign_up(4_120_681_003, "carol-joins", "carol-laptop");
    create(cloud, &ada, GLOBEX);
    let acme = space(cloud, &ada, &bob, Some(&carol));
    let third = next_by(&acme.second, &bob, |members| {
        set_role(members, ada.account, Role::Member);
    });
    store(cloud, &bob.token, &third);

    let shown = members(&ada.machine, ACME);

    assert_eq!(warnings(&shown), Vec::<String>::new(), "{shown:?}");
    assert_role(&shown, &ada.handle, MEMBER);
    assert_role(&shown, &bob.handle, ADMIN);
    assert_role(&shown, &carol.handle, MEMBER);
    for person in [&ada, &bob, &carol] {
        assert_device(&shown, person.device, person.host);
    }
    assert_listed(&ada.machine, ACME, MEMBER, &ada.handle);
    assert_listed(&ada.machine, GLOBEX, ADMIN, &ada.handle);
}

/// A device of a member's account without a valid certificate, such as one only the server
/// certified, is not shown by its certificate's name; the members' genuine devices are.
#[test]
fn members_show_only_devices_with_a_valid_certificate() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_681_101, "ada-lists", "ada-workstation");
    let bob = world.sign_up(4_120_681_102, "bob-listed", "bob-workstation");
    let acme = space(cloud, &ada, &bob, None);
    let (device, _, forged) = certified_by_the_server(bob.account, "bob-forged-laptop");
    add_devices(cloud, acme.id(), vec![entry(device, &forged)]);

    let shown = members(&ada.machine, ACME);

    assert_role(&shown, &ada.handle, ADMIN);
    assert_role(&shown, &bob.handle, ADMIN);
    assert_device(&shown, ada.device, ada.host);
    assert_device(&shown, bob.device, bob.host);
    assert_hidden(&shown, &["bob-forged-laptop"]);
}

/// How a forged revision 3 fails check 1 of "Roster".
enum Forged {
    /// Signed by the device of Carol, a member that is no admin.
    NoAdmin,
    /// Signed by a device on Bob's admin account that only the server certified.
    ServerCertified,
    /// Signed by Bob's device, with its signature broken.
    BrokenSignature,
}

/// The device follows `acme` to revision 2, then the proxy gives it a forged revision 3, which
/// demotes Ada and adds Mallory, failing as `forged` says, and a revision 4 built on it, validly
/// signed by Bob's device, which adds Trudy. Neither has an effect: the device warns, and keeps
/// working with revision 2, in its members and its space list. Once the server gives its own
/// revisions again, a genuine revision 3 by Bob's device is accepted.
fn forged_revision_has_no_effect(forged: Forged, ids: [u64; 3]) {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(ids[0], "ada-keeps", "ada-workstation");
    let bob = world.sign_up(ids[1], "bob-trusted", "bob-workstation");
    let carol = world.sign_up(ids[2], "carol-plain", "carol-laptop");
    let acme = space(cloud, &ada, &bob, Some(&carol));
    let before = members(&ada.machine, ACME);
    assert_role(&before, &bob.handle, ADMIN);
    assert_role(&before, &carol.handle, MEMBER);

    let change = |members: &mut Vec<Member>| {
        set_role(members, ada.account, Role::Member);
        members.push(made_up_member(MALLORY));
    };
    let mut certs = space_certs(cloud, &ada.token, acme.id());
    let third = match forged {
        Forged::NoAdmin => next_by(&acme.second, &carol, change),
        Forged::ServerCertified => {
            let (device, key, cert) = certified_by_the_server(bob.account, "bob-forged-laptop");
            add_devices(cloud, acme.id(), vec![entry(device, &cert)]);
            certs.push(cert);
            next(&acme.second, (device, &key), change)
        }
        Forged::BrokenSignature => broken_signature(&next_by(&acme.second, &bob, change)),
    };
    let fourth = next_by(&third, &bob, |members| members.push(made_up_member(TRUDY)));
    // The forged revision fails check 1 and passes checks 3 and 4; the one built on it is valid.
    let accepts = |previous: &Roster, next: &Roster| {
        accept_next(previous, next, &certs, ada.account, &ada.master)
    };
    assert!(
        matches!(
            accepts(&acme.second, &third),
            Err(RosterError::Signer(_) | RosterError::Signature)
        ),
        "{:?}",
        accepts(&acme.second, &third)
    );
    assert_eq!(check_roster(&third, &certs), Ok(()));
    assert_eq!(accepts(&third, &fourth), Ok(()));
    serve_revisions(
        cloud,
        acme.id(),
        &[&acme.first, &acme.second, &third, &fourth],
    );

    let shown = members(&ada.machine, ACME);

    assert!(!warnings(&shown).is_empty(), "no warning: {shown:?}");
    assert_role(&shown, &ada.handle, ADMIN);
    assert_role(&shown, &bob.handle, ADMIN);
    assert_role(&shown, &carol.handle, MEMBER);
    assert_hidden(&shown, &[MALLORY, TRUDY]);
    assert_listed(&ada.machine, ACME, ADMIN, &ada.handle);
    let again = members(&ada.machine, ACME);
    assert_role(&again, &ada.handle, ADMIN);
    assert_hidden(&again, &[MALLORY, TRUDY]);

    serve_stored_revisions(cloud, acme.id());
    let genuine = next_by(&acme.second, &bob, |members| {
        set_role(members, carol.account, Role::Admin);
    });
    store(cloud, &bob.token, &genuine);
    let after = members(&ada.machine, ACME);
    assert_role(&after, &carol.handle, ADMIN);
    assert_role(&after, &ada.handle, ADMIN);
    assert_hidden(&after, &[MALLORY, TRUDY]);
}

#[test]
fn revision_signed_by_a_device_of_an_account_that_is_no_admin_has_no_effect() {
    forged_revision_has_no_effect(
        Forged::NoAdmin,
        [4_120_681_201, 4_120_681_202, 4_120_681_203],
    );
}

#[test]
fn revision_signed_by_a_device_only_the_server_certified_has_no_effect() {
    forged_revision_has_no_effect(
        Forged::ServerCertified,
        [4_120_681_301, 4_120_681_302, 4_120_681_303],
    );
}

#[test]
fn revision_with_a_broken_signature_has_no_effect() {
    forged_revision_has_no_effect(
        Forged::BrokenSignature,
        [4_120_681_401, 4_120_681_402, 4_120_681_403],
    );
}

/// After the device accepted revision 2, the proxy gives it another valid revision 2, signed by
/// Ada's device, which adds Mallory, and a revision 3 built on it, which adds Trudy: the device
/// warns of a forked roster and keeps its own revision 2.
#[test]
fn forked_roster_is_warned_of_and_the_accepted_revision_kept() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_681_501, "ada-forked", "ada-workstation");
    let bob = world.sign_up(4_120_681_502, "bob-kept", "bob-workstation");
    let acme = space(cloud, &ada, &bob, None);
    let before = members(&ada.machine, ACME);
    assert_role(&before, &bob.handle, ADMIN);

    let other = next_by(&acme.first, &ada, |members| {
        members.push(made_up_member(MALLORY));
    });
    let built = next_by(&other, &ada, |members| members.push(made_up_member(TRUDY)));
    // Both are valid: the other revision 2 after revision 1, and the one built on it after it.
    let certs = space_certs(cloud, &ada.token, acme.id());
    let accepts = |previous: &Roster, next: &Roster| {
        accept_next(previous, next, &certs, ada.account, &ada.master)
    };
    assert_eq!(accepts(&acme.first, &other), Ok(()));
    assert_eq!(accepts(&other, &built), Ok(()));
    assert_ne!(other.hash(), acme.second.hash(), "the same revision 2");
    serve_revisions(cloud, acme.id(), &[&acme.first, &other, &built]);

    let shown = members(&ada.machine, ACME);

    assert!(
        warnings(&shown).iter().any(|line| line.contains(FORK)),
        "no warning of a fork: {shown:?}"
    );
    assert_role(&shown, &ada.handle, ADMIN);
    assert_role(&shown, &bob.handle, ADMIN);
    assert_hidden(&shown, &[MALLORY, TRUDY]);
}

/// An accepted revision that changes a member's `master` is warned of, naming the member; it takes
/// effect, and the member's devices are checked against the new key, which none of them chains to.
#[test]
fn changed_master_of_a_member_is_warned_of() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_681_601, "ada-warned", "ada-workstation");
    let bob = world.sign_up(4_120_681_602, "bob-recovers", "bob-workstation");
    let acme = space(cloud, &ada, &bob, None);
    let before = members(&ada.machine, ACME);
    assert!(
        !warnings(&before).iter().any(|line| line.contains(MASTER)),
        "a master key is warned of: {before:?}"
    );
    assert_device(&before, bob.device, bob.host);

    let third = next_by(&acme.second, &bob, |members| {
        set_role(members, ada.account, Role::Member);
        members
            .iter_mut()
            .find(|member| member.account == bob.account)
            .expect("Bob")
            .master = SigningKey::generate().public_key();
    });
    store(cloud, &bob.token, &third);
    let certs = space_certs(cloud, &ada.token, acme.id());
    assert_eq!(
        accept_next(&acme.second, &third, &certs, ada.account, &ada.master),
        Ok(())
    );

    let shown = members(&ada.machine, ACME);

    let handle = bob.handle.as_str();
    assert!(
        warnings(&shown)
            .iter()
            .any(|line| line.contains(MASTER) && words(line).contains(&handle)),
        "no warning of {handle}'s master key: {shown:?}"
    );
    assert_role(&shown, &ada.handle, MEMBER);
    assert_role(&shown, &bob.handle, ADMIN);
    assert_hidden(&shown, &[bob.host]);
    assert_listed(&ada.machine, ACME, MEMBER, &ada.handle);
}

/// A device follows only the spaces it has an anchor for: Bob's device, an admin in `acme`, which
/// it did not create, follows its own `globex` but not `acme`.
#[test]
fn device_follows_only_spaces_it_has_an_anchor_for() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_681_701, "ada-anchors", "ada-workstation");
    let bob = world.sign_up(4_120_681_702, "bob-unanchored", "bob-workstation");
    space(cloud, &ada, &bob, None);
    create(cloud, &bob, GLOBEX);

    let own = members(&bob.machine, GLOBEX);
    assert_role(&own, &bob.handle, ADMIN);
    assert_listed(&bob.machine, GLOBEX, ADMIN, &bob.handle);

    let listed = run(&bob.machine, &[SPACES]);
    assert!(listed.success, "{listed:?}");
    assert!(
        !holds_word(&plain_lines(&listed), ACME),
        "acme is listed: {listed:?}"
    );
    let other = run(&bob.machine, &[MEMBERS, ACME]);
    assert!(
        !holds_word(&plain_lines(&other), ada.handle.as_str()),
        "acme's members are shown: {other:?}"
    );
}

/// `skald members` gives the warnings of `skald devices` for each member's devices: of a device on
/// Bob's account with two certificates of different bodies, naming the device, and of Bob's linked
/// devices with one name, naming both.
#[test]
fn members_warn_of_two_bodies_and_shared_names() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_120_681_801, "ada-checks", "ada-workstation");
    let bob = world.sign_up(4_120_681_802, "bob-doubles", "bob-workstation");
    let acme = space(cloud, &ada, &bob, None);
    // Issued by Bob's device, each a valid certificate alone.
    let made_up = |name: &str| {
        let key = SigningKey::generate();
        let device = DeviceId::from_signing_key(&key.public_key());
        let body = CertBody {
            account: bob.account,
            device,
            kind: DeviceKind::Client,
            signing: key.public_key(),
            hpke: SigningKey::generate().public_key(),
            issuer: Some(bob.device),
            issued: now(),
            ssh: None,
            name: name.to_owned(),
            unknown: Unknown::default(),
        };
        (key, body)
    };
    let (twice_key, mut body) = made_up("bob-first-body");
    let twice = body.device;
    let one = issue(&body, &bob.key, &twice_key).expect("a certificate");
    body.name = "bob-second-body".to_owned();
    let two = issue(&body, &bob.key, &twice_key).expect("a certificate");
    let (namesake_key, body) = made_up(bob.host);
    let namesake = body.device;
    let same_name = issue(&body, &bob.key, &namesake_key).expect("a certificate");
    add_devices(
        cloud,
        acme.id(),
        vec![
            entry(twice, &one),
            entry(twice, &two),
            entry(namesake, &same_name),
        ],
    );

    let shown = members(&ada.machine, ACME);

    let id = |device: DeviceId| device.to_string().to_lowercase();
    let warned = warnings(&shown);
    let of = |device: DeviceId| -> Vec<&String> {
        warned
            .iter()
            .filter(|line| line.contains(&id(device)))
            .collect()
    };
    assert_eq!(of(twice).len(), 1, "no warning of two bodies: {shown:?}");
    let shared = of(namesake);
    assert_eq!(shared.len(), 1, "no warning of a shared name: {shown:?}");
    assert!(
        shared[0].contains(&id(bob.device)) && shared[0].contains(bob.host),
        "the warning of a shared name names not both devices: {shown:?}"
    );
    assert_device(&shown, bob.device, bob.host);
    assert_device(&shown, namesake, bob.host);
    assert_hidden(&shown, &["bob-first-body", "bob-second-body"]);
}
