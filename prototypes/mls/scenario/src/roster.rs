//! A test fixture that stands in for what a device gets from the server and accepts from the
//! roster: certificates and revocations, the members' pinned master keys and the channel's
//! entries. It gives the channel's admitted devices, as `docs/protocol/membership.md`, "Admitted
//! devices", says, from certificates valid as "Device checks" says.
//!
//! The channel is not a shared one, and the space mode admits no `custodian` or `auditor` device.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    DeviceId, Leaf, Result,
    cbor::Value,
    certificate::{Body, Certificate, Kind},
    revocation::{self, AccountId, Revocation},
};

/// What a device knows of the channel's devices.
#[derive(Debug, Clone, Default)]
pub struct Roster {
    /// The pinned master public key of each account.
    masters: BTreeMap<AccountId, [u8; 32]>,
    certificates: Vec<Certificate>,
    revocations: Vec<Vec<u8>>,
    /// The channel's `channel-member` entries: each member account and the devices its
    /// `excluded` lists.
    members: BTreeMap<AccountId, BTreeSet<DeviceId>>,
    /// The channel's `services`.
    services: BTreeSet<DeviceId>,
}

impl Roster {
    /// Pins `master` as the master key of `account`.
    pub fn pin(&mut self, account: AccountId, master: [u8; 32]) {
        self.masters.insert(account, master);
    }

    pub fn master(&self, account: &AccountId) -> Option<[u8; 32]> {
        self.masters.get(account).copied()
    }

    /// Adds an encoded certificate, as the server gives it; fails if it is malformed.
    pub fn add_certificate(&mut self, cert: &[u8]) -> Result<()> {
        self.certificates.push(Certificate::decode(cert)?);
        Ok(())
    }

    /// Adds an encoded revocation, as the server or a Commit gives it.
    pub fn add_revocation(&mut self, revocation: Vec<u8>) {
        self.revocations.push(revocation);
    }

    /// Adds `account` to the channel's members.
    pub fn add_member(&mut self, account: AccountId) {
        self.members.entry(account).or_default();
    }

    /// Lists `device` in the `excluded` of the member `account`.
    pub fn exclude(&mut self, account: AccountId, device: DeviceId) {
        self.members.entry(account).or_default().insert(device);
    }

    /// Adds `device` to the channel's `services`.
    pub fn add_service(&mut self, device: DeviceId) {
        self.services.insert(device);
    }

    /// The certificates of `device` that pass the proof check, all with one body: none if there
    /// are none, or if two with different bodies pass it.
    fn certificates(&self, device: &DeviceId) -> Option<Vec<&Certificate>> {
        let copies: Vec<_> = self
            .certificates
            .iter()
            .filter(|cert| cert.body.device == *device && cert.passes_proof_check())
            .collect();
        let first = copies.first()?;
        copies
            .iter()
            .all(|cert| cert.same_body(first))
            .then_some(copies)
    }

    /// The certificate bodies of the chain of `device`, its own first and the master key's
    /// last, if they pass checks 1 and 2 of "Device checks".
    pub fn chain(&self, device: &DeviceId) -> Option<Vec<Body>> {
        let mut chain: Vec<Body> = Vec::new();
        let mut next = *device;
        // A chain without a repeated device is at most as long as the list of certificates.
        for _ in 0..=self.certificates.len() {
            if chain.iter().any(|body| body.device == next) {
                return None;
            }
            let copies = self.certificates(&next)?;
            let body = copies[0].body.clone();
            if chain
                .first()
                .is_some_and(|first| body.kind != Kind::Client || body.account != first.account)
            {
                return None;
            }
            let key = match body.issuer {
                None => self.master(&body.account)?,
                Some(issuer) => self.certificates(&issuer)?[0].body.signing,
            };
            if !copies.iter().any(|cert| cert.signed_by(&key)) {
                return None;
            }
            let issuer = body.issuer;
            chain.push(body);
            match issuer {
                None => return Some(chain),
                Some(issuer) => next = issuer,
            }
        }
        None
    }

    /// The revocations that count.
    fn counting(&self) -> Vec<Revocation> {
        self.revocations
            .iter()
            .filter_map(|revocation| Value::decode(revocation).ok())
            .filter_map(|revocation| revocation::counting(revocation, self).ok().flatten())
            .collect()
    }

