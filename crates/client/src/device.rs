//! The machine's own device (`docs/protocol/membership.md`, "Linking a device"): its keys, kept with
//! the secrets, its `name`, its registration with the server and, on the account's first device,
//! the account master key, the certificate it issues, the account record it signs and the master
//! backup that seals it to the recovery key.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use skald_crypto::{HpkeKey, RecoveryKey, SigningKey, StateKey};
use skald_proto::{
    AccountBody, AccountId, CERT_NAME_MAX, Cbor as _, CertBody, DeviceId, DeviceKind, Unknown,
};
use zeroize::Zeroizing;

use crate::connection::{self, Connection, Method};
use crate::link_code;
use crate::person::Person;
use crate::recovery;
use crate::recovery_link;
use crate::secrets::{self, Secrets};
use crate::sign_in;
use crate::spaces::now;
use crate::state::{self, Link, State};

/// The secrets that hold the device's private keys: its Ed25519 signing key and its X25519 HPKE
/// key, each as its 32 bytes in base64url without padding.
const SIGNING_KEY: &str = "device_signing_key";
const HPKE_KEY: &str = "device_hpke_key";
/// The secret that holds the key that seals the device's MLS state, its 32 bytes the same way.
const STATE_KEY: &str = "mls_state_key";
/// The `name` of a device whose host name is empty.
const UNNAMED: &str = "skald device";

/// The problem of a certificate beyond the devices an account may have.
pub const DEVICES_LIMIT: &str = "https://skald.so/problems/devices-limit";

/// Why the device cannot be set up, beyond the server, the state or the secrets.
#[derive(Debug)]
pub enum Error {
    /// The secrets lack a private key of the device.
    MissingKeys,
    /// The secrets hold a private key of the device that is not one.
    UnreadableKeys,
    /// The secrets hold the keys of another device than the one the state knows.
    OtherKeys(DeviceId),
    /// The secrets lack the key of the device's MLS state, which the local state holds.
    MissingStateKey,
    /// A `name` the user gave is not 1 to 64 bytes.
    Name(String),
    /// The device is named already, and the user gave another `name`.
    Renamed(String),
    /// The server has the device's ID with other keys or `name`, or on another account.
    DeviceTaken,
    /// The server refused the device's certificate: the account has as many devices as the server
    /// lets it have.
    DevicesLimit,
    /// The device is to link with the recovery key, and the account has no master key.
    NoMasterKey,
    /// The certificate it issued fails the checks.
    Cert(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::MissingKeys => f.write_str(
                "the secrets lack this device's private keys; skald never replaces them: \
                 restore the secrets that held them",
            ),
            Error::UnreadableKeys => f.write_str(
                "the secrets hold this device's private keys in a form skald cannot read; skald \
                 never replaces them: restore the secrets that held them",
            ),
            Error::OtherKeys(device) => write!(
                f,
                "the private keys in the secrets are not those of this device, {device}; skald \
                 never replaces them: restore the secrets that held them"
            ),
            Error::MissingStateKey => f.write_str(
                "the secrets lack the key of this device's MLS state; skald never replaces it: \
                 restore the secrets that held it",
            ),
            Error::Name(name) => write!(
                f,
                "the device's name {:?} is not 1 to {CERT_NAME_MAX} bytes",
                connection::printable(name)
            ),
            Error::Renamed(name) => write!(
                f,
                "this device is named {:?} already; its name does not change",
                connection::printable(name)
            ),
            Error::DeviceTaken => f.write_str(
                "the server has this device's ID with other keys or name, or on another account",
            ),
            Error::DevicesLimit => f.write_str(
                "the account has as many devices as it may on this server, so the server does not \
                 store this device's certificate yet; once a device of the account is revoked, the \
                 next skald command on this machine stores it",
            ),
            Error::NoMasterKey => f.write_str(
                "the account has no master key, so no recovery key: run skald sign-in without \
                 --with-recovery-key",
            ),
            Error::Cert(err) => write!(f, "the certificate this device issued fails: {err}"),
        }
    }
}

