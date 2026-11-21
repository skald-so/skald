//! The spaces a device does not follow from its account's anchors objects: one whose anchors are
//! off one chain, with a warning; one only an anchors object of a revoked device, of another
//! account's device, of a `browser` or `service` device or with a broken signature names. With the
//! anchors objects withheld, it follows nothing new and keeps working. Each test has Ada's first machine create `globex` after her
//! second machine was linked, which the second machine follows in the same run.

use skald_proto::{Anchors, DeviceKind, Role, Roster};

use crate::fixtures::{Issuer, add_device_of_kind};
use crate::joined::{
    Account, GIVE_ANCHORS, Withheld, anchored, anchors_object, anchors_off_one_chain, follows,
    give_anchors_too, next_run, warn_of, with_broken_signature,
};
use crate::kit::create_space;
use crate::revocations::revoked_by;
use crate::talk::BOB_HOST;
use crate::world::{ACME, GLOBEX, Person, create, next_by, store};

/// Her first machine creates `globex`, and `acme`'s two anchors are off one chain
/// ([`anchors_off_one_chain`]). The second machine's next run follows `globex`, does not follow
/// `acme`, and warns of `acme`.
#[test]
fn space_whose_anchors_are_off_one_chain_is_not_followed_and_warned_of() {
    let account = Account::new(4_811_406_201, "ada-sees-a-forked-anchor", &[]);
    create_space(account.cloud(), &account.ada, GLOBEX);
    let acme = anchors_off_one_chain(&account);

    let (listed, said) = next_run(&account.laptop.machine);

    assert!(
        follows(&listed, GLOBEX),
        "globex is not followed: {listed:?}"
    );
    assert!(!follows(&listed, ACME), "acme is followed: {listed:?}");
    assert!(
        warn_of(&said, (ACME, acme)),
        "no warning of acme: {said:?} {listed:?}"
    );
}

/// Ada's account, whose first machine has created `globex`, and Bob, GitHub user `bob` with its
/// login, whose machine creates `acme`; its revision 3, signed by Bob's device, adds Ada as a
/// member, so `acme` is among her account's spaces but no anchors object of hers names it. From
/// then on the relay also gives the anchors object `forge` makes of the account, Bob and revision
/// 3. The second machine's next run follows `globex` and not `acme`, and its anchors object names
/// no `acme`.
#[track_caller]
fn assert_not_followed(
    (id, login): (u64, &str),
    bob: (u64, &str),
    forge: impl FnOnce(&Account, &Person, &Roster) -> Vec<Anchors>,
) {
    let account = Account::new(id, login, &[]);
    let (cloud, ada) = (account.cloud(), &account.ada);
    create_space(cloud, ada, GLOBEX);
    let bob = account.world.sign_up(bob.0, bob.1, BOB_HOST);
    let added = ada.member(Role::Member);
    let third = next_by(&create(cloud, &bob, ACME), &bob, move |list| {
        list.push(added)
    });
    store(cloud, &bob.token, &third);
    let acme = third.body().space;
    give_anchors_too(cloud, &forge(&account, &bob, &third));

    let (listed, _) = next_run(&account.laptop.machine);

    assert!(
        follows(&listed, GLOBEX),
        "globex is not followed: {listed:?}"
    );
    assert!(!follows(&listed, ACME), "acme is followed: {listed:?}");
    assert_eq!(
        anchored(cloud, &account.laptop, acme),
        None,
        "the second device's anchors object names acme"
    );
}

/// Ada's first device revokes her third device; the relay gives an anchors object of the third
/// device, signed with its key, naming `acme`, as the server keeps a revoked device's last one.
#[test]
fn space_only_a_revoked_device_anchors_is_not_followed() {
    assert_not_followed(
        (4_811_406_301, "ada-revoked-her-tablet"),
        (4_811_406_302, "bob-shares-with-ada"),
        |account, _, third| {
            let tablet = account.tablet();
            revoked_by(account.cloud(), &account.ada, tablet.device);
            let space = third.body().space;
            let key = (tablet.device, &tablet.key);
            vec![anchors_object(account.ada.account, key, &[(space, third)])]
        },
    );
}

