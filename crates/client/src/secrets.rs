//! The client's secrets, such as the session token: in the system's key storage through the keyring
//! project's stores (`architecture.md`, "Cryptography"), or without one in a file with 0600 permissions in the state
//! directory. They are kept as one JSON object of names and values.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The secrets file in the state directory.
pub const FILE: &str = "secrets.json";
/// The keyring service of the client's entries; an entry's user is its state directory.
const SERVICE: &str = "skald";
/// The setting that lets a test use the system's own key storage: `1`, and only on a machine whose
/// key storage nothing else uses, such as a fresh CI runner. Otherwise no test touches it.
#[cfg(test)]
const REAL_KEY_STORAGE: &str = "SKALD_TEST_REAL_KEY_STORAGE";

/// The secrets by name.
pub type Map = BTreeMap<String, String>;

/// Where secrets go.
#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum)]
pub enum Storage {
    /// The system's key storage; the file when the system has none.
    Auto,
    /// The file only.
    File,
}

/// Why the system's key storage failed.
#[derive(Debug)]
pub enum KeyringError {
    /// The system has no key storage the client can use: secrets go to the file.
    Unavailable,
    /// The key storage failed.
    Failed(String),
}

/// A key storage entry holding the secrets.
pub trait Keyring {
    /// The entry's value, if it has one.
    fn get(&self) -> Result<Option<String>, KeyringError>;
    fn set(&self, value: &str) -> Result<(), KeyringError>;
}

/// The system's key storage, through the keyring project's store for the platform: the entry of
/// `service` and `user`.
struct System {
    service: String,
    user: String,
}

impl System {
    fn entry(&self) -> Result<keyring_core::Entry, KeyringError> {
        platform_store()
            .and_then(|store| store.build(&self.service, &self.user, None))
            .map_err(keyring_error)
    }
}

/// The platform's key storage: Keychain, the Secret Service or Windows Credential Manager.
fn platform_store() -> keyring_core::Result<Arc<keyring_core::api::CredentialStore>> {
    #[cfg(target_os = "macos")]
    let store: Arc<keyring_core::api::CredentialStore> =
        apple_native_keyring_store::keychain::Store::new()?;
    #[cfg(windows)]
    let store: Arc<keyring_core::api::CredentialStore> =
        windows_native_keyring_store::Store::new()?;
    #[cfg(all(unix, not(target_os = "macos")))]
    let store: Arc<keyring_core::api::CredentialStore> =
        dbus_secret_service_keyring_store::Store::new()?;
    #[cfg(any(unix, windows))]
    return Ok(store);
    #[cfg(not(any(unix, windows)))]
    Err(keyring_core::Error::NoDefaultStore)
}

impl Keyring for System {
    fn get(&self) -> Result<Option<String>, KeyringError> {
        match self.entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(err) => Err(keyring_error(err)),
        }
    }

    fn set(&self, value: &str) -> Result<(), KeyringError> {
        self.entry()?.set_password(value).map_err(keyring_error)
    }
}

/// A key storage that is missing, unreachable or locked is unavailable: no D-Bus or Secret Service,
/// for one. Any other error is a failure.
fn keyring_error(err: keyring_core::Error) -> KeyringError {
    match err {
        keyring_core::Error::NoDefaultStore
        | keyring_core::Error::NoStorageAccess(_)
        | keyring_core::Error::PlatformFailure(_) => KeyringError::Unavailable,
        err => KeyringError::Failed(err.to_string()),
    }
}

/// Why the secrets cannot be read or kept.
#[derive(Debug)]
pub enum Error {
    File(PathBuf, io::Error),
    Keyring(String),
    /// The stored secrets are not a JSON object of strings.
    Malformed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::File(path, err) => {
                write!(f, "the secrets file {} failed: {err}", path.display())
            }
            Error::Keyring(err) => write!(f, "the system's key storage failed: {err}"),
            Error::Malformed(place) => write!(f, "the secrets in {place} are malformed"),
        }
    }
}

pub struct Secrets {
    file: PathBuf,
    keyring: Option<Box<dyn Keyring>>,
}