/// The device's private keys.
pub struct Keys {
    signing: SigningKey,
    hpke: HpkeKey,
}

/// The 32 bytes of the secret `name` in `stored`, if it holds one.
///
/// # Errors
/// If it is not a key of 32 bytes in base64url without padding.
fn stored_key(stored: &secrets::Map, name: &str) -> Result<Option<Zeroizing<[u8; 32]>>, Error> {
    let Some(value) = stored.get(name) else {
        return Ok(None);
    };
    let bytes = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| Error::UnreadableKeys)?,
    );
    let mut key = Zeroizing::new([0; 32]);
    if bytes.len() != key.len() {
        return Err(Error::UnreadableKeys);
    }
    key.copy_from_slice(&bytes);
    Ok(Some(key))
}

/// The key that seals the device's MLS state, as `stored`, the secrets, holds it. Without one, a
/// new key put into `stored` and given as new, unless `held`, whether the local state holds an MLS
/// state sealed under the missing key.
///
/// # Errors
/// If the key is unreadable, or missing while `held`.
pub fn state_key(stored: &mut secrets::Map, held: bool) -> Result<(StateKey, bool), Error> {
    if let Some(key) = stored_key(stored, STATE_KEY)? {
        return Ok((StateKey::from_bytes(&key), false));
    }
    if held {
        return Err(Error::MissingStateKey);
    }
    let key = StateKey::generate();
    stored.insert(
        STATE_KEY.to_owned(),
        URL_SAFE_NO_PAD.encode(*key.to_bytes()),
    );
    Ok((key, true))
}

impl Keys {
    /// New keys from the operating system's cryptographically secure generator.
    pub fn generate() -> Self {
        Keys {
            signing: SigningKey::generate(),
            hpke: HpkeKey::generate(),
        }
    }

    /// The keys `stored` holds, or `None` if it holds neither.
    ///
    /// # Errors
    /// If it holds one only, or one that is not a key of 32 bytes.
    pub fn load(stored: &secrets::Map) -> Result<Option<Self>, Error> {
        match (
            stored_key(stored, SIGNING_KEY)?,
            stored_key(stored, HPKE_KEY)?,
        ) {
            (None, None) => Ok(None),
            (Some(signing), Some(hpke)) => Ok(Some(Keys {
                signing: SigningKey::from_bytes(&signing),
                hpke: HpkeKey::from_bytes(&hpke),
            })),
            _ => Err(Error::MissingKeys),
        }
    }

    /// The keys of a `service` device, the private keys `signing` and `hpke` of its key secret.
    pub fn of_service(signing: &[u8; 32], hpke: &[u8; 32]) -> Self {
        Keys {
            signing: SigningKey::from_bytes(signing),
            hpke: HpkeKey::from_bytes(hpke),
        }
    }

    /// Puts the keys, and the key `state_key` that seals the device's MLS state, into `stored`.
    pub fn store_with(&self, stored: &mut secrets::Map, state_key: &StateKey) {
        self.store(stored);
        stored.insert(
            STATE_KEY.to_owned(),
            URL_SAFE_NO_PAD.encode(*state_key.to_bytes()),
        );
    }

    /// Puts the keys into `stored`.
    pub fn store(&self, stored: &mut secrets::Map) {
        stored.insert(
            SIGNING_KEY.to_owned(),
            URL_SAFE_NO_PAD.encode(*self.signing.to_bytes()),
        );
        stored.insert(
            HPKE_KEY.to_owned(),
            URL_SAFE_NO_PAD.encode(*self.hpke.to_bytes()),
        );
    }

    /// The device's ID, derived from its signing key.
    pub fn id(&self) -> DeviceId {
        DeviceId::from_signing_key(&self.signing.public_key())
    }

    /// The device's signing key.
    pub fn signing(&self) -> &SigningKey {
        &self.signing
    }

    /// The device's HPKE key.
    pub fn hpke(&self) -> &HpkeKey {
        &self.hpke
    }