    /// The certificate body of `device` if its certificate is valid: checks 1 to 4 of "Device
    /// checks".
    pub fn valid(&self, device: &DeviceId) -> Option<Body> {
        let chain = self.chain(device)?;
        let counting = self.counting();
        if counting.iter().any(|revocation| revocation.names(device)) {
            return None;
        }
        for pair in chain.windows(2) {
            let (below, issuer) = (&pair[0].device, &pair[1].device);
            if counting
                .iter()
                .any(|revocation| revocation.names(issuer) && !revocation.keep.contains(below))
            {
                return None;
            }
        }
        chain.into_iter().next()
    }

    /// The devices admitted to the channel: those with a valid certificate that are `client`
    /// devices of a member account its `excluded` does not list, or in the channel's `services`.
    pub fn admitted(&self) -> Admitted {
        let mut admitted = Admitted::default();
        for cert in &self.certificates {
            let Some(body) = self.valid(&cert.body.device) else {
                continue;
            };
            let member = body.kind == Kind::Client
                && self
                    .members
                    .get(&body.account)
                    .is_some_and(|excluded| !excluded.contains(&body.device));
            if member || self.services.contains(&body.device) {
                admitted.0.insert(body.device, body.signing);
            }
        }
        admitted
    }
}

/// The admitted devices of a channel and the signing keys of their certificates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Admitted(BTreeMap<DeviceId, [u8; 32]>);

impl Admitted {
    /// Whether the leaf's `identity` and `signature_key` match an admitted device's certificate.
    pub fn admits(&self, leaf: &Leaf) -> bool {
        DeviceId::try_from(leaf.identity.as_slice())
            .ok()
            .and_then(|device| self.0.get(&device))
            .is_some_and(|signing| signing[..] == leaf.signature_key[..])
    }

    /// Fails unless the leaf's device is admitted.
    pub fn check(&self, leaf: &Leaf) -> Result<()> {
        if self.admits(leaf) {
            Ok(())
        } else {
            Err(format!("device {:02x?} is not admitted", leaf.identity).into())
        }
    }

    /// These devices and the device of `leaf`: what a member with another view admits.
    pub fn with(mut self, leaf: &Leaf) -> Self {
        let device = leaf.identity.as_slice().try_into().expect("a device ID");
        let signing = leaf.signature_key.as_slice().try_into().expect("a key");
        self.0.insert(device, signing);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Identity,
        certificate::issue,
        object,
        revocation::{LABEL, revoke},
        scenarios::leaf,
    };

    const ACCOUNT: AccountId = [0xa1; 16];
    const OTHER: AccountId = [0xa2; 16];

    /// The roster of one member account with a master key.
    fn roster() -> (Roster, Identity) {
        let master = Identity::generate();
        let mut roster = Roster::default();
        roster.pin(ACCOUNT, master.public_key());
        roster.add_member(ACCOUNT);
        (roster, master)
    }

    /// A new device of `account` with a certificate by `issuer`: the master key if `by` is none.
    fn certify(
        roster: &mut Roster,
        account: AccountId,
        kind: Kind,
        issuer: &Identity,
        by: Option<&Identity>,
    ) -> Identity {
        let device = Identity::generate();
        let body = Body::new(account, &device, kind, by.map(Identity::device_id));
        roster
            .add_certificate(&issue(&body, by.unwrap_or(issuer), &device))
            .unwrap();
        device
    }

    /// A master-key revocation of `device` with `keep` and `also`.
    fn master_revocation(
        master: &Identity,
        device: &Identity,
        keep: &[&Identity],
        also: &[&Identity],
    ) -> Vec<u8> {
        let ids = |devices: &[&Identity]| {
            Value::Array(
                devices
                    .iter()
                    .map(|device| Value::Bytes(device.device_id().to_vec()))
                    .collect(),
            )
        };
        let mut entries = vec![
            (Value::Uint(1), Value::Bytes(ACCOUNT.to_vec())),
            (Value::Uint(2), Value::Bytes(device.device_id().to_vec())),
        ];
        if !keep.is_empty() {
            entries.push((Value::Uint(3), ids(keep)));
        }
        entries.push((Value::Uint(5), Value::Uint(1)));
        if !also.is_empty() {
            entries.push((Value::Uint(6), ids(also)));
        }
        object::sign(master, LABEL, Value::Map(entries).encode())
    }