/// The relay gives two anchors objects by Bob's device, signed with its key, naming `acme`: one of
/// Ada's account and one of Bob's.
#[test]
fn space_only_another_account_s_device_anchors_is_not_followed() {
    assert_not_followed(
        (4_811_406_401, "ada-ignores-bob-anchors"),
        (4_811_406_402, "bob-forges-ada-anchors"),
        |account, bob, third| {
            let space = third.body().space;
            let key = (bob.device, &bob.key);
            vec![
                anchors_object(account.ada.account, key, &[(space, third)]),
                anchors_object(bob.account, key, &[(space, third)]),
            ]
        },
    );
}

/// A device of Ada's account of `kind`, with a valid certificate her first device issued, is
/// written to the server's database; the relay gives its anchors object, signed with its key,
/// naming `acme`.
#[track_caller]
fn assert_not_followed_by_kind(ada: (u64, &str), bob: (u64, &str), kind: DeviceKind) {
    assert_not_followed(ada, bob, |account, _, third| {
        let (cloud, ada) = (account.cloud(), &account.ada);
        let issuer = Issuer::Device(ada);
        let device = add_device_of_kind(cloud, (ada.account, kind), "ada-other", &issuer);
        let key = (device.id, &device.signing);
        vec![anchors_object(
            ada.account,
            key,
            &[(third.body().space, third)],
        )]
    });
}

/// [`assert_not_followed_by_kind`], a `browser` device.
#[test]
fn space_only_a_browser_device_anchors_is_not_followed() {
    assert_not_followed_by_kind(
        (4_811_406_701, "ada-ignores-browser-anchors"),
        (4_811_406_702, "bob-shares-past-the-browser"),
        DeviceKind::Browser,
    );
}

/// [`assert_not_followed_by_kind`], a `service` device.
#[test]
fn space_only_a_service_device_anchors_is_not_followed() {
    assert_not_followed_by_kind(
        (4_811_406_801, "ada-ignores-service-anchors"),
        (4_811_406_802, "bob-shares-past-the-service"),
        DeviceKind::Service,
    );
}

/// The relay gives an anchors object of Ada's third device, valid, naming `acme`, with its
/// signature broken.
#[test]
fn space_only_an_anchors_object_with_a_broken_signature_names_is_not_followed() {
    assert_not_followed(
        (4_811_406_501, "ada-sees-a-broken-anchor"),
        (4_811_406_502, "bob-beside-the-breakage"),
        |account, _, third| {
            let tablet = account.tablet();
            let space = third.body().space;
            let key = (tablet.device, &tablet.key);
            let signed = anchors_object(account.ada.account, key, &[(space, third)]);
            vec![with_broken_signature(&signed)]
        },
    );
}

/// Ada's first machine creates `globex`. While the relay refuses the account's anchors objects
/// with `503 Service Unavailable`, and then while it withholds them, the second machine's runs
/// succeed, still follow her personal space and do not follow `globex`. Once the relay gives them
/// again, its next run follows `globex`.
#[test]
fn with_the_anchors_objects_withheld_the_device_follows_nothing_new_and_keeps_working() {
    let account = Account::new(4_811_406_601, "ada-gets-no-anchors", &[]);
    let (cloud, ada, laptop) = (account.cloud(), &account.ada, &account.laptop);
    create_space(cloud, ada, GLOBEX);
    let personal = ada.handle.to_string();

    cloud.relay.refuse(GIVE_ANCHORS, "503 Service Unavailable");
    let (refused, _) = next_run(&laptop.machine);
    cloud.relay.stop_refusing();
    let withheld = Withheld::start(cloud);
    let (emptied, _) = next_run(&laptop.machine);

    for listed in [&refused, &emptied] {
        assert!(follows(listed, &personal), "no personal space: {listed:?}");
        assert!(!follows(listed, GLOBEX), "globex is followed: {listed:?}");
    }
    withheld.stop();
    let (given, _) = next_run(&laptop.machine);
    assert!(follows(&given, GLOBEX), "globex is not followed: {given:?}");
}