    /// The body of the device's certificate that the account's master key issues: a `client`
    /// device named `name` on `account`, without `issuer`, issued now.
    pub fn master_cert_body(&self, account: AccountId, name: String) -> CertBody {
        CertBody {
            account,
            device: self.id(),
            kind: DeviceKind::Client,
            signing: self.signing.public_key(),
            hpke: self.hpke.public_key(),
            issuer: None,
            issued: now(),
            ssh: None,
            github: None,
            name,
            unknown: Unknown::default(),
        }
    }
}

/// The keys of the device the state knows, `known`, as `stored` holds them; or, if the state knows
/// none, the keys `stored` holds, if any.
///
/// # Errors
/// If the keys of a known device are missing or unreadable, or are another device's.
pub fn known_keys(
    stored: &secrets::Map,
    known: Option<&state::Device>,
) -> Result<Option<Keys>, Error> {
    let keys = Keys::load(stored)?;
    match (known, &keys) {
        (Some(_), None) => Err(Error::MissingKeys),
        (Some(device), Some(keys)) if keys.id() != device.id => Err(Error::OtherKeys(device.id)),
        _ => Ok(keys),
    }
}

/// The `name` of a new device: `given` if the user gave one, otherwise one made of `host`, the host
/// name.
///
/// # Errors
/// If `given` is not 1 to 64 bytes.
pub fn name(given: Option<&str>, host: &str) -> Result<String, Error> {
    match given {
        Some(name) if (1..=CERT_NAME_MAX).contains(&name.len()) => Ok(name.to_owned()),
        Some(name) => Err(Error::Name(name.to_owned())),
        None => Ok(host_name(host)),
    }
}

/// A valid `name` made of the host name `host`: cut to 64 bytes at a character's boundary, and
/// [`UNNAMED`] if empty.
fn host_name(host: &str) -> String {
    let host = host.trim();
    match &host[..host.floor_char_boundary(CERT_NAME_MAX)] {
        "" => UNNAMED.to_owned(),
        name => name.to_owned(),
    }
}

/// `DeviceRegistration` in the API spec.
#[derive(Serialize)]
struct Registration<'a> {
    device: String,
    kind: &'a str,
    signing: String,
    hpke: String,
    name: &'a str,
}

/// `MasterKeyRequest` in the API spec.
#[derive(Serialize)]
struct MasterKeyRequest {
    master: String,
    cert: String,
    record: String,
    backup: String,
}

/// `MasterKey` in the API spec.
#[derive(Deserialize)]
pub struct MasterKey {
    pub master: Option<String>,
    pub record: Option<String>,
    pub backup: Option<String>,
}

/// `GET /account/master`, as the session `token`.
///
/// # Errors
/// If the server does not give the answer.
pub fn master_key(connection: &mut Connection, token: &str) -> Result<MasterKey, sign_in::Error> {
    let body = connection.get_as("/account/master", token)?;
    serde_json::from_slice(&body)
        .map_err(|_| sign_in::Error::Unexpected("a master key answer without its key".to_owned()))
}

/// `device`, the machine's own device in `state`, with its keys `keys`, signed in to `account`
/// with the session `token`: it registers if it has not, then links as the account's first device
/// or waits for linking. A recovery key it creates goes to `stored`, the secrets, kept in
/// `secrets`.
pub struct Setup<'a> {
    pub connection: &'a mut Connection,
    pub state: &'a State,
    pub secrets: &'a Secrets,
    pub stored: &'a mut secrets::Map,
    pub token: &'a str,
    pub account: AccountId,
    pub device: state::Device,
    pub keys: &'a Keys,
    /// Where its person answers, if the run is the person's or has its page: then, waiting for
    /// linking, it shows link codes there.
    pub person: Option<Person<'a>>,
    /// With the recovery key, the devices its person names: then, waiting for linking, it links
    /// with the account's recovery key instead.
    pub recovery: Option<&'a recovery_link::Named>,
}

