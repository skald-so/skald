//! What the tests share: Ada and Bob in `acme`, the KeyPackages API as Bob's session and Ada's use
//! it, the fields of a KeyPackage, and moving their expiry in the server's database.

use std::collections::BTreeSet;

use serde_json::json;
use skald_crypto::{MlsDevice, SigningKey, StateKey};
use skald_proto::{Cert, ChannelId, Role};

use crate::answers::{handed_of, status_of};
use crate::cloud::Cloud;
use crate::fixtures::{cert_of, fronted};
use crate::front::Front;
use crate::machine::Finished;
use crate::world::{ACME, Person, World, create, next_by, now, post_as, run, store};

/// The command of a run that connects.
pub const RECV: &str = "recv";

/// How many unused KeyPackages a device keeps, and below how many it uploads more.
pub const KEPT: u64 = 20;

/// Ada, whose machine signed up and created `acme`, and Bob, whom a revision adds to it; Bob's
/// session takes Ada's device's KeyPackages.
pub struct Pair {
    pub world: World,
    pub front: Front,
    pub ada: Person,
    pub bob: Person,
    /// The certificate of Ada's device.
    pub ada_cert: Cert,
}

impl Pair {
    /// In a world of its own, GitHub users `ids` with `logins` sign up as Ada and Bob.
    pub fn start(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let (world, front) = fronted();
        let ada = world.sign_up(ids[0], logins[0], "ada-workstation");
        let bob = world.sign_up(ids[1], logins[1], "bob-workstation");
        let cloud = &world.cloud;
        let created = create(cloud, &ada, ACME);
        let with_bob = next_by(&created, &ada, |list| list.push(bob.member(Role::Member)));
        store(cloud, &ada.token, &with_bob);
        Pair {
            ada_cert: cert_of(cloud, &ada),
            world,
            front,
            ada,
            bob,
        }
    }

    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// What the server says of Ada's device's KeyPackages: how many unused ones expire in more than
    /// 7 days, and whether its last-resort one is to be replaced.
    #[track_caller]
    pub fn status(&self) -> (u64, bool) {
        let path = format!("/devices/{}/key-packages", self.ada.device);
        let (status, body) = self.cloud().get_as(&path, &self.ada.token);
        assert_eq!(status, 200, "GET {path}: {body}");
        status_of(&body)
    }

    /// One KeyPackage of Ada's device that the server hands Bob's session, if any.
    #[track_caller]
    pub fn take(&self) -> Option<Vec<u8>> {
        let request = json!({ "devices": [self.ada.device.to_string()] });
        let (status, body) = post_as(
            self.cloud(),
            &self.bob.token,
            "/key-packages",
            Some(&request),
        );
        assert_eq!(status, 200, "POST /key-packages: {body}");
        match handed_of(&body).as_slice() {
            [] => None,
            [(device, bytes)] if *device == self.ada.device => Some(bytes.clone()),
            _ => panic!("not one KeyPackage of Ada's device: {body}"),
        }
    }

    /// `count` KeyPackages of Ada's device that Bob takes, each one other than the last-resort
    /// one, all different; fails unless each is Ada's device's.
    #[track_caller]
    pub fn take_others(&self, count: u64) -> Vec<Vec<u8>> {
        let mut taken = Vec::new();
        for n in 0..count {
            let one = self
                .take()
                .unwrap_or_else(|| panic!("KeyPackage {n} of {count}"));
            assert!(!taken.contains(&one), "KeyPackage {n} handed out twice");
            self.assert_of_ada(&one);
            taken.push(one);
        }
        taken
    }

    /// Bob takes every unused KeyPackage of Ada's device, [`KEPT`] of them, then its last-resort
    /// one twice, the same bytes each time; gives them, the last-resort one last.
    #[track_caller]
    pub fn take_all(&self) -> (Vec<Vec<u8>>, Vec<u8>) {
        let others = self.take_others(KEPT);
        let last = self.take().expect("the last-resort KeyPackage");
        assert!(
            !others.contains(&last),
            "the last-resort one among the others"
        );
        assert_eq!(self.take(), Some(last.clone()), "the last-resort one again");
        self.assert_of_ada(&last);
        (others, last)
    }

