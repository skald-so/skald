//! The first device signs with a key file in the person's SSH directory that GitHub lists for the
//! account's GitHub user, never with one it does not list; the options naming a key, turning
//! signing off and naming the GitHub user.

use crate::{
    ADA_GITHUB, ADA_LOGIN, GITHUB, Kind, NO_SSH, SSH_KEY, Setup, WORK_GITHUB, WORK_LOGIN,
    assert_member_sees_passed, assert_says_unsigned_naming, assert_unsigned, bob_with, elsewhere,
    keys_dir, new_key, ssh_dir,
};

/// Ada's first device, as her GitHub user's login says.
const ADA: (u64, &str) = (ADA_GITHUB, ADA_LOGIN);

/// An unencrypted key file in Ada's SSH directory, which GitHub lists as an authentication key of
/// her GitHub user beside another key: her first device's certificate names her GitHub user and
/// carries a signature by it, the run says so, and Bob's device, a member of her space, checks it
/// as passed and shows her login.
#[test]
fn a_listed_key_file_signs_the_first_device_and_a_member_checks_it_passed() {
    let setup = Setup::new("");
    let other = elsewhere(&setup.machine, "other");
    setup.list(&[&other, &setup.key], &[]);

    let signed = setup.sign_up(&[]);

    signed.assert_signed(ADA, &signed.key, &[&other]);
    let bob = bob_with(&signed.world, &signed.api.url(), &signed.ada);
    assert_member_sees_passed(&bob, signed.ada.device, ADA_LOGIN);
}

/// Ada's SSH directory holds a key GitHub does not list for her GitHub user, under the name
/// OpenSSH tries first, and an ECDSA key it lists as a signing key: the certificate is signed by
/// the listed one only.
#[test]
fn a_listed_key_signs_and_one_github_does_not_list_is_not_used() {
    let setup = Setup::new("");
    let listed = new_key(&ssh_dir(&setup.machine).join("id_ecdsa"), Kind::Ecdsa, "");
    setup.list(&[], &[&listed]);

    let signed = setup.sign_up(&[]);

    signed.assert_signed(ADA, &listed, &[&signed.key]);
}

/// Ada's SSH directory holds only a key GitHub does not list for her GitHub user: the certificate
/// goes without `ssh`, the run says no key of her GitHub user was found, and the sign-in succeeds.
#[test]
fn with_only_a_key_github_does_not_list_the_certificate_goes_without_ssh() {
    let setup = Setup::new("");
    setup.list(&[&elsewhere(&setup.machine, "other")], &[]);

    let signed = setup.sign_up(&[]);

    assert_unsigned(&signed.cert());
    assert_says_unsigned_naming(&signed.shown, ADA_LOGIN);
}

/// Two listed keys, one in Ada's SSH directory and one in a file outside it: with the option naming
/// the second, the certificate is signed by it, not by the first.
#[test]
fn the_option_naming_a_key_signs_with_that_key() {
    let setup = Setup::new("");
    let named = new_key(&keys_dir(&setup.machine).join("work"), Kind::Ed25519, "");
    setup.list(&[&setup.key], &[&named]);

    let signed = setup.sign_up(&[SSH_KEY, named.file()]);

    signed.assert_signed(ADA, &named, &[&signed.key]);
}

/// A listed key in Ada's SSH directory: with the option turning signing off, the certificate goes
/// without `ssh`, the run names no key, and the sign-in succeeds.
#[test]
fn the_option_turning_signing_off_leaves_the_certificate_without_ssh() {
    let setup = Setup::new("");
    setup.list_key();

    let signed = setup.sign_up(&[NO_SSH]);

    assert_unsigned(&signed.cert());
    let named = signed.shown.contains(&signed.key.fingerprint);
    assert!(!named, "the key is named: {}", signed.shown);
}

/// The key in Ada's SSH directory is listed for another GitHub user, not for the one she signs in
/// as: with the option naming that user, the certificate names that user and carries a signature
/// by the key, and the run says so with that user's login.
#[test]
fn the_option_naming_a_github_user_signs_with_that_users_key() {
    let setup = Setup::new("");
    let other = elsewhere(&setup.machine, "other");
    setup.list(&[&other], &[]);
    setup
        .api
        .list((WORK_GITHUB, WORK_LOGIN), &[&setup.key], &[]);

    let signed = setup.sign_up(&[GITHUB, WORK_LOGIN]);

    signed.assert_signed((WORK_GITHUB, WORK_LOGIN), &signed.key, &[&other]);
}