/// What setting up the device gives.
type Done = Result<(), sign_in::Error>;

impl Setup<'_> {
    pub fn run(mut self) -> Done {
        if !self.device.registered {
            self.register()?;
            self.state.set_registered()?;
        }
        match self.device.link.clone() {
            Link::Certified { .. } => {
                println!("This device holds its certificate, signed by the account's master key.");
                self.say_if_no_recovery_key();
                Ok(())
            }
            Link::Waiting | Link::Recovering { .. } if self.recovery.is_some() => self.recover(),
            Link::Waiting => {
                let offered = self.say_if_no_recovery_key();
                self.wait(offered)
            }
            Link::Recovering { .. } => {
                println!(
                    "This device is linking with the account's recovery key: to finish, run skald \
                     sign-in --with-recovery-key at a terminal."
                );
                self.wait(false)
            }
            Link::Granted { master, cert, link } => {
                link_code::store(self.connection, self.state, self.token, master, cert, link)?;
                Ok(())
            }
            Link::Sent {
                master,
                cert,
                record,
                backup,
            } => self.give_master(master, cert, &record, &backup),
            Link::New => self.link(),
        }
    }

    /// Waits for linking: by link codes it shows its person ([`link_code::show`]), first saying, if
    /// `offered`, that the account's recovery key links it too.
    fn wait(self, offered: bool) -> Done {
        let Some(person) = self.person else {
            waits();
            return Ok(());
        };
        if offered {
            println!(
                "To link this device with the account's recovery key instead, run skald sign-in \
                 --with-recovery-key at a terminal."
            );
        }
        link_code::show(
            &mut link_code::NewDevice {
                connection: self.connection,
                state: self.state,
                token: self.token,
                account: self.account,
                device: &self.device,
                keys: self.keys,
            },
            person,
        )
    }

    /// `POST /devices`: the same registration again, after a lost answer, is taken too.
    fn register(&mut self) -> Done {
        let registration = Registration {
            device: self.keys.id().to_string(),
            kind: "client",
            signing: URL_SAFE_NO_PAD.encode(self.keys.signing.public_key()),
            hpke: URL_SAFE_NO_PAD.encode(self.keys.hpke.public_key()),
            name: &self.device.name,
        };
        let body = serde_json::to_vec(&registration).expect("a registration is JSON");
        match self
            .connection
            .send_json_as(Method::Post, "/devices", self.token, &body)
        {
            Ok(_) => Ok(()),
            Err(connection::Error::Refused { status: 409, .. }) => Err(Error::DeviceTaken.into()),
            Err(err) => Err(err.into()),
        }
    }

    /// `GET /account/master`, then [`no_recovery_key`] on its record; an answer it does not get
    /// says nothing. Gives whether the account has a recovery key, an account record.
    fn say_if_no_recovery_key(&mut self) -> bool {
        match master_key(self.connection, self.token) {
            Ok(master) if master.master.is_some() => {
                no_recovery_key(master.record.as_deref());
                master.record.is_some()
            }
            _ => false,
        }
    }

    /// Links the device with the account's recovery key ([`recovery_link::link`]).
    fn recover(self) -> Done {
        let named = self.recovery.cloned().unwrap_or_default();
        recovery_link::link(
            &mut recovery_link::NewDevice {
                connection: self.connection,
                state: self.state,
                secrets: self.secrets,
                token: self.token,
                account: self.account,
                device: &self.device,
                keys: self.keys,
            },
            &named,
        )
    }

    /// Asks the server whether the account has a master key, and acts on it as "Linking a device"
    /// says: without one, the device is the account's first, unless it is to link with the
    /// recovery key; with one, it waits for linking.
    fn link(self) -> Done {
        let master = master_key(self.connection, self.token)?;
        match master.master {
            None if self.recovery.is_some() => Err(Error::NoMasterKey.into()),
            None => {
                let created = self.create_master()?;
                // The recovery key is kept before anything that it backs is.
                recovery::keep(self.secrets, self.stored, &created.recovery)?;
                self.state
                    .set_recovery_waits(Some(self.connection.server()))?;
                self.state.set_link(&Link::Sent {
                    master: created.master,
                    cert: created.cert.clone(),
                    record: created.record.clone(),
                    backup: created.backup.clone(),
                })?;
                self.give_master(
                    created.master,
                    created.cert,
                    &created.record,
                    &created.backup,
                )
            }
            Some(_) => {
                self.state.set_link(&Link::Waiting)?;
                if self.recovery.is_some() {
                    return self.recover();
                }
                no_recovery_key(master.record.as_deref());
                self.wait(master.record.is_some())
            }
        }
    }

    /// Creates the account's master key, issues the device's certificate with it, issued now, and
    /// checks it, as [`own_certificate`] does. The master private key is gone once it returns.
    fn create_master(&self) -> Result<Created, Error> {
        let body = self
            .keys
            .master_cert_body(self.account, self.device.name.clone());
        own_certificate(&body, self.keys)
    }

    /// `PUT /account/master`: gives the master public key, the certificate, the account record and
    /// the master backup, as often as it takes for the server to take them; drops them and the
    /// recovery key if the account took another device's master key.
    fn give_master(self, master: [u8; 32], cert: Vec<u8>, record: &[u8], backup: &[u8]) -> Done {
        let request = MasterKeyRequest {
            master: URL_SAFE_NO_PAD.encode(master),
            cert: URL_SAFE_NO_PAD.encode(&cert),
            record: URL_SAFE_NO_PAD.encode(record),
            backup: URL_SAFE_NO_PAD.encode(backup),
        };
        let body = serde_json::to_vec(&request).expect("a master key request is JSON");
        match self
            .connection
            .send_json_as(Method::Put, "/account/master", self.token, &body)
        {
            Ok(_) => {
                self.state.set_link(&Link::Certified { master, cert })?;
                println!(
                    "Created the account's master key and issued this device's certificate with \
                     it: the device holds its certificate. The server keeps the master key sealed \
                     to the account's recovery key."
                );
                Ok(())
            }
            Err(connection::Error::Refused { status: 409, .. }) => {
                recovery::forget(self.secrets, self.stored)?;
                self.state.set_recovery_waits(None)?;
                self.state.set_link(&Link::Waiting)?;
                println!(
                    "Another device gave the account's master key first: this device dropped its \
                     own."
                );
                self.wait(false)
            }
            Err(err) => Err(err.into()),
        }
    }
}

