//! A second body of Ada's laptop that passes the proof check, made with the laptop's own keys as a
//! thief of the laptop could: neither machine trusts the laptop by either body, also once the
//! server gives only one of them.

use skald_crypto::{HpkeKey, issue};
use skald_proto::Cert;

use crate::world::{device_entry, now};
use crate::{Looked, Scene, warnings_naming};

/// The name the second body gives the laptop.
const STOLEN_NAME: &str = "ada-laptop-stolen";

/// A second body of the laptop: another `name` and `hpke` key, proved by the laptop's signing key
/// and, holding no other device's key, signed by it too.
fn second_body(scene: &Scene) -> Cert {
    let mut body = scene.cert.body().clone();
    body.name = STOLEN_NAME.to_owned();
    body.hpke = HpkeKey::generate().public_key();
    body.issued = now();
    let key = &scene.laptop.signing;
    issue(&body, key, key).expect("a certificate")
}

/// Fails unless each machine warned of the laptop in `looked`.
#[track_caller]
fn assert_warned(scene: &Scene, looked: &Looked) {
    for shown in [&looked.own, &looked.member] {
        assert!(
            !warnings_naming(shown, scene.laptop.id).is_empty(),
            "no warning of the laptop: {looked:?}"
        );
    }
}

/// The relay gives the second body beside the laptop's certificate: both machines warn of the
/// laptop and trust it by neither. Once the relay gives only the server's lists again, with the
/// laptop's certificate alone, they still trust it in two looks.
#[test]
fn second_body_given_beside_the_first_leaves_neither_trusted_once_it_is_gone() {
    let scene = Scene::new(
        [4_957_204_001, 4_957_204_002],
        ["ada-two-bodies", "bob-sees-two-bodies"],
    );
    let entry = device_entry(scene.laptop.id, &second_body(&scene));
    scene.change_lists(move |devices| devices.push(entry.clone()));

    let both = scene.look();

    assert_warned(&scene, &both);
    scene.assert_untrusted(&both, &[STOLEN_NAME]);
    scene.stop_changing();
    for _ in 0..2 {
        let first_alone = scene.look();
        scene.assert_untrusted(&first_alone, &[STOLEN_NAME]);
    }
}

/// The server's database holds the second body in place of the laptop's certificate: both machines
/// warn of the laptop and trust it by neither. Once the database holds its certificate again, they
/// still do not trust it.
#[test]
fn second_body_given_in_place_of_the_first_leaves_neither_trusted() {
    let scene = Scene::new(
        [4_957_205_001, 4_957_205_002],
        ["ada-stolen-body", "bob-sees-stolen-body"],
    );

    scene.give(&second_body(&scene));
    let second_alone = scene.look();

    assert_warned(&scene, &second_alone);
    scene.assert_untrusted(&second_alone, &[STOLEN_NAME]);
    scene.give(&scene.cert);
    let first_alone = scene.look();
    scene.assert_untrusted(&first_alone, &[STOLEN_NAME]);
}
