//! The server gives Ada's laptop, which both machines trusted, with another certificate: each
//! machine warns once and does not trust the laptop until the server gives its certificate back,
//! then trusts it with no warning. Before that, the same certificates given again, and the laptop
//! withheld and given back, warn of nothing.

use skald_crypto::{HpkeKey, SigningKey, issue};
use skald_proto::{Cert, DeviceId};

use crate::{ADA_HOST, Finished, Looked, Scene, warnings_naming};

/// The name a swapped certificate gives the laptop.
const SWAPPED_NAME: &str = "ada-laptop-swapped";

/// What the server changes in the laptop's certificate. It holds none of the keys of the account's
/// devices, so its certificate fails the proof check or check 1.
#[derive(Debug, Clone, Copy)]
enum Swap {
    /// Other `signing` and `hpke` keys, proved by the new signing key and signed by a key of no
    /// device.
    Keys,
    /// Another `name`, signed and proved by keys of no device.
    Name,
    /// The same body with another `signature`.
    Signature,
}

/// The laptop's certificate `genuine` with `swap` made to it.
fn swapped(genuine: &Cert, swap: Swap) -> Cert {
    let mut body = genuine.body().clone();
    match swap {
        Swap::Keys => {
            let signing = SigningKey::generate();
            body.signing = signing.public_key();
            body.hpke = HpkeKey::generate().public_key();
            issue(&body, &SigningKey::generate(), &signing).expect("a certificate")
        }
        Swap::Name => {
            body.name = SWAPPED_NAME.to_owned();
            issue(&body, &SigningKey::generate(), &SigningKey::generate()).expect("a certificate")
        }
        Swap::Signature => {
            let mut signature = *genuine.signature();
            signature[0] ^= 1;
            Cert::from_parts(genuine.body_bytes().to_vec(), signature, *genuine.proof())
                .expect("a certificate")
        }
    }
}

/// Fails if a machine warned of the laptop in `looked`.
#[track_caller]
fn assert_quiet(looked: &Looked, laptop: DeviceId) {
    for shown in [&looked.own, &looked.member] {
        assert_eq!(
            warnings_naming(shown, laptop),
            Vec::<String>::new(),
            "a warning of the laptop: {looked:?}"
        );
    }
}

/// Both machines look again, then with the laptop withheld by the relay, then given back: they
/// trust it whenever it is given and warn of nothing. The server's database then holds the
/// laptop's certificate with `swap` made to it: each machine warns of the laptop once over two
/// looks and trusts it in neither. Once the database holds its certificate again, both trust it
/// with no warning.
fn swap_warns_once(swap: Swap, ids: [u64; 2], logins: [&str; 2]) {
    let scene = Scene::new(ids, logins);
    let laptop = scene.laptop.id;

    let again = scene.look();
    scene.assert_trusted(&again);
    assert_quiet(&again, laptop);
    let gone = laptop.to_string();
    scene.change_lists(move |devices| devices.retain(|entry| entry["device"] != gone.as_str()));
    let withheld = scene.look();
    assert_quiet(&withheld, laptop);
    scene.stop_changing();
    let back = scene.look();
    scene.assert_trusted(&back);
    assert_quiet(&back, laptop);

    scene.give(&swapped(&scene.cert, swap));
    let looks = [scene.look(), scene.look()];

    let warned = |pick: fn(&Looked) -> &Finished| -> usize {
        looks
            .iter()
            .map(|looked| warnings_naming(pick(looked), laptop).len())
            .sum()
    };
    assert_eq!(
        warned(|looked| &looked.own),
        1,
        "Ada's machine did not warn once of the swapped certificate: {looks:?}"
    );
    assert_eq!(
        warned(|looked| &looked.member),
        1,
        "Bob's machine did not warn once of the swapped certificate: {looks:?}"
    );
    for looked in &looks {
        scene.assert_untrusted(looked, &[SWAPPED_NAME]);
    }

    scene.give(&scene.cert);
    let restored = scene.look();
    scene.assert_trusted(&restored);
    assert_quiet(&restored, laptop);
    assert!(
        restored.trusts(scene.ada.device, ADA_HOST),
        "Ada's machine is not trusted: {restored:?}"
    );
}

#[test]
fn certificate_with_other_keys_warns_once_until_it_is_given_back() {
    swap_warns_once(
        Swap::Keys,
        [4_957_201_001, 4_957_201_002],
        ["ada-swapped-keys", "bob-sees-swapped-keys"],
    );
}

#[test]
fn certificate_with_another_name_warns_once_until_it_is_given_back() {
    swap_warns_once(
        Swap::Name,
        [4_957_202_001, 4_957_202_002],
        ["ada-swapped-name", "bob-sees-swapped-name"],
    );
}

#[test]
fn certificate_with_another_signature_warns_once_until_it_is_given_back() {
    swap_warns_once(
        Swap::Signature,
        [4_957_203_001, 4_957_203_002],
        ["ada-swapped-signature", "bob-sees-swapped-signature"],
    );
}