/// What the account's first device creates: the master public key, the certificate in CBOR, the
/// account record and the master backup in CBOR, and the recovery key.
#[derive(Debug)]
struct Created {
    master: [u8; 32],
    cert: Vec<u8>,
    record: Vec<u8>,
    backup: Vec<u8>,
    recovery: RecoveryKey,
}

/// A new master key, and the certificate of `body` that it issues and `keys` prove, checked against
/// it as "Device checks" says; a new recovery key, the account record of revision 1 that the master
/// key signs, and the master backup that seals the master key to the recovery key, as "Account"
/// says. The master private key is gone once it returns.
fn own_certificate(body: &CertBody, keys: &Keys) -> Result<Created, Error> {
    let master = SigningKey::generate();
    let public = master.public_key();
    let cert = skald_crypto::issue(body, &master, &keys.signing)
        .map_err(|err| Error::Cert(err.to_string()))?;
    let recovery = RecoveryKey::generate();
    let account = AccountBody {
        account: body.account,
        master: public,
        recovery: HpkeKey::from_recovery_key(&recovery.to_bytes()).public_key(),
        revision: 1,
        unknown: Unknown::default(),
    };
    let record = skald_crypto::sign_account(&account, &master)
        .expect("an account record of the spec's form");
    let backup = skald_crypto::seal_master_backup(&master, &recovery, body.account);
    drop(master);
    skald_crypto::check_device(&public, std::slice::from_ref(&cert), body.device)
        .map_err(|err| Error::Cert(err.to_string()))?;
    Ok(Created {
        master: public,
        cert: cert.to_cbor(),
        record: record.to_cbor(),
        backup: backup.to_cbor(),
        recovery,
    })
}