impl Secrets {
    /// The secrets of the state directory `dir`, in `storage`.
    pub fn new(dir: &Path, storage: Storage) -> Self {
        Self::of_service(dir, storage, SERVICE)
    }

    /// The secrets of the state directory `dir`, in `storage`, under the keyring service `service`.
    fn of_service(dir: &Path, storage: Storage, service: &str) -> Self {
        let keyring = match storage {
            Storage::Auto => Some(Box::new(System {
                service: service.to_owned(),
                user: dir.display().to_string(),
            }) as Box<dyn Keyring>),
            Storage::File => None,
        };
        Self::with(dir, keyring)
    }

    /// The secrets of the state directory `dir`, in `keyring` if given, else in the file.
    pub fn with(dir: &Path, keyring: Option<Box<dyn Keyring>>) -> Self {
        Secrets {
            file: dir.join(FILE),
            keyring,
        }
    }

    /// The stored secrets: the key storage's if it has them, otherwise the file's, if any.
    pub fn load(&self) -> Result<Map, Error> {
        if let Some(keyring) = &self.keyring {
            match keyring.get() {
                Ok(Some(value)) => return parse(&value, "the system's key storage"),
                Ok(None) | Err(KeyringError::Unavailable) => {}
                Err(KeyringError::Failed(err)) => return Err(Error::Keyring(err)),
            }
        }
        match fs::read_to_string(&self.file) {
            Ok(value) => parse(&value, &self.file.display().to_string()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Map::new()),
            Err(err) => Err(Error::File(self.file.clone(), err)),
        }
    }

    /// Keeps `secrets` instead of the stored ones: in the key storage, removing the file, or
    /// without one in the file.
    pub fn save(&self, secrets: &Map) -> Result<(), Error> {
        let value = serde_json::to_string(secrets).expect("a map of strings is JSON");
        if let Some(keyring) = &self.keyring {
            match keyring.set(&value) {
                Ok(()) => return self.remove_file(),
                Err(KeyringError::Unavailable) => {}
                Err(KeyringError::Failed(err)) => return Err(Error::Keyring(err)),
            }
        }
        self.write_file(&value)
            .map_err(|err| Error::File(self.file.clone(), err))
    }

    fn remove_file(&self) -> Result<(), Error> {
        match fs::remove_file(&self.file) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => {
                Err(Error::File(self.file.clone(), err))
            }
            _ => Ok(()),
        }
    }

    /// Writes the file whole or not at all: a new file with 0600 permissions, renamed over it.
    fn write_file(&self, value: &str) -> io::Result<()> {
        let new = self.file.with_extension("json.new");
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&new)?;
        // The mode applies only to a file the open creates.
        #[cfg(unix)]
        file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        file.write_all(value.as_bytes())?;
        file.sync_all()?;
        fs::rename(&new, &self.file)
    }
}

