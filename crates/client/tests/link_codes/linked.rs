//! A device linked with a link code: its certificate, issued by the linked device the code was
//! typed on, is valid against the account's master key; the account's devices and those of a
//! member of a space with the account list it, and warn that it was added; it follows the spaces
//! of the linked device, its personal space too; and no request and nothing the server stores holds
//! the code.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use skald_crypto::check_valid_device;
use skald_proto::{Cbor as _, Cert, DeviceKind, Role};

use crate::anchors::held;
use crate::cloud::Cloud;
use crate::linking::{NewDevice, forms, holds, link};
use crate::machine::{DEVICES, NOT_LINKED};
use crate::world::{
    ACME, GLOBEX, Person, WARNING, World, create, lines_naming, members, next_by, plain_lines, run,
    space_certs, space_id, spaces, store, warnings,
};

/// The host name of Ada's new machine, its certificate's `name`.
const LAPTOP: &str = "ada-laptop";

/// `skald devices` on `machine`; fails unless it succeeds.
fn devices(person: &Person) -> crate::machine::Finished {
    let shown = run(&person.machine, &[DEVICES]);
    assert!(shown.success, "{shown:?}");
    shown
}

/// The certificates of the devices of the account of the session `token`, from the server.
fn certs(cloud: &Cloud, token: &str) -> Vec<Cert> {
    cloud
        .devices(token)
        .into_iter()
        .filter_map(|device| device.cert)
        .collect()
}

/// Fails unless the anchors object of `new`, a device of `first`'s account, is at the revisions of
/// `first`'s and pins the same personal space.
#[track_caller]
fn assert_starts_as_first(cloud: &Cloud, first: &Person, new: &Person) {
    let objects = held(cloud, &first.token);
    let of = |device| {
        objects
            .iter()
            .find(|object| object.device == device)
            .unwrap_or_else(|| panic!("no anchors object of {device}: {objects:?}"))
    };
    let (by_first, by_new) = (of(first.device), of(new.device));
    assert_eq!(
        by_new.anchors, by_first.anchors,
        "the revisions it starts from"
    );
    assert_eq!(by_new.personal, by_first.personal, "the personal space");
    assert!(by_first.personal.is_some(), "{objects:?}");
}

