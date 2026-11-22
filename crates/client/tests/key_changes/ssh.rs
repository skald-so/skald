//! SSH signatures: a device's certificate carries an SSH signature by a key GitHub does not list
//! for the GitHub user it names.

use skald_crypto::issue;
use skald_github_stand_in::SshKeys;
use skald_proto::{AccountId, Cbor as _, DeviceId, SSH_LABEL, SSH_NAMESPACE, signed_data};
use ssh_key::encoding::Encode as _;
use ssh_key::private::Ed25519Keypair;
use ssh_key::{HashAlg, PrivateKey};

use crate::fixtures::{Issuer, add_device, hex};
use crate::world::{ACME, MEMBERS, World, create_with_member, members};
use crate::{ADA_HOST, BOB_HOST, assert_says_none, assert_waits_and_shows_once, signed_up_with};

/// The hidden setting of the GitHub API's address.
const GITHUB_API: &str = "SKALD_GITHUB_API_URL";

/// The `name` of the certificate of Ada's laptop.
const LAPTOP: &str = "ada-laptop";

/// An Ed25519 SSH key made from `seed`.
fn ssh_key_of(seed: u8) -> PrivateKey {
    Ed25519Keypair::from_seed(&[seed; 32]).into()
}

/// The binary form of the SSH signature by `key` of the device `device` of `account` with the
/// signing key `signing`, as the spec makes it.
fn signature_by(
    key: &PrivateKey,
    (account, device): (AccountId, DeviceId),
    signing: &[u8; 32],
) -> Vec<u8> {
    let parts: [&[u8]; 3] = [account.as_bytes(), device.as_bytes(), signing];
    let signed = key.sign(
        SSH_NAMESPACE,
        HashAlg::Sha512,
        &signed_data(SSH_LABEL, &parts),
    );
    (signed.expect("an SSH signature").encode_vec()).expect("its binary form")
}

/// Bob's machine reaches GitHub's API at the server's GitHub stand-in from its sign-in on; it
/// creates `acme` and adds Ada. Ada's laptop, a device the tests write to the server's database
/// certified by her machine's device, names her GitHub user and carries an SSH signature by a key
/// the stand-in does not list for that user, which lists another. Bob's `skald members acme`
/// checks it: a key change waits and shows once, naming the laptop.
#[test]
fn a_failed_ssh_signature_waits_until_seen() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_188_501_001, "ada-laptop-signed-badly", ADA_HOST);
    let bob = signed_up_with(
        &world,
        (4_188_501_002, "bob-checks-ssh-signatures"),
        BOB_HOST,
        (GITHUB_API, cloud.github.url()),
    );
    create_with_member(cloud, &bob, ACME, &ada);
    assert_says_none(&members(&bob.machine, ACME).shown());

    let listed = ssh_key_of(0x31)
        .public_key()
        .to_openssh()
        .expect("a public key line");
    cloud.github.put_ssh_keys(SshKeys {
        user: ada.user.id,
        authentication: vec![listed],
        signing: vec![],
    });
    let laptop = add_device(cloud, ada.account, LAPTOP, &Issuer::Device(&ada));
    let cert = cloud
        .cert(&ada.token, laptop.id)
        .expect("the laptop's certificate");
    let mut body = cert.body().clone();
    let at = (ada.account, laptop.id);
    body.ssh = Some(signature_by(&ssh_key_of(0x47), at, &body.signing));
    body.github = Some(ada.user.id);
    let signed = issue(&body, &ada.key, &laptop.signing).expect("a certificate");
    cloud.execute(&format!(
        "UPDATE devices SET cert = '\\x{}'::bytea WHERE id = '{}'::uuid",
        hex(&signed.to_cbor()),
        laptop.id
    ));

    members(&bob.machine, ACME);

    let names = [laptop.id.to_string()];
    assert_waits_and_shows_once(&bob.machine, &[MEMBERS, ACME], &names);
}