    #[test]
    fn admits_client_devices_of_member_accounts_and_services() {
        let (mut roster, master) = roster();
        let client = certify(&mut roster, ACCOUNT, Kind::Client, &master, None);
        let linked = certify(&mut roster, ACCOUNT, Kind::Client, &master, Some(&client));
        let browser = certify(&mut roster, ACCOUNT, Kind::Browser, &master, Some(&client));
        let excluded = certify(&mut roster, ACCOUNT, Kind::Client, &master, None);
        roster.exclude(ACCOUNT, excluded.device_id());
        let service = certify(&mut roster, ACCOUNT, Kind::Service, &master, Some(&client));
        let other_service = certify(&mut roster, ACCOUNT, Kind::Service, &master, None);
        roster.add_service(service.device_id());
        let custodian = certify(&mut roster, ACCOUNT, Kind::Custodian, &master, None);

        let other_master = Identity::generate();
        roster.pin(OTHER, other_master.public_key());
        let outsider = certify(&mut roster, OTHER, Kind::Client, &other_master, None);
        let uncertified = Identity::generate();

        let admitted = roster.admitted();
        for (device, want, why) in [
            (&client, true, "a client by the master key"),
            (&linked, true, "a client by a client"),
            (&service, true, "a service in `services`"),
            (&browser, false, "a browser"),
            (&excluded, false, "an excluded client"),
            (&other_service, false, "a service not in `services`"),
            (&custodian, false, "a custodian"),
            (
                &outsider,
                false,
                "a client of an account that is not a member",
            ),
            (&uncertified, false, "a device without a certificate"),
        ] {
            assert_eq!(admitted.admits(&leaf(device)), want, "{why}");
            assert_eq!(admitted.check(&leaf(device)).is_ok(), want, "{why}");
        }
        roster.add_member(OTHER);
        assert!(
            roster.admitted().admits(&leaf(&outsider)),
            "a member's client"
        );
        // Adding an account again keeps its excluded devices.
        roster.add_member(ACCOUNT);
        assert!(!roster.admitted().admits(&leaf(&excluded)));
    }

    #[test]
    fn admits_a_leaf_only_with_its_certificates_key() {
        let (mut roster, master) = roster();
        let client = certify(&mut roster, ACCOUNT, Kind::Client, &master, None);
        let admitted = roster.admitted();
        let other = Identity::generate();
        for (leaf, why) in [
            (
                Leaf {
                    identity: client.device_id().to_vec(),
                    signature_key: other.public_key().to_vec(),
                },
                "another key",
            ),
            (
                Leaf {
                    identity: client.device_id()[..15].to_vec(),
                    signature_key: client.public_key().to_vec(),
                },
                "a short identity",
            ),
            (
                Leaf {
                    identity: client.device_id().to_vec(),
                    signature_key: client.public_key()[..31].to_vec(),
                },
                "a short key",
            ),
        ] {
            assert!(!admitted.admits(&leaf), "{why}");
            assert!(admitted.check(&leaf).is_err(), "{why}");
        }
        let with = admitted.clone().with(&leaf(&other));
        assert!(with.admits(&leaf(&other)) && with.admits(&leaf(&client)));
        assert!(!admitted.admits(&leaf(&other)));
    }

    #[test]
    fn a_chain_ends_at_the_pinned_master_key() {
        let (mut roster, master) = roster();
        let client = certify(&mut roster, ACCOUNT, Kind::Client, &master, None);
        let linked = certify(&mut roster, ACCOUNT, Kind::Client, &master, Some(&client));
        let chain = roster.chain(&linked.device_id()).unwrap();
        assert_eq!(
            chain.iter().map(|body| body.device).collect::<Vec<_>>(),
            [linked.device_id(), client.device_id()]
        );

        let forger = Identity::generate();
        let forged = certify(&mut roster, ACCOUNT, Kind::Client, &forger, None);
        let unpinned = certify(&mut roster, OTHER, Kind::Client, &forger, None);
        let browser = certify(&mut roster, ACCOUNT, Kind::Browser, &master, None);
        let by_browser = certify(&mut roster, ACCOUNT, Kind::Client, &master, Some(&browser));
        let by_forged = certify(&mut roster, ACCOUNT, Kind::Client, &master, Some(&forged));
        let other_master = Identity::generate();
        roster.pin(OTHER, other_master.public_key());
        let foreign = certify(&mut roster, OTHER, Kind::Client, &other_master, None);
        let by_foreign = certify(&mut roster, ACCOUNT, Kind::Client, &master, Some(&foreign));
        let by_unknown = certify(
            &mut roster,
            ACCOUNT,
            Kind::Client,
            &master,
            Some(&Identity::generate()),
        );
        for (device, why) in [
            (&forged, "signed by a key that is not the master key"),
            (&unpinned, "of an account without a pinned master key"),
            (&by_browser, "issued by a browser"),
            (&by_forged, "issued by a device without a valid chain"),
            (&by_foreign, "issued by a device of another account"),
            (&by_unknown, "issued by a device without a certificate"),
        ] {
            assert!(roster.chain(&device.device_id()).is_none(), "{why}");
            assert!(roster.valid(&device.device_id()).is_none(), "{why}");
        }
        assert!(roster.chain(&browser.device_id()).is_some());
        assert!(roster.chain(&foreign.device_id()).is_some());
    }

