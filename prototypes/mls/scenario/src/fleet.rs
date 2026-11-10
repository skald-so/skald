use std::marker::PhantomData;

use crate::{Device, Identity, Result};

/// Devices on one library, kept only as their signing keys and saved states.
pub struct Fleet<D> {
    devices: Vec<Saved>,
    device: PhantomData<D>,
}

struct Saved {
    identity: Identity,
    state: Vec<u8>,
}

impl<D: Device> Fleet<D> {
    /// `count` new devices, each with a new signing key.
    pub fn new(count: usize) -> Result<Self> {
        let mut fleet = Self {
            devices: Vec::new(),
            device: PhantomData,
        };
        for _ in 0..count {
            fleet.add_device()?;
        }
        Ok(fleet)
    }

    /// Adds a new device; returns its index.
    pub fn add_device(&mut self) -> Result<usize> {
        let identity = Identity::generate();
        let state = D::new(&identity)?.save()?;
        self.devices.push(Saved { identity, state });
        Ok(self.devices.len() - 1)
    }

    pub fn identity(&self, device: usize) -> &Identity {
        &self.devices[device].identity
    }

    /// The state the device saved after its last successful step.
    pub fn state(&self, device: usize) -> &[u8] {
        &self.devices[device].state
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
    use crate::{Commit, Leaf, Received};

    /// A device whose whole state is a counter of its steps.
    struct Counter(u8);

    impl Device for Counter {
        const LIBRARY: &'static str = "counter";

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
        fn add(&mut self, _: &[u8], _: &[Vec<u8>]) -> Result<Commit> {
            unimplemented!()
        }
        fn merge_pending_commit(&mut self, _: &[u8]) -> Result<()> {
            unimplemented!()
        }
        fn join(&mut self, _: &[u8]) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn encrypt(&mut self, _: &[u8], _: &[u8]) -> Result<Vec<u8>> {
            unimplemented!()
        }
        fn process(&mut self, _: &[u8], _: &[u8]) -> Result<Received> {
            unimplemented!()
        }
        fn epoch(&self, _: &[u8]) -> Result<u64> {
            unimplemented!()
        }
        fn members(&self, _: &[u8]) -> Result<Vec<Leaf>> {
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
}
