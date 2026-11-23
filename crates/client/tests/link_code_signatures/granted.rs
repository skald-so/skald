//! The new device takes a certificate only if it holds the SSH signature and GitHub user as the
//! new device sent them, or neither (`membership.md`, "Link code"). The test plays the issuing
//! device with Ada's first device's key, from the code the new device shows, as the tests of
//! linking with a link code do (`link_codes/grants.rs`).

use std::fs;

use skald_crypto::{SshCheck, check_ssh_signature};
use skald_proto::{CertBody, SSH_LABEL, SSH_NAMESPACE, signed_data};
use ssh_key::encoding::Encode as _;
use ssh_key::{HashAlg, PrivateKey};

use crate::linking::NewDevice;
use crate::played_issuer::{Granted, answer};
use crate::ssh_kit::{
    ADA_GITHUB, ADA_LOGIN, Kind, SshKey, WORK_GITHUB, assert_unsigned, keys_dir, new_key,
};
use crate::{Scene, assert_no_certificate};

/// The binary form of the SSH signature `key` makes over the device of `body`, as the spec says a
/// certificate's `ssh` is made.
fn made_by(key: &SshKey, body: &CertBody) -> Vec<u8> {
    let file = fs::read_to_string(&key.path).expect("the key's private key file");
    let private = PrivateKey::from_openssh(file).expect("an OpenSSH private key");
    let over: [&[u8]; 3] = [
        body.account.as_bytes(),
        body.device.as_bytes(),
        &body.signing,
    ];
    let made = (private.sign(
        SSH_NAMESPACE,
        HashAlg::Sha512,
        &signed_data(SSH_LABEL, &over),
    ))
    .expect("sign the device key");
    made.encode_vec().expect("encode the SSH signature")
}

/// Fails unless the run's honest certificate body with `ssh` and `github` passes the check against
/// a list of `key` alone.
#[track_caller]
fn assert_passes(granted: &Granted, key: &SshKey) {
    assert_eq!(
        check_ssh_signature(&granted.cert, &[&key.line]),
        SshCheck::Passed { key: 0 },
        "not a signature by {} over the new device: {:?}",
        key.fingerprint,
        granted.cert
    );
}

/// The played issuing device answers the next run of `new`, opens its request, and sends a grant
/// whose certificate is the honest one, with the request's `ssh` and `github`, as `change` makes
/// it; gives the request's `ssh` and `github`.
fn grant_with(
    scene: &Scene,
    new: &mut NewDevice,
    change: impl FnOnce(&mut Granted),
) -> (Option<Vec<u8>>, Option<u64>) {
    let cloud = &scene.world.cloud;
    let run = answer(cloud, &scene.ada, new);
    let sent = (run.request.ssh.clone(), run.request.github);
    let mut granted = Granted::honest(cloud, &scene.ada, &run);
    (granted.cert.ssh, granted.cert.github) = sent.clone();
    change(&mut granted);
    granted.send(cloud, &scene.ada, &run);
    sent
}

/// Fails unless the new device `new` exits with success once the last grant, with neither, linked
/// it, and its certificate on the server carries neither.
#[track_caller]
fn assert_taken_without(scene: &Scene, new: &mut NewDevice) {
    let shown = new.finish();
    assert!(shown.success, "a grant with neither failed: {}", shown.text);
    let cert = (scene.world.cloud.cert(&new.token, new.device)).expect("the new certificate");
    assert_unsigned(&cert);
}

/// Ada's new machine has a key in her SSH directory that GitHub's API lists for her GitHub user,
/// beside another key of hers in a file outside that directory. Its request carries a signature by that key over the
/// new device and her GitHub user. A grant whose certificate carries the signature by her other
/// listed key in its place fails the run, and so does one that names another GitHub user with the
/// new device's signature: the new device sends no certificate. One that carries neither links it,
/// and its certificate on the server carries neither.
#[test]
fn a_grant_that_changes_the_signature_or_the_github_user_is_refused_and_one_with_neither_is_taken()
{
    let scene = Scene::start(None);
    let (laptop, key) = scene.new_machine(&scene.api.url());
    let other = new_key(&keys_dir(&laptop).join("other"), Kind::Ed25519, "");
    scene
        .api
        .list((ADA_GITHUB, ADA_LOGIN), &[&key, &other], &[]);
    let mut new = scene.new_device(laptop);

    let sent = grant_with(&scene, &mut new, |granted| {
        assert_passes(granted, &key);
        granted.cert.ssh = Some(made_by(&other, &granted.cert));
        assert_passes(granted, &other);
    });
    assert!(sent.0.is_some(), "the request carries no SSH signature");
    assert_eq!(sent.1, Some(ADA_GITHUB), "the request's GitHub user");
    new.wait_for_failures(1);
    assert_no_certificate(&scene, &new);

    grant_with(&scene, &mut new, |granted| {
        granted.cert.github = Some(WORK_GITHUB);
    });
    new.wait_for_failures(2);
    assert_no_certificate(&scene, &new);

    grant_with(&scene, &mut new, |granted| {
        (granted.cert.ssh, granted.cert.github) = (None, None);
    });
    assert_taken_without(&scene, &mut new);
}

/// Ada's new machine has no key in her SSH directory; GitHub's API lists for her GitHub user a key
/// in a file elsewhere on that machine. Its request carries neither an SSH signature nor a GitHub
/// user. A grant whose certificate carries a signature by that listed key over the new device and
/// her GitHub user fails the run: the new device sends no certificate. One that carries neither
/// links it.
#[test]
fn a_grant_that_adds_a_signature_the_new_device_did_not_send_is_refused() {
    let scene = Scene::start(None);
    let (laptop, key) = scene.new_machine(&scene.api.url());
    key.remove_files();
    let listed = new_key(&keys_dir(&laptop).join("listed"), Kind::Ed25519, "");
    scene.api.list((ADA_GITHUB, ADA_LOGIN), &[&listed], &[]);
    let mut new = scene.new_device(laptop);

    let sent = grant_with(&scene, &mut new, |granted| {
        granted.cert.ssh = Some(made_by(&listed, &granted.cert));
        granted.cert.github = Some(ADA_GITHUB);
        assert_passes(granted, &listed);
    });
    assert_eq!(sent, (None, None), "the request carries a signature");
    new.wait_for_failures(1);
    assert_no_certificate(&scene, &new);

    grant_with(&scene, &mut new, |_| {});
    assert_taken_without(&scene, &mut new);
}