    #[test]
    fn a_chain_does_not_repeat_a_device() {
        let (mut roster, _) = roster();
        let (a, b) = (Identity::generate(), Identity::generate());
        for (device, issuer) in [(&a, &b), (&b, &a)] {
            let body = Body::new(ACCOUNT, device, Kind::Client, Some(issuer.device_id()));
            roster
                .add_certificate(&issue(&body, issuer, device))
                .unwrap();
        }
        assert!(roster.chain(&a.device_id()).is_none());
        let selfish = Identity::generate();
        let body = Body::new(ACCOUNT, &selfish, Kind::Client, Some(selfish.device_id()));
        roster
            .add_certificate(&issue(&body, &selfish, &selfish))
            .unwrap();
        assert!(roster.chain(&selfish.device_id()).is_none());
    }

    #[test]
    fn uses_only_certificates_that_pass_the_proof_check() {
        let (mut roster, master) = roster();
        let device = Identity::generate();
        let body = Body::new(ACCOUNT, &device, Kind::Client, None);
        // A certificate for the device's ID with its key, but a proof by another key.
        roster
            .add_certificate(&issue(&body, &master, &Identity::generate()))
            .unwrap();
        assert!(roster.chain(&device.device_id()).is_none());

        roster
            .add_certificate(&issue(&body, &master, &device))
            .unwrap();
        assert!(roster.chain(&device.device_id()).is_some());
        // A copy of the body whose signature does not verify changes nothing.
        roster
            .add_certificate(&issue(&body, &Identity::generate(), &device))
            .unwrap();
        assert!(roster.valid(&device.device_id()).is_some());
        assert!(roster.admitted().admits(&leaf(&device)));

        // Two bodies with the device's proof: neither is valid.
        let renamed = Body {
            name: "other".into(),
            ..body
        };
        roster
            .add_certificate(&issue(&renamed, &master, &device))
            .unwrap();
        assert!(roster.chain(&device.device_id()).is_none());
        assert!(!roster.admitted().admits(&leaf(&device)));

        assert!(roster.add_certificate(b"\x80").is_err());
    }

    #[test]
    fn revocations_that_count_revoke_a_device_and_those_it_certified() {
        let (mut roster, master) = roster();
        let client = certify(&mut roster, ACCOUNT, Kind::Client, &master, None);
        let kept = certify(&mut roster, ACCOUNT, Kind::Client, &master, Some(&client));
        let dropped = certify(&mut roster, ACCOUNT, Kind::Client, &master, Some(&client));
        let below_kept = certify(&mut roster, ACCOUNT, Kind::Client, &master, Some(&kept));
        let signer = certify(&mut roster, ACCOUNT, Kind::Client, &master, None);
        let also = certify(&mut roster, ACCOUNT, Kind::Client, &master, None);
        let outsider = Identity::generate();
        let all = [&client, &kept, &dropped, &below_kept, &signer, &also];
        for device in all {
            assert!(roster.valid(&device.device_id()).is_some());
        }

        // Ignored: by a device without a certificate, and malformed.
        roster.add_revocation(revoke(&outsider, &ACCOUNT, &client.device_id(), 1));
        roster.add_revocation(b"\x80".to_vec());
        roster.add_revocation(vec![0x81, 0x40]);
        for device in all {
            assert!(roster.valid(&device.device_id()).is_some());
        }

        roster.add_revocation(master_revocation(&master, &client, &[&kept], &[&also]));
        let valid: Vec<_> = all
            .iter()
            .map(|device| roster.valid(&device.device_id()).is_some())
            .collect();
        assert_eq!(valid, [false, true, false, true, true, false]);

        // Every counting revocation of an issuer has to keep the device below it.
        roster.add_revocation(revoke(&signer, &ACCOUNT, &client.device_id(), 1));
        assert!(roster.valid(&kept.device_id()).is_none());
        assert!(roster.valid(&below_kept.device_id()).is_none());
        assert!(roster.valid(&signer.device_id()).is_some());
        assert!(!roster.admitted().admits(&leaf(&kept)));
        assert!(roster.admitted().admits(&leaf(&signer)));
    }
}
