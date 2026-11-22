//! Without `ssh-keygen`, a GitHub user or GitHub, the first device's certificate goes without
//! `ssh`, the run says why, and the sign-in succeeds.

use skald_github_stand_in::Fault;
use skald_proto::Cert;

use crate::browser::Browser;
use crate::cloud::{Cloud, Providers};
use crate::machine::Finished;
use crate::world::World;
use crate::{
    ADA_GITHUB, ADA_LOGIN, Api, GITHUB, Kind, Setup, assert_says_signed, assert_says_unsigned,
    assert_signed_by, assert_unsigned, machine, new_key, nowhere, ssh_dir, with_ssh_keygen,
};

/// A listed key in Ada's SSH directory, but no `ssh-keygen` on the machine's `PATH`: no `ssh`, the
/// run says `ssh-keygen` is missing, and the sign-in succeeds.
#[test]
fn without_ssh_keygen_the_certificate_goes_without_ssh() {
    let setup = Setup::without_ssh_keygen();
    setup.list_key();

    setup.sign_up(&[]).assert_unsigned("ssh-keygen");
}

/// GitHub's API cannot be reached: with a key in Ada's SSH directory, no `ssh`, the run says GitHub
/// was not reached, and the sign-in succeeds.
#[test]
fn with_github_unreachable_the_certificate_goes_without_ssh() {
    let setup = Setup::new("").reaching(&nowhere());

    setup.sign_up(&[]).assert_unsigned("GitHub");
}

/// GitHub's API answers with a server error, or as GitHub at its rate limit, though it lists the
/// key in Ada's SSH directory: no `ssh`, the run says why, naming GitHub, and the sign-in succeeds.
#[test]
fn with_github_answering_errors_or_at_its_rate_limit_the_certificate_goes_without_ssh() {
    for fault in [Fault::ApiStatus { status: 502 }, Fault::RateLimited] {
        let setup = Setup::new("");
        setup.list_key();
        setup.api.github.set_faults(&[fault]);

        let signed = setup.sign_up(&[]);

        signed.assert_unsigned("GitHub");
        let asked = signed.api.github.requests();
        assert!(!asked.is_empty(), "{fault:?}: GitHub's API was not asked");
    }
}

/// The Google subjects, emails and host names of two people on a server with Google sign-in only.
const CAROL: (&str, &str, &str) = (
    "110248495921238995101",
    "carol.ssh@example.com",
    "carol-workstation",
);
const DAVE: (&str, &str, &str) = (
    "110248495921238995102",
    "dave.ssh@example.com",
    "dave-workstation",
);

/// The person `(subject, email, host)` signs a new machine on `host`, with `ssh-keygen` and a key
/// in the person's SSH directory, in with Google as the first device of a new account, with the
/// options `extra`; fails unless the run succeeds. Gives the key, what the run printed and the
/// device's certificate.
#[track_caller]
fn sign_up_with_google(
    (world, api): (&World, &Api),
    (subject, email, host): (&str, &str, &str),
    extra: &[&str],
) -> (crate::SshKey, Finished, Cert) {
    let cloud = &world.cloud;
    let machine = with_ssh_keygen(machine(host, &api.url()));
    let key = new_key(&ssh_dir(&machine).join("id_ed25519"), Kind::Ed25519, "");
    api.list((ADA_GITHUB, ADA_LOGIN), &[&key], &[]);
    let person = cloud.person(subject, email);
    let signed_in = cloud.signed_in_with_google(&world.browser.page(), &machine, &person, extra);
    assert!(signed_in.success, "the sign-in failed: {signed_in:?}");
    let devices = cloud.devices(&cloud.last_token());
    assert_eq!(devices.len(), 1, "the account's devices: {devices:?}");
    let cert = devices[0].cert.clone().expect("the device's certificate");
    (key, signed_in, cert)
}

/// On a server with Google sign-in only, Carol's account has no GitHub sign-in method: with a key
/// in her SSH directory that GitHub lists for a GitHub user, her certificate goes without `ssh`,
/// the run says no GitHub user is known, and the sign-in succeeds. Dave, with the same setup and
/// the option naming that GitHub user, signs with his key.
#[test]
fn without_a_github_sign_in_method_only_the_option_naming_a_github_user_signs() {
    let world = World {
        cloud: Cloud::with(Providers::Google, 120, 1),
        browser: Browser::launch(),
    };
    let api = Api::start();

    let (_, signed_in, cert) = sign_up_with_google((&world, &api), CAROL, &[]);
    assert_unsigned(&cert);
    assert_says_unsigned(&signed_in.shown(), "GitHub");

    let (key, signed_in, cert) = sign_up_with_google((&world, &api), DAVE, &[GITHUB, ADA_LOGIN]);
    assert_signed_by(&cert, ADA_GITHUB, &key, &[]);
    assert_says_signed(&signed_in.shown(), &key, ADA_LOGIN);
}