/// Ada's account follows `globex`, which she created, and her personal space; Bob created `acme`
/// and added her. Ada's new machine signs in at a terminal and is linked with its code on her
/// first machine. Its certificate is issued by her first device for the new one's keys and name,
/// and is valid against her master key, as her account and Bob, a member of `acme`, see it. Her
/// first device, her new one and Bob's list the new device, and the first one and Bob's warn that
/// it was added. The new device follows her spaces as the first one does, the personal one marked
/// so, and keeps an anchors object at the revisions the first one accepted. The relay saw no form
/// of the code on any request, and the server's database holds none.
#[test]
fn a_device_linked_with_a_code_is_certified_by_the_linked_one_and_follows_its_spaces() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_101_001, "ada-links-a-laptop", "ada-workstation");
    let bob = world.sign_up(4_175_101_002, "bob-sees-the-laptop", "bob-workstation");
    create(cloud, &ada, GLOBEX);
    let second = create(cloud, &bob, ACME);
    let third = next_by(&second, &bob, |list| list.push(ada.member(Role::Member)));
    store(cloud, &bob.token, &third);
    let acme = space_id(cloud, &bob.token, ACME);
    // What each device knew of Ada's devices before the link.
    let on_ada = spaces(&ada.machine);
    devices(&ada);
    members(&bob.machine, ACME);

    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    let codes = crate::linking::codes_in(&new.shown());
    let linking = link(cloud, &ada, &mut new);
    let laptop = new.into_person(cloud, &ada);

    // The certificate.
    let held_certs = certs(cloud, &ada.token);
    let cert = check_valid_device(&ada.master, &held_certs, &[], laptop.device)
        .expect("the new device's certificate is valid against the master key");
    let body = cert.body();
    assert_eq!(body.issuer, Some(ada.device), "issued by the first device");
    assert_eq!(
        (body.account, body.kind, body.name.as_str()),
        (ada.account, DeviceKind::Client, LAPTOP)
    );
    let by_bob = space_certs(cloud, &bob.token, acme);
    assert!(
        check_valid_device(&ada.master, &by_bob, &[], laptop.device).is_ok(),
        "as a member of acme sees it"
    );

    // The devices that list it, and warn of it.
    let id = laptop.device.to_string().to_lowercase();
    let on_first = devices(&ada);
    assert!(!lines_naming(&on_first, LAPTOP).is_empty(), "{on_first:?}");
    // Its run of skald link syncs its spaces to add the new device, or a later run does.
    let warned_linking = (linking.text.lines().map(str::to_lowercase))
        .any(|line| line.contains(WARNING) && line.contains(&id));
    assert!(
        warned_linking || warnings(&on_first).iter().any(|line| line.contains(&id)),
        "the first device does not warn of the new one: {}, {on_first:?}",
        linking.text
    );
    let on_laptop = devices(&laptop);
    assert!(
        !on_laptop.shown().to_lowercase().contains(NOT_LINKED),
        "{on_laptop:?}"
    );
    for host in [LAPTOP, ada.host] {
        assert!(
            !lines_naming(&on_laptop, host).is_empty(),
            "{host}: {on_laptop:?}"
        );
    }
    let on_bob = members(&bob.machine, ACME);
    assert!(!lines_naming(&on_bob, LAPTOP).is_empty(), "{on_bob:?}");
    assert!(
        warnings(&on_bob).iter().any(|line| line.contains(&id)),
        "bob's device does not warn of ada's new one: {on_bob:?}"
    );

    // The spaces it follows, and its anchors.
    let on_new = spaces(&laptop.machine);
    assert_eq!(plain_lines(&on_new), plain_lines(&on_ada), "{on_new:?}");
    assert_starts_as_first(cloud, &ada, &laptop);

    // The code.
    assert!(!codes.is_empty(), "no code shown");
    let sent = cloud.relay.sent_bytes();
    let stored = cloud.rows().join("\n").into_bytes();
    for code in &codes {
        for form in forms(code) {
            assert!(!holds(&sent, &form), "a request holds the code as {form:?}");
            assert!(
                !holds(&stored, &form),
                "the server holds the code as {form:?}"
            );
        }
    }
}

/// A new device linked with the name of a linked device of the account: both are listed, and the
/// account's devices warn of two valid devices with one name, naming both.
#[test]
fn a_new_device_with_a_linked_ones_name_gets_a_warning_naming_both() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_101_011, "ada-names-twice", "ada-workstation");

    let mut new = NewDevice::start(&world, &ada, ada.host);
    link(cloud, &ada, &mut new);
    let twin = new.into_person(cloud, &ada);

    let on_first = devices(&ada);
    let named: Vec<String> = warnings(&on_first)
        .into_iter()
        .filter(|line| line.contains(&twin.device.to_string().to_lowercase()))
        .filter(|line| line.contains(&ada.device.to_string().to_lowercase()))
        .collect();
    assert_eq!(named.len(), 1, "one warning naming both: {on_first:?}");
}

/// Ada's first machine follows `globex`, which she created. As her new machine starts from its link
/// object, the relay gives it, before the server's revisions of `globex`, a revision 3 Ada's device
/// signed that the link does not name. The new device starts from the revision the link names: its
/// anchors object is at the revisions her first device accepted.
#[test]
fn the_new_device_starts_only_from_the_revisions_its_link_names() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_101_031, "ada-gets-other-revisions", "ada-workstation");
    let second = create(cloud, &ada, GLOBEX);
    let other = URL_SAFE_NO_PAD.encode(next_by(&second, &ada, |_| {}).to_cbor());
    spaces(&ada.machine);

    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    let revisions = format!("GET /spaces/{}/revisions", second.body().space);
    cloud.relay.rewrite_once(&revisions, move |body| {
        let mut answer: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        if let Some(listed) = answer["revisions"].as_array_mut() {
            listed.insert(0, Value::from(other.as_str()));
        }
        answer.to_string()
    });
    link(cloud, &ada, &mut new);
    let laptop = new.into_person(cloud, &ada);
    spaces(&laptop.machine);
    assert_starts_as_first(cloud, &ada, &laptop);
}
