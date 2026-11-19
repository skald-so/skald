//! The token alone, without its device's proof, as a session alone for a `client` device: it
//! reads the space's revisions, devices and revocations, its grant, and the GroupInfo and history
//! of its token's channels, and uploads its own device's KeyPackages. It is refused every other
//! request, each account request and other devices' KeyPackages included, and changes nothing.

use serde_json::{Value, json};
use skald_crypto::{MlsDevice, SigningKey, StateKey};
use skald_proto::DeviceKind;

use crate::api::{Person, assert_refused};
use crate::bot::{alone, ops, token};
use crate::common::device_flow::{json, text};
use crate::common::devices::{DEVICES, Device, assert_taken, b64, devices_of};
use crate::common::hex;
use crate::rosters::next;
use crate::tokens::{RECEIVE, SEND, Stage, bearer, call, creation, grant_of, tokens_path};
use crate::wire::now;

/// The KeyPackages of one device: `POST` uploads, `GET` counts.
fn key_packages_path(device: skald_proto::DeviceId) -> String {
    format!("/devices/{device}/key-packages")
}

/// Hands out KeyPackages: `POST`.
const HAND_OUT: &str = "/key-packages";

/// The upload of `packages`.
fn upload_of(packages: &[Vec<u8>]) -> Value {
    let packages: Vec<String> = packages.iter().map(|bytes| b64(bytes)).collect();
    json!({ "key_packages": packages })
}

/// `device`'s MLS client.
fn mls(device: &Device) -> MlsDevice {
    MlsDevice::new(
        &SigningKey::from_bytes(&device.signing.secret),
        &StateKey::generate(),
    )
}

/// Whether the database holds `bytes`.
fn holds(stage: &Stage, bytes: &[u8]) -> bool {
    stage.db().rows().concat().contains(&hex(bytes))
}

/// Writes a `client` device of alice's account certified by her laptop, and stores the laptop's
/// revocation of it; gives it.
fn revoked_tablet(stage: &Stage) -> Device {
    let alice = &stage.f.team.alice;
    let tablet = alice.certified_by(stage.db(), &alice.laptop, DeviceKind::Client, "Tablet");
    alice.revoke(&alice.by_device(&alice.laptop, tablet.id));
    tablet
}

/// `DELETE <path>` with `token`.
fn deleted(stage: &Stage, path: &str, token: &str) -> crate::common::Response {
    let bearer = bearer(token);
    call(
        stage.base(),
        "delete",
        path,
        &[("authorization", &bearer)],
        None,
    )
}

/// The devices `person`'s session lists for its account.
fn account_devices(person: &Person) -> std::collections::BTreeMap<String, Option<Vec<u8>>> {
    devices_of(&person.session.call("get", DEVICES, None))
}

/// The token alone reads `acme`'s revisions, its devices and revocations as alice's session reads
/// them, and its grant; another space's revisions and devices, and every read of the account or
/// of the space beyond those, are refused.
#[test]
fn the_token_alone_reads_the_spaces_revisions_devices_and_revocations_and_no_account_data() {
    let stage = Stage::new(3_545_500_001);
    revoked_tablet(&stage);
    let alice = &stage.f.team.alice;
    let space = stage.space();
    let grant = stage.bot_grant();
    let created = stage.create_with(&grant);
    let alone = alone(&stage, &created.token);

    assert_eq!(alone.revisions(space), stage.f.revisions, "revisions");
    assert_eq!(
        alone.devices(space),
        alice.session.devices(space),
        "devices"
    );
    let revocations = alice.session.space_revocations(space);
    assert_eq!(revocations.len(), 1, "the tablet's revocation");
    assert_eq!(alone.space_revocations(space), revocations, "revocations");
    let own = stage.own(&created.token);
    assert_eq!(own.status, 200, "{}", text(&own));
    assert_eq!(grant_of(&json(&own)), grant, "its grant");

    let (globex, _) = stage.other_channel();
    assert_refused(&alone.revisions_response(globex), "globex's revisions");
    assert_refused(&alone.devices_response(globex), "globex's devices");
    let reads = [
        "/session".to_owned(),
        "/spaces".to_owned(),
        DEVICES.to_owned(),
        "/account/devices".to_owned(),
        "/account/master".to_owned(),
        "/account/anchors".to_owned(),
        "/account/usage".to_owned(),
        "/account/sign-in-methods".to_owned(),
        "/account/sessions".to_owned(),
        "/account/links/waiting".to_owned(),
        format!("/spaces/{space}/members"),
        format!("/spaces/{space}/usage"),
        tokens_path(space),
    ];
    for path in &reads {
        assert_refused(&alone.call("get", path, None), path);
    }
}

