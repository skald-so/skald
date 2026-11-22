//! An encrypted key file: at the person's terminal, `ssh-keygen` asks for its passphrase, which the
//! person types; as an agent or without a terminal, it is not used and nothing asks.

use std::thread;
use std::time::Duration;

use crate::at_terminal::{first_at_terminal, shown_key};
use crate::{
    ADA_GITHUB, ADA_HOST, ADA_LOGIN, ASKS_PASSPHRASE, PASSPHRASE, SAYS_NO_SSH, Setup, Signed,
    assert_asks_nothing, assert_says_signed, assert_signed_by, own_cert,
};

/// How long the person waits, once asked for the passphrase, before typing it: `ssh-keygen` turns
/// echo off around the question.
const ECHO_OFF: Duration = Duration::from_millis(300);

/// A listed key in Ada's SSH directory, encrypted: at her terminal, once she approves the code,
/// the run asks for its passphrase, she types it, and her first device's certificate carries a
/// signature by that key.
#[test]
fn at_the_persons_terminal_ssh_keygen_asks_for_the_passphrase_the_person_types() {
    let setup = Setup::new(PASSPHRASE);
    setup.list_key();
    let (world, machine) = (&setup.world, &setup.machine);
    let user = world.cloud.user(ADA_GITHUB, ADA_LOGIN);

    let mut run = first_at_terminal(&world.cloud, &world.browser.page(), machine, &user, &[]);
    run.wait_for("a question for the passphrase", |shown| {
        shown.to_lowercase().contains(ASKS_PASSPHRASE)
    });
    thread::sleep(ECHO_OFF);
    run.type_line(PASSPHRASE);
    shown_key(&mut run);
    run.enter();
    let shown = run.finish();

    assert!(shown.success, "the sign-in failed: {shown:?}");
    assert!(!shown.text.contains(PASSPHRASE), "the passphrase is shown");
    let ada = setup.world.person(user, setup.machine, ADA_HOST);
    assert_signed_by(&own_cert(&setup.world, &ada), ADA_GITHUB, &setup.key, &[]);
    assert_says_signed(&shown.text, &setup.key, ADA_LOGIN);
}

/// Fails unless `signed`'s certificate goes without `ssh`, its sign-in said so and asked nothing.
#[track_caller]
fn assert_unused(signed: &Signed) {
    signed.assert_unsigned(SAYS_NO_SSH);
    assert_asks_nothing(&signed.shown);
}

/// The same encrypted key, signing in as an agent at a terminal: nothing asks for a passphrase,
/// the certificate goes without `ssh`, the run says why, and the sign-in succeeds.
#[test]
fn as_an_agent_an_encrypted_key_file_is_not_used_and_nothing_asks() {
    let setup = Setup::new(PASSPHRASE);
    setup.list_key();

    assert_unused(&setup.sign_up_at_terminal(true));
}

/// The same encrypted key, signing in without a terminal: nothing asks for a passphrase, the
/// certificate goes without `ssh`, the run says why, and the sign-in succeeds.
#[test]
fn without_a_terminal_an_encrypted_key_file_is_not_used_and_nothing_asks() {
    let setup = Setup::new(PASSPHRASE);
    setup.list_key();

    assert_unused(&setup.sign_up(&[]));
}
