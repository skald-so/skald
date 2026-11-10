use std::{collections::BTreeMap, marker::PhantomData};

use crate::{
    Admitted, Device, Identity, Result, Roster,
    certificate::{self, Body, Kind},
    revocation::AccountId,
    scenarios::ACCOUNT,
};

/// Devices on one library, kept only as their signing keys and saved states, and the [`Roster`]
/// they share: the channel's one member account is [`ACCOUNT`], whose master key certifies each
/// device [`Fleet::add_device`] adds as a `client` device.
pub struct Fleet<D> {
    devices: Vec<Saved>,
    roster: Roster,
    masters: BTreeMap<AccountId, Identity>,
    device: PhantomData<D>,
}

struct Saved {
    identity: Identity,
    state: Vec<u8>,
}

impl<D: Device> Fleet<D> {
    /// `count` new devices, each with a new signing key and a certificate.
    pub fn new(count: usize) -> Result<Self> {
        let mut fleet = Self {
            devices: Vec::new(),
            roster: Roster::default(),
            masters: BTreeMap::new(),
            device: PhantomData,
        };
        fleet.roster.add_member(ACCOUNT);
        for _ in 0..count {
            fleet.add_device()?;
        }
        Ok(fleet)
    }

    /// Adds a new `client` device of [`ACCOUNT`]; returns its index.
    pub fn add_device(&mut self) -> Result<usize> {
        let device = self.add_device_without_certificate()?;
        self.certify(device, ACCOUNT, Kind::Client)?;
        Ok(device)
    }

    /// Adds a new device without a certificate; returns its index.
    pub fn add_device_without_certificate(&mut self) -> Result<usize> {
        let identity = Identity::generate();
        let state = D::new(&identity)?.save()?;
        self.devices.push(Saved { identity, state });
        Ok(self.devices.len() - 1)
    }

    /// Gives the device a certificate of `kind` on `account`, by the account's master key. An
    /// account gets its master key, pinned in the roster, with its first certificate.
    pub fn certify(&mut self, device: usize, account: AccountId, kind: Kind) -> Result<()> {
        let master = self
            .masters
            .entry(account)
            .or_insert_with(Identity::generate);
        self.roster.pin(account, master.public_key());
        let identity = &self.devices[device].identity;
        let body = Body::new(account, identity, kind, None);
        self.roster
            .add_certificate(&certificate::issue(&body, master, identity))
    }

    pub fn identity(&self, device: usize) -> &Identity {
        &self.devices[device].identity
    }

    /// The state the device saved after its last successful step.
    pub fn state(&self, device: usize) -> &[u8] {
        &self.devices[device].state
    }

    pub fn roster(&self) -> &Roster {
        &self.roster
    }

    pub fn roster_mut(&mut self) -> &mut Roster {
        &mut self.roster
    }

    /// The devices the roster admits to the channel.
    pub fn admitted(&self) -> Admitted {
        self.roster.admitted()
    }

    /// One step of a device, as a one-shot run: loads its saved state, runs `step` and saves the
    /// state if the step succeeds. A failed step leaves the saved state as it was.
    pub fn step<T>(&mut self, device: usize, step: impl FnOnce(&mut D) -> Result<T>) -> Result<T> {
        let saved = &mut self.devices[device];
        let mut loaded = D::load(&saved.identity, &saved.state)?;
        let output = step(&mut loaded)?;
        saved.state = loaded.save()?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Commit, HistoryKey, Joining, Leaf, Received, Staged, scenarios::leaf};

    const OTHER: AccountId = [0xa2; 16];

    /// A device whose whole state is a counter of its steps.
    struct Counter(u8);

    impl Device for Counter {
        const LIBRARY: &'static str = "counter";
        type Observer = crate::delivery::tests::Unused;

