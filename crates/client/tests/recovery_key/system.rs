//! The system's key storage entry of a machine, as the client names it: the service `skald`, the
//! user the machine's state directory, its absolute path. Deleted on drop.

use std::sync::Arc;

use keyring_core::api::CredentialStore;

use crate::machine::Machine;

pub struct Entry(keyring_core::Entry);

impl Entry {
    pub fn of(machine: &Machine) -> Self {
        let dir = machine
            .state_dir()
            .canonicalize()
            .expect("the state directory");
        let entry = store()
            .build("skald", &dir.display().to_string(), None)
            .expect("a key storage entry");
        Entry(entry)
    }

    pub fn value(&self) -> Option<String> {
        match self.0.get_password() {
            Ok(value) => Some(value),
            Err(keyring_core::Error::NoEntry) => None,
            Err(err) => panic!("the system's key storage: {err}"),
        }
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        let _ = self.0.delete_credential();
    }
}

fn store() -> Arc<CredentialStore> {
    #[cfg(target_os = "macos")]
    let store: Arc<CredentialStore> =
        apple_native_keyring_store::keychain::Store::new().expect("the Keychain");
    #[cfg(windows)]
    let store: Arc<CredentialStore> =
        windows_native_keyring_store::Store::new().expect("Credential Manager");
    #[cfg(all(unix, not(target_os = "macos")))]
    let store: Arc<CredentialStore> =
        dbus_secret_service_keyring_store::Store::new().expect("the Secret Service");
    store
}
