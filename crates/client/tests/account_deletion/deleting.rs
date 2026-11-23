//! Deleting the account at a terminal: the command lists the spaces that go, says what goes 30 days
//! later, takes the typed username, signs in again when the sign-in is old, and once the server has
//! deleted the account, the machine keeps no key or state of it.

use skald_github_stand_in::User;

use crate::cloud::Cloud;
use crate::deletion::{
    AtGrant, DELETE_REQUEST, Held, LATER, PERSONAL, assert_deleted, assert_keeps_nothing,
    delete_at_terminal, deleted_lines, hand_over, make_sign_in_old, marked_deleted, shared_with,
};
use crate::fixtures::fronted;
use crate::keys;
use crate::linking::lines_holding;
use crate::machine::{KEY_STORAGE, Machine};
use crate::system::Entry;
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::{ACME, World, create, handle_of};

/// A team space of Ada's alone, which her deletion deletes.
const SKETCHBOOK: &str = "sketchbook";

/// Ada's first machine created `acme`, with Bob a member, and handed her admin rights there to
/// Bob; it also created `sketchbook`, of Ada alone. Her sign-in is moved back past the server's
/// setting. `skald delete-account` at a terminal lists her personal space, as personal, and
/// `sketchbook` as deleted, says what goes 30 days later and takes her username; it signs in
/// again once she approves its code on the verification page, and the server deletes her account:
/// her sessions, the old and the new, are refused, Bob's session gets her marked deleted in
/// `acme`, which stays, and the run says her account is deleted. Her machine's state directory
/// then holds no local state and no file with her session tokens, her device's private keys, her
/// account's or device's ID or her username.
#[test]
fn after_a_fresh_sign_in_and_the_typed_username_the_account_goes_and_the_machine_keeps_nothing() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_001_001, "ada-deletes-herself", ADA_HOST);
    let bob = world.sign_up(6_120_001_002, "bob-stays-on", BOB_HOST);
    let space = shared_with(cloud, &ada, &bob);
    hand_over(&ada, &bob);
    create(cloud, &ada, SKETCHBOOK);
    let held = Held::of(cloud, &ada);
    make_sign_in_old(cloud, &ada.token);
    let since = cloud.relay.mark();

    let deletion = delete_at_terminal(
        &world,
        &ada.machine,
        ada.handle.as_str(),
        AtGrant::Approve(&ada.user),
    );

    let text = &deletion.shown.text;
    assert!(deletion.shown.success, "{text}");
    assert_eq!(deletion.grants, 1, "not one sign-in again: {text}");
    assert!(
        deletion.asked >= 1,
        "it did not ask for the username: {text}"
    );
    let personal: Vec<String> = (lines_holding(text, ada.handle.as_str()).into_iter())
        .filter(|line| line.to_lowercase().contains(PERSONAL))
        .collect();
    assert!(
        !personal.is_empty(),
        "the personal space is not listed: {text}"
    );
    assert!(
        !lines_holding(text, SKETCHBOOK).is_empty(),
        "{SKETCHBOOK} is not listed: {text}"
    );
    assert!(text.contains(LATER), "nothing said of {LATER}: {text}");
    assert!(
        !deleted_lines(text).is_empty(),
        "it does not say the account is deleted: {text}"
    );
    assert!(
        cloud.count_sent(since, DELETE_REQUEST) > 0,
        "no request deleted the account"
    );
    assert_deleted(cloud, &ada);
    let fresh = cloud.last_token();
    assert_ne!(cloud.session_of(&fresh).0, 200, "the new session works");
    assert!(
        marked_deleted(cloud, &bob, space, &ada),
        "Bob's session does not get Ada marked deleted"
    );
    assert!(
        cloud.spaces(&bob.token).iter().any(|(id, _)| *id == space),
        "{ACME} is gone"
    );
    assert_keeps_nothing(cloud, &ada, &held);
}

/// The signing and HPKE public keys of the only device of the account of the session `token`.
fn device_keys(cloud: &Cloud, token: &str) -> ([u8; 32], [u8; 32]) {
    let devices = cloud.devices(token);
    assert_eq!(devices.len(), 1, "the account's devices: {devices:?}");
    let cert = devices[0].cert.clone().expect("the device's certificate");
    (cert.body().signing, cert.body().hpke)
}

/// Signs `machine` in as the first device of `user`'s account; gives its session token.
fn signed_in(world: &World, machine: &Machine, user: &User) -> String {
    let signed_in = world
        .cloud
        .signed_in(&world.browser.page(), machine, user, &[]);
    assert!(signed_in.success, "{signed_in:?}");
    world.cloud.last_token()
}

/// With the system's key storage, only with `SKALD_TEST_REAL_KEY_STORAGE=1`: Ada's machine keeps
/// its secrets in the key storage's entry of the machine; once `skald delete-account` at a terminal
/// has deleted her account, the entry holds no session token and neither of the device's private
/// keys, or is gone. The test deletes the entry.
#[test]
fn with_the_systems_key_storage_the_entry_keeps_nothing_once_the_account_is_deleted() {
    if std::env::var("SKALD_TEST_REAL_KEY_STORAGE").as_deref() != Ok("1") {
        return;
    }
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = cloud.user(6_120_001_101, "ada-deletes-her-keyring");
    let machine = Machine::new()
        .with_host_name(ADA_HOST)
        .with_env(KEY_STORAGE, "auto");
    let entry = Entry::of(&machine);
    let token = signed_in(&world, &machine, &ada);
    let (signing, hpke) = device_keys(cloud, &token);
    let before = entry.value().expect("the entry holds the secrets");
    assert!(
        keys::private_key(before.as_bytes(), &signing, keys::ed25519).is_some(),
        "the entry does not hold the device's signing key"
    );
    let handle = handle_of(cloud, &token);

    let deletion = delete_at_terminal(&world, &machine, handle.as_str(), AtGrant::Approve(&ada));

    assert!(deletion.shown.success, "{}", deletion.shown.text);
    assert_ne!(cloud.session_of(&token).0, 200, "the session works");
    if let Some(after) = entry.value() {
        let bytes = after.as_bytes();
        for token in cloud.tokens() {
            assert!(!after.contains(&token), "the entry holds a session token");
        }
        assert!(
            keys::private_key(bytes, &signing, keys::ed25519).is_none(),
            "the entry holds the device's signing key"
        );
        assert!(
            keys::private_key(bytes, &hpke, keys::x25519).is_none(),
            "the entry holds the device's HPKE key"
        );
    }
}
