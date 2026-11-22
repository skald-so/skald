//! An encrypted key file: at the person's terminal, `ssh-keygen` asks for its passphrase, which the
//! person types; as an agent or without a terminal, it is not used and nothing asks.

use std::fs;
use std::thread;
use std::time::Duration;

use crate::at_terminal::{first_at_terminal, shown_key};
use crate::{
    ADA_GITHUB, ADA_HOST, ADA_LOGIN, ASKS_PASSPHRASE, Kind, PASSPHRASE, SAYS_NO_SSH, Setup, Signed,
    assert_asks_nothing, assert_says_signed, assert_signed_by, keys_dir, new_key, own_cert,
    ssh_dir,
};

/// How long the person waits, once asked for the passphrase, before typing it: `ssh-keygen` turns
/// echo off around the question.
const ECHO_OFF: Duration = Duration::from_millis(300);

/// How many times `ssh-keygen` asks for a passphrase in
/// [`at_the_persons_terminal_each_passphrase_typed_goes_to_ssh_keygen`].
const QUESTIONS: usize = 12;

/// A listed key in Ada's SSH directory, encrypted: at her terminal, once she approves the code,
/// the run asks for its passphrase, she types it, and her first device's certificate carries a
/// signature by that key.
#[test]
fn at_the_persons_terminal_ssh_keygen_asks_for_the_passphrase_the_person_types() {
    typed(Setup::new(PASSPHRASE), 1);
}

/// As the person's terminal test above, with a display, where OpenSSH asks through a program,
/// `SSH_ASKPASS`, for a passphrase it cannot ask for at a terminal: it asks at the terminal.
#[test]
fn at_the_persons_terminal_with_a_display_ssh_keygen_asks_there() {
    let mut setup = Setup::new(PASSPHRASE);
    setup.machine = setup.machine.with_env("DISPLAY", ":0");
    typed(setup, 1);
}

/// Ada's SSH directory holds, named before her listed encrypted key, files whose public key is
/// hers but whose private key is another, encrypted with the same passphrase. At her terminal,
/// `ssh-keygen` asks for the passphrase of each, then fails to sign with it, then asks for her
/// key's: each passphrase she types goes to `ssh-keygen`, none to the command's own reading of her
/// lines, which waits meanwhile, and her key signs.
#[test]
fn at_the_persons_terminal_each_passphrase_typed_goes_to_ssh_keygen() {
    let setup = Setup::new(PASSPHRASE);
    let other = new_key(
        &keys_dir(&setup.machine).join("other"),
        Kind::Ed25519,
        PASSPHRASE,
    );
    let dir = ssh_dir(&setup.machine);
    for at in 1..QUESTIONS {
        let name = format!("another_{at:02}");
        fs::copy(&other.path, dir.join(&name)).expect("copy a private key file");
        (fs::copy(setup.key.public(), dir.join(format!("{name}.pub"))))
            .expect("copy a public key file");
    }
    typed(setup, QUESTIONS);
}

/// Lists the key in `setup`'s SSH directory, encrypted with [`PASSPHRASE`]; Ada signs up at her
/// terminal, typing the passphrase each time she is asked for it, `questions` times. Fails unless her
/// first device's certificate carries a signature by that key, her sign-in said so and showed no
/// passphrase.
#[track_caller]
fn typed(setup: Setup, questions: usize) {
    setup.list_key();
    let (world, machine) = (&setup.world, &setup.machine);
    let user = world.cloud.user(ADA_GITHUB, ADA_LOGIN);

    let mut run = first_at_terminal(&world.cloud, &world.browser.page(), machine, &user, &[]);
    for asked in 1..=questions {
        run.wait_for("a question for the passphrase", |shown| {
            shown.to_lowercase().matches(ASKS_PASSPHRASE).count() >= asked
        });
        thread::sleep(ECHO_OFF);
        run.type_line(PASSPHRASE);
    }
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

    assert_unused(&setup.sign_up_at_terminal(true, &[]));
}

/// The same encrypted key, signing in without a terminal: nothing asks for a passphrase, the
/// certificate goes without `ssh`, the run says why, and the sign-in succeeds.
#[test]
fn without_a_terminal_an_encrypted_key_file_is_not_used_and_nothing_asks() {
    let setup = Setup::new(PASSPHRASE);
    setup.list_key();

    assert_unused(&setup.sign_up(&[]));
}