    /// Fails unless `key_package` is of Ada's device: its `identity` is the device's ID and its
    /// `signature_key` the certificate's signing key.
    #[track_caller]
    pub fn assert_of_ada(&self, key_package: &[u8]) {
        let fields = Fields::of(key_package);
        assert_eq!(fields.identity, *self.ada.device.as_bytes(), "the identity");
        assert_eq!(
            fields.signature,
            self.ada_cert.body().signing,
            "the signature_key"
        );
    }

    /// Fails unless a device of the tests', with Ada's certificate as the one admitted, adds Ada's
    /// device to a group with `key_package`, as the client's own code checks it.
    #[track_caller]
    pub fn assert_adds_ada(&self, key_package: &[u8]) {
        let mut adder = MlsDevice::new(&SigningKey::generate(), &StateKey::generate());
        let channel = ChannelId::generate();
        adder.create_group(channel, now()).expect("a group");
        let admitted = std::slice::from_ref(&self.ada_cert);
        if let Err(err) = adder.add_devices(channel, &[key_package], admitted, now()) {
            panic!("the KeyPackage does not add Ada's device: {err:?}");
        }
    }

    /// Runs `skald recv` on Ada's machine; fails unless it succeeds.
    #[track_caller]
    pub fn recv(&self) -> Finished {
        let finished = run(&self.ada.machine, &[RECV]);
        assert!(finished.success, "{finished:?}");
        finished
    }

    /// Moves the expiry of `count` of the unused KeyPackages of Ada's device, or of its last-resort
    /// one, to 6 days from now in the server's database.
    pub fn expire_soon(&self, count: u64, last_resort: bool) {
        self.cloud().execute(&format!(
            "UPDATE key_packages SET expires_at = now() + interval '6 days'
             WHERE ctid IN (
               SELECT ctid FROM key_packages
               WHERE device_id = '{}'::uuid AND last_resort = {last_resort}
               LIMIT {count})",
            self.ada.device
        ));
    }
}

/// The fields of a KeyPackage in an `MLSMessage` that the tests read (RFC 9420, §6 and §10): its
/// init key, and its leaf's encryption key, signature key and `basic` credential's identity, the
/// 16 bytes of a device ID.
pub struct Fields {
    pub init: [u8; 32],
    pub encryption: [u8; 32],
    pub signature: [u8; 32],
    pub identity: [u8; 16],
}

impl Fields {
    #[track_caller]
    pub fn of(message: &[u8]) -> Self {
        // `MLSMessage`: version `mls10` and wire format `mls_key_package`; then the KeyPackage:
        // version and ciphersuite `0x0001`.
        assert_eq!(
            message.get(..8),
            Some([0, 1, 0, 5, 0, 1, 0, 1].as_slice()),
            "an MLSMessage of a KeyPackage of ciphersuite 0x0001"
        );
        let mut at = 8;
        let init = vector(message, &mut at, 32);
        let encryption = vector(message, &mut at, 32);
        let signature = vector(message, &mut at, 32);
        // The credential type `basic`, then the identity.
        assert_eq!(
            message.get(at..at + 2),
            Some([0, 1].as_slice()),
            "a basic credential"
        );
        at += 2;
        let identity = vector(message, &mut at, 16);
        Fields {
            init: init.try_into().expect("32 bytes"),
            encryption: encryption.try_into().expect("32 bytes"),
            signature: signature.try_into().expect("32 bytes"),
            identity: identity.try_into().expect("16 bytes"),
        }
    }
}

/// The variable-length vector of `len` bytes at `at` in `message`, its length taking one byte;
/// moves `at` past it.
#[track_caller]
fn vector(message: &[u8], at: &mut usize, len: usize) -> Vec<u8> {
    let start = *at;
    assert_eq!(
        message.get(start),
        u8::try_from(len).ok().as_ref(),
        "a length of {len} at {start}"
    );
    *at = start + 1 + len;
    message.get(start + 1..*at).expect("a field").to_vec()
}

/// The X25519 public keys of the KeyPackages `key_packages`: their init and encryption keys.
pub fn hpke_keys<'a>(key_packages: impl IntoIterator<Item = &'a Vec<u8>>) -> BTreeSet<[u8; 32]> {
    key_packages
        .into_iter()
        .flat_map(|bytes| {
            let fields = Fields::of(bytes);
            [fields.init, fields.encryption]
        })
        .collect()
}
