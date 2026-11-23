//! SSH signatures of a machine linked by its code of 12 characters (`membership.md`, "Link code"):
//! signed in with the browser before the code links it, it signs its request, and its certificate
//! carries the signature; linked without a session, it knows no account to sign for, so it sends
//! none and says why.

use crate::fixtures::fronted_with;
use crate::front::Front;
use crate::linking::{YES, link_with, typed};
use crate::ssh_kit::{
    ADA_GITHUB, ADA_HOST, ADA_LOGIN, Api, Kind, SshKey, assert_says_signed, assert_says_unsigned,
    assert_signed_by, assert_unsigned, machine, new_key, sign_up, ssh_dir, with_ssh_keygen,
};
use crate::terminal::Shown;
use crate::world::{Person, World};
use crate::{Fresh, certified_named, until};

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// Why a machine linked without a session goes without a signature, on the line that says so.
const WITHOUT_SESSION: &str = "without a session";

/// Ada, signed up on her first machine, and her new machine, with `ssh-keygen` and a key in her SSH
/// directory that GitHub's API lists for her GitHub user, signing in at a terminal and showing its
/// code of 12 characters. Its fields drop in order, the new machine's run first.
struct Laptop {
    fresh: Fresh,
    key: SshKey,
    ada: Person,
    _api: Api,
    /// The front the machines reach the server through, as a device key session needs.
    _front: Front,
    world: World,
}

impl Laptop {
    fn start() -> Self {
        let (world, front) = fronted_with(|_| {});
        let api = Api::start();
        let first = (machine(ADA_HOST, &api.url()), ADA_HOST);
        let (ada, _) = sign_up(&world, (ADA_GITHUB, ADA_LOGIN), first, &[]);
        let laptop = with_ssh_keygen(machine(LAPTOP, &api.url()));
        let key = new_key(&ssh_dir(&laptop).join("id_ed25519"), Kind::Ed25519, "");
        api.list((ADA_GITHUB, ADA_LOGIN), &[&key], &[]);
        let fresh = Fresh::start_on(&world.cloud, laptop, LAPTOP, &[]);
        Laptop {
            fresh,
            key,
            ada,
            _api: api,
            _front: front,
            world,
        }
    }

    /// Ada's first machine's `skald link` takes the new machine's code, and she confirms it; fails
    /// unless both succeed. Gives what the new machine showed and its certificate.
    #[track_caller]
    fn link(&mut self) -> (Shown, skald_proto::Cert) {
        let cloud = &self.world.cloud;
        let code = self.fresh.code(cloud);
        let linked = link_with(&self.ada, &typed(&code), LAPTOP, YES);
        assert!(linked.success, "skald link failed: {}", linked.text);
        let shown = self.fresh.finish();
        assert!(shown.success, "the new machine failed: {}", shown.text);
        let device = certified_named(cloud, &self.ada.token, LAPTOP);
        let cert = (cloud.cert(&self.ada.token, device)).expect("the new certificate");
        (shown, cert)
    }
}

/// Ada approves her new machine's browser sign-in before her first machine takes its code. Its
/// certificate names her GitHub user and carries a signature by her listed key, and it said so.
#[test]
fn signed_in_with_the_browser_before_the_code_links_it_the_machine_signs_its_request() {
    let mut laptop = Laptop::start();
    laptop.fresh.approve_as(&laptop.world, &laptop.ada.user);
    until("the new machine signed in", || {
        laptop.fresh.signed_in(&laptop.world.cloud)
    });

    let (shown, cert) = laptop.link();

    assert_signed_by(&cert, ADA_GITHUB, &laptop.key, &[]);
    assert_says_signed(&shown.text, &laptop.key, ADA_LOGIN);
}

/// Nobody approves the new machine's browser sign-in: linked by its code without a session, its
/// certificate carries neither a signature nor a GitHub user, and it says it went without one as it
/// linked without a session.
#[test]
fn linked_without_a_session_the_machine_sends_no_signature_and_says_why() {
    let mut laptop = Laptop::start();

    let (shown, cert) = laptop.link();

    assert_unsigned(&cert);
    assert_says_unsigned(&shown.text, WITHOUT_SESSION);
}