        fn new(_: &Identity) -> Result<Self> {
            Ok(Self(0))
        }
        fn load(_: &Identity, state: &[u8]) -> Result<Self> {
            Ok(Self(state[0]))
        }
        fn save(&self) -> Result<Vec<u8>> {
            Ok(vec![self.0])
        }
        fn key_package(&mut self) -> Result<Vec<u8>> {
            self.0 += 1;
            Ok(vec![self.0])
        }
        fn create_group(&mut self, _: &[u8]) -> Result<()> {
            self.0 += 1;
            Err("fails".into())
        }
        fn last_resort_key_package(&mut self) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn add(&mut self, _: &[u8], _: &[Vec<u8>], _: &Admitted) -> Result<Commit> {
            unimplemented!()
        }
        fn remove(&mut self, _: &[u8], _: &[Vec<u8>], _: &[u8]) -> Result<Commit> {
            unimplemented!()
        }
        fn empty_commit(&mut self, _: &[u8], _: &[u8]) -> Result<Commit> {
            unimplemented!()
        }
        fn merge_pending_commit(&mut self, _: &[u8]) -> Result<()> {
            unimplemented!()
        }
        fn clear_pending_commit(&mut self, _: &[u8]) -> Result<()> {
            unimplemented!()
        }
        fn delete_group(&mut self, _: &[u8]) -> Result<()> {
            unimplemented!()
        }
        fn join_with(
            &mut self,
            _: &[u8],
            _: impl FnOnce(&Joining) -> Result<()>,
        ) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn join_external(
            &mut self,
            _: &[u8],
            _: impl FnOnce(&Joining) -> Result<()>,
        ) -> Result<Commit> {
            unimplemented!()
        }
        fn encrypt(&mut self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn process_with(
            &mut self,
            _: &[u8],
            _: &[u8],
            _: impl FnOnce(&Staged) -> Result<()>,
        ) -> Result<Received> {
            unimplemented!()
        }
        fn epoch(&self, _: &[u8]) -> Result<u64> {
            unimplemented!()
        }
        fn members(&self, _: &[u8]) -> Result<Vec<Leaf>> {
            unimplemented!()
        }
        fn history_key(&self, _: &[u8]) -> Result<HistoryKey> {
            unimplemented!()
        }
    }

    #[test]
    fn a_step_saves_the_state_only_on_success() {
        let mut fleet = Fleet::<Counter>::new(2).unwrap();
        assert_eq!(fleet.step(0, |d| d.key_package()).unwrap(), [1]);
        assert_eq!(fleet.step(0, |d| d.key_package()).unwrap(), [2]);
        assert_eq!(fleet.state(0), [2]);
        assert_eq!(fleet.state(1), [0]);

        assert!(fleet.step(0, |d| d.create_group(b"g")).is_err());
        assert_eq!(fleet.state(0), [2]);
        assert_eq!(fleet.step(0, |d| d.key_package()).unwrap(), [3]);
    }

    #[test]
    fn each_device_has_its_own_key() {
        let mut fleet = Fleet::<Counter>::new(1).unwrap();
        let added = fleet.add_device().unwrap();
        assert_eq!(added, 1);
        assert_ne!(fleet.identity(0).device_id(), fleet.identity(1).device_id());
    }

    #[test]
    fn admits_the_certified_devices_of_its_account() {
        let mut fleet = Fleet::<Counter>::new(2).unwrap();
        let uncertified = fleet.add_device_without_certificate().unwrap();
        let other = fleet.add_device_without_certificate().unwrap();
        fleet.certify(other, OTHER, Kind::Client).unwrap();
        let browser = fleet.add_device_without_certificate().unwrap();
        fleet.certify(browser, ACCOUNT, Kind::Browser).unwrap();
        assert_eq!(fleet.state(uncertified), [0]);

        let admitted = fleet.admitted();
        assert_eq!(admitted, fleet.roster().admitted());
        for (device, want) in [
            (0, true),
            (1, true),
            (uncertified, false),
            (other, false),
            (browser, false),
        ] {
            assert_eq!(
                admitted.admits(&leaf(fleet.identity(device))),
                want,
                "device {device}"
            );
        }
        let (a, b) = (
            fleet
                .roster()
                .valid(&fleet.identity(0).device_id())
                .unwrap(),
            fleet
                .roster()
                .valid(&fleet.identity(other).device_id())
                .unwrap(),
        );
        assert_eq!((a.account, b.account), (ACCOUNT, OTHER));
        assert!(
            fleet.roster().master(&ACCOUNT).is_some()
                && fleet.roster().master(&OTHER) != fleet.roster().master(&ACCOUNT)
        );

        fleet.roster_mut().add_member(OTHER);
        assert!(fleet.admitted().admits(&leaf(fleet.identity(other))));
    }
}