fn parse(value: &str, place: &str) -> Result<Map, Error> {
    serde_json::from_str(value).map_err(|_| Error::Malformed(place.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::test_dir::TestDir;

    /// A stand-in key storage: its entry's value, or its answer to every call.
    #[derive(Clone)]
    struct StandIn {
        value: Rc<RefCell<Option<String>>>,
        broken: Option<fn() -> KeyringError>,
    }

    impl StandIn {
        fn new(broken: Option<fn() -> KeyringError>) -> Self {
            StandIn {
                value: Rc::new(RefCell::new(None)),
                broken,
            }
        }
    }

    impl Keyring for StandIn {
        fn get(&self) -> Result<Option<String>, KeyringError> {
            match self.broken {
                Some(error) => Err(error()),
                None => Ok(self.value.borrow().clone()),
            }
        }

        fn set(&self, value: &str) -> Result<(), KeyringError> {
            match self.broken {
                Some(error) => Err(error()),
                None => {
                    *self.value.borrow_mut() = Some(value.to_owned());
                    Ok(())
                }
            }
        }
    }

    fn token(value: &str) -> Map {
        Map::from([("session_token".to_owned(), value.to_owned())])
    }

    #[test]
    fn nothing_stored_loads_empty() {
        let dir = TestDir::new("secrets-empty");
        assert_eq!(
            Secrets::new(&dir, Storage::File).load().unwrap(),
            Map::new()
        );
        let keyring = StandIn::new(None);
        let secrets = Secrets::with(&dir, Some(Box::new(keyring)));
        assert_eq!(secrets.load().unwrap(), Map::new());
    }

    #[cfg(unix)]
    #[test]
    fn file_has_0600_permissions_and_holds_the_secrets() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TestDir::new("secrets-file");
        // A file left with other permissions gets 0600 too.
        fs::write(dir.join(FILE), "{}").unwrap();
        fs::set_permissions(dir.join(FILE), fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(dir.join("secrets.json.new"), "{}").unwrap();
        fs::set_permissions(
            dir.join("secrets.json.new"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let secrets = Secrets::new(&dir, Storage::File);
        secrets.save(&token("one")).unwrap();

        let mode = fs::metadata(dir.join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        assert!(!dir.join("secrets.json.new").exists());
        assert_eq!(secrets.load().unwrap(), token("one"));
        secrets.save(&token("two")).unwrap();
        assert_eq!(
            Secrets::new(&dir, Storage::File).load().unwrap(),
            token("two")
        );
    }

    #[test]
    fn key_storage_holds_the_secrets_and_the_file_goes() {
        let dir = TestDir::new("secrets-keyring");
        Secrets::new(&dir, Storage::File)
            .save(&token("old"))
            .unwrap();
        let keyring = StandIn::new(None);
        let secrets = Secrets::with(&dir, Some(Box::new(keyring.clone())));
        // Secrets kept in the file before the key storage was there are read from the file.
        assert_eq!(secrets.load().unwrap(), token("old"));

        secrets.save(&token("new")).unwrap();

        assert!(!dir.join(FILE).exists(), "the file is removed");
        assert_eq!(
            keyring.value.borrow().as_deref(),
            Some(r#"{"session_token":"new"}"#)
        );
        assert_eq!(secrets.load().unwrap(), token("new"));
    }

    #[test]
    fn without_a_key_storage_the_file_holds_them() {
        let dir = TestDir::new("secrets-unavailable");
        let keyring = StandIn::new(Some(|| KeyringError::Unavailable));
        let secrets = Secrets::with(&dir, Some(Box::new(keyring)));

        secrets.save(&token("one")).unwrap();

        assert_eq!(
            Secrets::new(&dir, Storage::File).load().unwrap(),
            token("one")
        );
        assert_eq!(secrets.load().unwrap(), token("one"));
    }

    #[test]
    fn failing_key_storage_is_an_error_and_nothing_goes_to_the_file() {
        let dir = TestDir::new("secrets-failed");
        let keyring = StandIn::new(Some(|| KeyringError::Failed("locked".to_owned())));
        let secrets = Secrets::with(&dir, Some(Box::new(keyring)));

        let error = secrets.save(&token("one")).expect_err("refused");
        assert!(
            matches!(&error, Error::Keyring(err) if err == "locked"),
            "{error:?}"
        );
        assert!(!dir.join(FILE).exists());
        let error = secrets.load().expect_err("refused");
        assert!(
            matches!(&error, Error::Keyring(err) if err == "locked"),
            "{error:?}"
        );
    }

    #[test]
    fn malformed_secrets_are_an_error() {
        let dir = TestDir::new("secrets-malformed");
        fs::write(dir.join(FILE), "not json").unwrap();
        let error = Secrets::new(&dir, Storage::File)
            .load()
            .expect_err("malformed");
        assert!(matches!(error, Error::Malformed(_)), "{error:?}");

        let keyring = StandIn::new(None);
        keyring.set("[1]").unwrap();
        let error = Secrets::with(&dir, Some(Box::new(keyring)))
            .load()
            .expect_err("malformed");
        assert!(matches!(error, Error::Malformed(_)), "{error:?}");
    }

    #[test]
    fn unreadable_file_is_an_error() {
        let dir = TestDir::new("secrets-unreadable");
        fs::create_dir(dir.join(FILE)).unwrap();
        let error = Secrets::new(&dir, Storage::File)
            .load()
            .expect_err("a directory");
        assert!(matches!(error, Error::File(..)), "{error:?}");
    }

    #[test]
    fn key_storage_takes_the_secrets_with_or_without_a_file_and_fails_on_one_it_cannot_remove() {
        let dir = TestDir::new("secrets-remove");
        let secrets = Secrets::with(&dir, Some(Box::new(StandIn::new(None))));
        secrets.save(&token("one")).unwrap();

        fs::create_dir(dir.join(FILE)).unwrap();
        let error = secrets.save(&token("two")).expect_err("the file stays");
        assert!(matches!(error, Error::File(..)), "{error:?}");
    }

    /// An entry of the system's own key storage, deleted on drop: a test leaves nothing behind,
    /// also when it fails.
    struct SystemEntry(System);

    impl Drop for SystemEntry {
        fn drop(&mut self) {
            if let Ok(entry) = self.0.entry() {
                let _ = entry.delete_credential();
            }
        }
    }

    /// The system's own key storage in auto mode, only with SKALD_TEST_REAL_KEY_STORAGE=1: a
    /// device's keys and a session token go into it, under a keyring service of the test's own,
    /// and not into the file; a later run reads them back; once the entry is deleted, nothing is
    /// stored.
    #[test]
    fn system_key_storage_keeps_the_device_keys() {
        if std::env::var(REAL_KEY_STORAGE).as_deref() != Ok("1") {
            return;
        }
        let dir = TestDir::new("secrets-system");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let service = format!("skald-test-{}-{nanos}", std::process::id());
        let entry = SystemEntry(System {
            service: service.clone(),
            user: dir.display().to_string(),
        });
        assert!(matches!(entry.0.get(), Ok(None)), "a new entry");
        let keys = crate::device::Keys::generate();
        let mut stored = token("tok-system-0123456789abcdefghijkl");
        keys.store(&mut stored);

        Secrets::of_service(&dir, Storage::Auto, &service)
            .save(&stored)
            .unwrap();

        assert!(!dir.join(FILE).exists(), "the secrets went to the file");
        let held = entry.0.get().unwrap().expect("the entry holds the secrets");
        assert_eq!(parse(&held, "the entry").unwrap(), stored);
        let loaded = Secrets::of_service(&dir, Storage::Auto, &service)
            .load()
            .unwrap();
        assert_eq!(loaded, stored);
        let read = crate::device::Keys::load(&loaded).unwrap().expect("keys");
        assert_eq!(read.id(), keys.id());

        entry.0.entry().unwrap().delete_credential().unwrap();
        assert!(matches!(entry.0.get(), Ok(None)), "the entry is deleted");
        assert_eq!(
            Secrets::of_service(&dir, Storage::Auto, &service)
                .load()
                .unwrap(),
            Map::new()
        );
    }

    #[test]
    fn errors_say_what_failed() {
        let file = PathBuf::from("/state/secrets.json");
        let error = Error::File(file, io::Error::from(io::ErrorKind::PermissionDenied));
        assert!(
            error
                .to_string()
                .starts_with("the secrets file /state/secrets.json failed: "),
            "{error}"
        );
        assert_eq!(
            Error::Keyring("locked".to_owned()).to_string(),
            "the system's key storage failed: locked"
        );
        assert_eq!(
            Error::Malformed("the file".to_owned()).to_string(),
            "the secrets in the file are malformed"
        );
    }

    #[test]
    fn key_storage_errors_are_told_apart() {
        assert!(matches!(
            keyring_error(keyring_core::Error::NoDefaultStore),
            KeyringError::Unavailable
        ));
        assert!(matches!(
            keyring_error(keyring_core::Error::NoStorageAccess("no bus".into())),
            KeyringError::Unavailable
        ));
        assert!(matches!(
            keyring_error(keyring_core::Error::PlatformFailure(
                "no secret service".into()
            )),
            KeyringError::Unavailable
        ));
        assert!(matches!(
            keyring_error(keyring_core::Error::BadEncoding(vec![0x80])),
            KeyringError::Failed(_)
        ));
    }
}