/// The token alone asks to store the next revision of `acme`, which alice's laptop signed, to
/// create a space, a channel ID and the personal space, to register a device, to store a
/// revocation, to create, list and revoke tokens, and to end its session: each is refused, and
/// none changes what alice's session or the token itself reads.
#[test]
fn the_token_alone_is_refused_every_account_request_and_changes_nothing() {
    let stage = Stage::new(3_545_500_011);
    let alice = &stage.f.team.alice;
    let space = stage.space();
    let created = token(&stage, &[SEND, RECEIVE]);
    let alone = alone(&stage, &created.token);
    let spaces = alice.session.spaces();
    let tablet = alice.certified_by(stage.db(), &alice.laptop, DeviceKind::Client, "Tablet");
    let newcomer = Device::new("Newcomer");
    let last = stage.f.revisions.last().expect("a revision");
    let revision = alice.sign(&next(last, &alice.laptop));
    let own = format!("{}/{}", tokens_path(space), created.id);
    let another = creation(stage.bot.id, &stage.bot_grant(), &[SEND]);

    let refused = [
        ("a revision", alone.store(space, &revision)),
        ("a space", alone.create("other")),
        ("a channel ID", alone.channel_response(space)),
        (
            "the personal space",
            alone.call("post", "/account/personal-space", None),
        ),
        (
            "a registration",
            alone.call("post", DEVICES, Some(&newcomer.registration())),
        ),
        (
            "a revocation",
            alone.revoke(&alice.by_device(&alice.laptop, tablet.id)),
        ),
        (
            "a token",
            alone.call("post", &tokens_path(space), Some(&another)),
        ),
        ("its own revocation", deleted(&stage, &own, &created.token)),
        ("signing out", deleted(&stage, "/session", &created.token)),
    ];
    for (what, response) in &refused {
        assert_refused(response, what);
    }

    assert_eq!(alone.revisions(space), stage.f.revisions, "revisions");
    assert!(stage.works(&created.token), "the token");
    assert_eq!(stage.listed(alice), vec![created.id.clone()], "tokens");
    assert_eq!(alice.session.spaces(), spaces, "alice's spaces");
    let devices = account_devices(alice);
    assert!(
        !devices.contains_key(&newcomer.id.to_string()),
        "registered: {devices:?}"
    );
    assert_eq!(
        alice.session.revocations(),
        Vec::<Vec<u8>>::new(),
        "revocations"
    );
}

/// The token alone reads `general`'s GroupInfo, entries, links and wraps as alice's session reads
/// them; those of `ops`, a channel its grant does not list, are refused.
#[test]
fn the_token_alone_reads_the_group_info_and_history_of_its_channels_only() {
    let mut stage = Stage::new(3_545_500_021);
    let ops = ops(&mut stage);
    let created = token(&stage, &[SEND, RECEIVE]);
    let alone = alone(&stage, &created.token);
    let alice = &stage.f.team.alice;
    let space = stage.space();
    let reads = [
        "group-info",
        "history/entries",
        "history/links",
        "history/wraps",
    ];

    for read in reads {
        let path = format!("/spaces/{space}/channels/{}/{read}", stage.f.general);
        let response = alone.call("get", &path, None);
        assert_eq!(response.status, 200, "{read}: {}", text(&response));
        let expected = alice.session.call("get", &path, None);
        assert_eq!(json(&response), json(&expected), "{read}");
    }
    for read in reads {
        let path = format!("/spaces/{space}/channels/{ops}/{read}");
        assert_refused(&alone.call("get", &path, None), &format!("ops's {read}"));
    }
}

/// The token alone uploads KeyPackages of the bot, which the server takes and keeps. It may not
/// upload or count those of alice's laptop, nor have one handed out: the laptop's KeyPackage that
/// alice uploaded stays for bob.
#[test]
fn the_token_alone_uploads_its_own_devices_key_packages_and_gets_no_other_devices() {
    let stage = Stage::new(3_545_500_031);
    let created = token(&stage, &[SEND, RECEIVE]);
    let alone = alone(&stage, &created.token);
    let t = &stage.f.team;
    let laptop = t.alice.laptop.id;

    let mut bot = mls(&stage.bot);
    let packages: Vec<Vec<u8>> = (0..3)
        .map(|_| bot.key_package(now()).expect("a KeyPackage").message)
        .collect();
    let path = key_packages_path(stage.bot.id);
    assert_taken(&alone.call("post", &path, Some(&upload_of(&packages))));
    for package in &packages {
        assert!(holds(&stage, package), "the bot's KeyPackage kept");
    }

    let mut alices = mls(&t.alice.laptop);
    let kept = alices.key_package(now()).expect("a KeyPackage").message;
    let laptops = key_packages_path(laptop);
    let upload = upload_of(std::slice::from_ref(&kept));
    assert_taken(&t.alice.session.call("post", &laptops, Some(&upload)));
    let other = alices.key_package(now()).expect("a KeyPackage").message;
    let upload = upload_of(std::slice::from_ref(&other));
    assert_refused(
        &alone.call("post", &laptops, Some(&upload)),
        "an upload for alice's laptop",
    );
    assert!(!holds(&stage, &other), "alice's laptop's KeyPackage kept");
    assert_refused(
        &alone.call("get", &laptops, None),
        "the count of alice's laptop",
    );
    let asked = json!({ "devices": [laptop.to_string()] });
    assert_refused(
        &alone.call("post", HAND_OUT, Some(&asked)),
        "alice's laptop's KeyPackage handed out",
    );

    let response = t.bob.session.call("post", HAND_OUT, Some(&asked));
    assert_eq!(response.status, 200, "{}", text(&response));
    let handed = json(&response);
    let listed: Vec<(&Value, &Value)> = handed["key_packages"]
        .as_array()
        .unwrap_or_else(|| panic!("no key_packages in {handed}"))
        .iter()
        .map(|entry| (&entry["device"], &entry["key_package"]))
        .collect();
    let expected = (json!(laptop.to_string()), json!(b64(&kept)));
    assert_eq!(
        listed,
        vec![(&expected.0, &expected.1)],
        "bob gets the laptop's KeyPackage"
    );
}