/// Says that the account has no recovery key when its master key has no account `record`, as on
/// an account made before recovery keys.
fn no_recovery_key(record: Option<&str>) {
    if record.is_none() {
        println!(
            "This account has no recovery key: it was made before recovery keys, and none can be \
             made for it."
        );
    }
}

/// Says that the device waits for linking.
fn waits() {
    println!(
        "This device is not linked yet: it waits for linking by a linked device of the account."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_name_gives_a_valid_name() {
        assert_eq!(name(None, "ada-laptop").unwrap(), "ada-laptop");
        assert_eq!(name(None, " ada \n").unwrap(), "ada");
        assert_eq!(name(None, "").unwrap(), UNNAMED);
        assert_eq!(name(None, "  ").unwrap(), UNNAMED);
        let long = "a".repeat(70);
        assert_eq!(name(None, &long).unwrap(), "a".repeat(CERT_NAME_MAX));
        // 1 + 2 * 50 bytes: cut before the character that crosses 64 bytes.
        let cut = name(None, &format!("a{}", "ü".repeat(50))).unwrap();
        assert_eq!(cut, format!("a{}", "ü".repeat(31)));
        assert_eq!(cut.len(), 63);
    }

    #[test]
    fn given_name_is_used_as_it_is_if_valid() {
        assert_eq!(name(Some(" Ada's "), "host").unwrap(), " Ada's ");
        assert_eq!(name(Some(&"b".repeat(64)), "host").unwrap(), "b".repeat(64));
        for invalid in [String::new(), "b".repeat(65)] {
            let error = name(Some(&invalid), "host").expect_err("invalid");
            assert!(matches!(error, Error::Name(_)), "{error:?}");
        }
    }

    #[test]
    fn keys_are_kept_apart_and_read_back() {
        let keys = Keys::generate();
        let mut stored = secrets::Map::from([("session_token".to_owned(), "t".to_owned())]);
        keys.store(&mut stored);
        assert_eq!(stored.len(), 3);
        assert_eq!(stored[SIGNING_KEY].len(), 43);
        assert_eq!(stored[HPKE_KEY].len(), 43);
        let read = Keys::load(&stored).unwrap().expect("keys");
        assert_eq!(read.id(), keys.id());
        assert_eq!(read.hpke.public_key(), keys.hpke.public_key());
        assert_eq!(
            Keys::load(&secrets::Map::new()).unwrap().map(|k| k.id()),
            None
        );
    }

    #[test]
    fn missing_unreadable_or_other_keys_are_errors() {
        let keys = Keys::generate();
        let mut stored = secrets::Map::new();
        keys.store(&mut stored);
        let known = state::Device {
            id: keys.id(),
            name: "ada".to_owned(),
            account: None,
            registered: true,
            link: Link::New,
        };
        assert_eq!(
            known_keys(&stored, Some(&known)).unwrap().map(|k| k.id()),
            Some(keys.id())
        );
        assert!(matches!(
            known_keys(&secrets::Map::new(), Some(&known)),
            Err(Error::MissingKeys)
        ));
        for name in [SIGNING_KEY, HPKE_KEY] {
            let mut one = stored.clone();
            one.remove(name);
            assert!(matches!(known_keys(&one, None), Err(Error::MissingKeys)));
            for broken in ["not a key", "AAAA"] {
                let mut bad = stored.clone();
                bad.insert(name.to_owned(), broken.to_owned());
                assert!(matches!(
                    known_keys(&bad, Some(&known)),
                    Err(Error::UnreadableKeys)
                ));
            }
        }
        let mut other = secrets::Map::new();
        Keys::generate().store(&mut other);
        assert!(matches!(
            known_keys(&other, Some(&known)),
            Err(Error::OtherKeys(id)) if id == keys.id()
        ));
        assert!(known_keys(&secrets::Map::new(), None).unwrap().is_none());
    }

    /// The state key is made once, beside the device keys, and read back after; it is never
    /// replaced while an MLS state is sealed under it.
    #[test]
    fn state_key_is_made_once_and_never_replaced() {
        let mut stored = secrets::Map::new();
        Keys::generate().store(&mut stored);
        let (made, new) = state_key(&mut stored, false).unwrap();
        assert!(new, "a new key");
        assert_eq!(stored.len(), 3);
        assert_eq!(stored[STATE_KEY].len(), 43);
        let (read, new) = state_key(&mut stored, true).unwrap();
        assert!(!new, "the stored key");
        assert_eq!(read.to_bytes(), made.to_bytes());

        let mut lost = secrets::Map::new();
        assert!(matches!(
            state_key(&mut lost, true),
            Err(Error::MissingStateKey)
        ));
        assert!(lost.is_empty(), "no key made");
        let mut broken = secrets::Map::from([(STATE_KEY.to_owned(), "AAAA".to_owned())]);
        assert!(matches!(
            state_key(&mut broken, false),
            Err(Error::UnreadableKeys)
        ));
        assert!(Error::MissingStateKey.to_string().contains("MLS state"));
    }

    #[test]
    fn own_certificate_is_issued_by_a_new_master_key_and_passes_its_checks() {
        let keys = Keys::generate();
        let body = CertBody {
            account: AccountId::generate(),
            device: keys.id(),
            kind: DeviceKind::Client,
            signing: keys.signing.public_key(),
            hpke: keys.hpke.public_key(),
            issuer: None,
            issued: 1_800_000_000,
            ssh: None,
            github: None,
            name: "ada".to_owned(),
            unknown: Unknown::default(),
        };
        let created = own_certificate(&body, &keys).expect("issued");
        let master = created.master;
        let cert = skald_proto::Cert::from_cbor(&created.cert).expect("a certificate");
        assert_eq!(*cert.body(), body);
        assert_eq!(
            skald_crypto::check_device(&master, std::slice::from_ref(&cert), keys.id()),
            Ok(&cert)
        );
        // The record names the account, the master key and the recovery key, and the backup opens
        // with that recovery key to the master key.
        let record = skald_proto::AccountRecord::from_cbor(&created.record).expect("a record");
        assert_eq!(skald_crypto::check_account(&record, &master), Ok(()));
        let recovery = HpkeKey::from_recovery_key(&created.recovery.to_bytes()).public_key();
        assert_eq!(
            (
                record.body().account,
                record.body().recovery,
                record.body().revision
            ),
            (body.account, recovery, 1)
        );
        let backup = skald_proto::MasterBackup::from_cbor(&created.backup).expect("a backup");
        let opened =
            skald_crypto::open_master_backup(&backup, &created.recovery, body.account, &master)
                .expect("the backup opens");
        assert_eq!(opened.public_key(), master);
        let other = own_certificate(&body, &keys).expect("issued");
        assert_ne!(other.master, master, "a new master key each time");
        assert_ne!(
            *other.recovery.to_bytes(),
            *created.recovery.to_bytes(),
            "a new recovery key each time"
        );
        // A body the device's keys do not prove is not issued.
        let other_device = CertBody {
            device: Keys::generate().id(),
            ..body
        };
        let error = own_certificate(&other_device, &keys).expect_err("not this device's");
        assert!(matches!(error, Error::Cert(_)), "{error:?}");
    }

    #[test]
    fn key_errors_say_key() {
        for error in [
            Error::MissingKeys,
            Error::UnreadableKeys,
            Error::OtherKeys(Keys::generate().id()),
        ] {
            assert!(error.to_string().contains("private keys"), "{error}");
        }
        let no_master = Error::NoMasterKey.to_string();
        assert!(no_master.contains("no master key"), "{no_master}");
        assert!(
            no_master.contains("without --with-recovery-key"),
            "{no_master}"
        );
    }
}
