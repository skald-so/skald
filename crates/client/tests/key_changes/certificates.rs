//! Certificates: the server gives a device Bob's machine trusted with another certificate, or with
//! a second body that passes the proof check.

use skald_crypto::{HpkeKey, SigningKey, issue};
use skald_proto::{Cbor as _, Cert};

use crate::fixtures::{Issuer, Keyed, add_device, hex};
use crate::world::{ACME, MEMBERS, members};
use crate::{Pair, assert_says_none, assert_waits_and_shows_once};

/// The `name` of the certificate of Ada's laptop.
const LAPTOP: &str = "ada-laptop";

/// Ada's laptop, a device the tests write to the server's database on her account, certified by
/// her machine's device; Bob's machine trusts it.
struct Laptop {
    pair: Pair,
    keyed: Keyed,
    cert: Cert,
}

impl Laptop {
    fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let pair = Pair::new(ids, logins);
        let cloud = &pair.world.cloud;
        let keyed = add_device(cloud, pair.ada.account, LAPTOP, &Issuer::Device(&pair.ada));
        let cert = cloud
            .cert(&pair.ada.token, keyed.id)
            .expect("the laptop's certificate");
        let trusted = members(&pair.bob.machine, ACME);
        assert!(
            trusted.stdout.contains(&keyed.id.to_string()),
            "Bob's machine does not show the laptop: {trusted:?}"
        );
        assert_says_none(&trusted.shown());
        Laptop { pair, keyed, cert }
    }

    /// The server's database holds `cert` as the laptop's certificate; Bob's `skald members acme`
    /// looks.
    fn given(&self, cert: &Cert) {
        let id = self.keyed.id;
        let bytes = hex(&cert.to_cbor());
        let sql = format!("UPDATE devices SET cert = '\\x{bytes}'::bytea WHERE id = '{id}'::uuid");
        self.pair.world.cloud.execute(&sql);
        members(&self.pair.bob.machine, ACME);
    }

    /// The laptop's ID, as the key change names it.
    fn names(&self) -> Vec<String> {
        vec![self.keyed.id.to_string()]
    }
}

/// The server gives the laptop with a certificate of other keys, which a key of no device signed:
/// a key change waits on Bob's machine, also once the server gives the laptop's certificate back,
/// and shows once, naming the laptop.
#[test]
fn a_swapped_certificate_waits_until_seen() {
    let laptop = Laptop::new(
        [4_188_201_001, 4_188_201_002],
        ["ada-whose-laptop-is-swapped", "bob-sees-a-swapped-laptop"],
    );
    let mut body = laptop.cert.body().clone();
    let signing = SigningKey::generate();
    body.signing = signing.public_key();
    body.hpke = HpkeKey::generate().public_key();
    let swapped = issue(&body, &SigningKey::generate(), &signing).expect("a certificate");

    laptop.given(&swapped);
    laptop.given(&laptop.cert);

    let machine = &laptop.pair.bob.machine;
    assert_waits_and_shows_once(machine, &[MEMBERS, ACME], &laptop.names());
}

/// The server gives a second body of the laptop that passes the proof check, made with the
/// laptop's own keys as a thief of it could: a key change waits on Bob's machine and shows once,
/// naming the laptop.
#[test]
fn a_second_body_of_a_device_waits_until_seen() {
    let laptop = Laptop::new(
        [4_188_202_001, 4_188_202_002],
        ["ada-whose-laptop-is-stolen", "bob-sees-a-second-body"],
    );
    let mut body = laptop.cert.body().clone();
    body.name = "ada-laptop-stolen".to_owned();
    body.hpke = HpkeKey::generate().public_key();
    let key = &laptop.keyed.signing;
    let second = issue(&body, key, key).expect("a certificate");

    laptop.given(&second);

    let machine = &laptop.pair.bob.machine;
    assert_waits_and_shows_once(machine, &[MEMBERS, ACME], &laptop.names());
}
