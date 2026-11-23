//! What deletes nothing at the terminal: a username other than the account's, and a sign-in again
//! whose code is denied, expires or is approved by another account.

use crate::deletion::{
    AtGrant, DELETE_REQUEST, Deletion, assert_kept, delete_at_terminal, make_sign_in_old,
};
use crate::fixtures::fronted;
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::Person;

/// Fails unless `deletion` showed `grants` device authorizations and exited failing, its output
/// saying `said` if given. Whether it asked for the username first is its own choice.
#[track_caller]
fn assert_refused(deletion: &Deletion, grants: usize, said: Option<&str>) {
    let text = &deletion.shown.text;
    assert!(!deletion.shown.success, "{text}");
    assert_eq!(deletion.grants, grants, "device authorizations: {text}");
    if let Some(said) = said {
        assert!(
            text.to_lowercase().contains(said),
            "it does not say {said}: {text}"
        );
    }
}

/// Ada's username with something added: another name, which she might mistype.
fn mistyped(ada: &Person) -> String {
    format!("{}-x", ada.handle)
}

/// Ada's `skald delete-account` at a terminal, given another username than hers, sends no request
/// that deletes the account and exits failing; her account and her machine's state stay.
#[test]
fn another_username_deletes_nothing() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_002_001, "ada-mistypes-her-name", ADA_HOST);
    let since = cloud.relay.mark();

    let deletion = delete_at_terminal(
        &world,
        &ada.machine,
        &mistyped(&ada),
        AtGrant::Approve(&ada.user),
    );

    let text = &deletion.shown.text;
    assert!(!deletion.shown.success, "{text}");
    assert!(
        deletion.asked >= 1,
        "it did not ask for the username: {text}"
    );
    assert_eq!(
        cloud.count_sent(since, DELETE_REQUEST),
        0,
        "a request to delete the account: {text}"
    );
    assert_kept(cloud, &ada);
}

/// With Ada's sign-in moved back past the server's setting, her `skald delete-account` at a
/// terminal, given her username, shows a device authorization, which she denies: the run exits
/// failing, saying the code was denied, and her account and her machine's state stay.
#[test]
fn a_denied_code_deletes_nothing() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_002_101, "ada-denies-the-code", ADA_HOST);
    make_sign_in_old(cloud, &ada.token);

    let deletion = delete_at_terminal(
        &world,
        &ada.machine,
        ada.handle.as_str(),
        AtGrant::Deny(&ada.user),
    );

    assert_refused(&deletion, 1, Some("denied"));
    assert_kept(cloud, &ada);
}

/// With Ada's sign-in moved back past the server's setting, the device authorization her `skald
/// delete-account` shows expires: the run exits failing, saying the code expired, and her account
/// and her machine's state stay.
#[test]
fn an_expired_code_deletes_nothing() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_002_201, "ada-lets-the-code-expire", ADA_HOST);
    make_sign_in_old(cloud, &ada.token);

    let deletion = delete_at_terminal(&world, &ada.machine, ada.handle.as_str(), AtGrant::Expire);

    assert_refused(&deletion, 1, Some("expire"));
    assert_kept(cloud, &ada);
}

/// With Ada's sign-in moved back past the server's setting, the device authorization her `skald
/// delete-account` shows is approved by Bob, signed in as himself: the run exits failing, and
/// neither account is deleted; Ada's machine still runs as her device.
#[test]
fn a_code_approved_by_another_account_deletes_nothing() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_002_301, "ada-asks-bob-to-approve", ADA_HOST);
    let bob = world.sign_up(6_120_002_302, "bob-approves-for-ada", BOB_HOST);
    make_sign_in_old(cloud, &ada.token);

    let deletion = delete_at_terminal(
        &world,
        &ada.machine,
        ada.handle.as_str(),
        AtGrant::Approve(&bob.user),
    );

    assert_refused(&deletion, 1, None);
    assert_kept(cloud, &ada);
    assert_kept(cloud, &bob);
}
