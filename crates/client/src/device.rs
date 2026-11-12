//! The machine's own device (`docs/protocol/membership.md`, "Linking a device"): its keys, kept with
//! the secrets, its `name`, its registration with the server and, on the account's first device,
//! the account master key and the certificate it issues.

use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use skald_crypto::{HpkeKey, SigningKey};
use skald_proto::{AccountId, CERT_NAME_MAX, Cbor as _, CertBody, DeviceId, DeviceKind, Unknown};
use zeroize::Zeroizing;

use crate::connection::{self, Connection, Method};
use crate::secrets;
use crate::sign_in;
use crate::state::{self, Link, State};

/// The secrets that hold the device's private keys: its Ed25519 signing key and its X25519 HPKE
/// key, each as its 32 bytes in base64url without padding.
const SIGNING_KEY: &str = "device_signing_key";
const HPKE_KEY: &str = "device_hpke_key";
/// The `name` of a device whose host name is empty.
const UNNAMED: &str = "skald device";

/// Why the device cannot be set up, beyond the server, the state or the secrets.
#[derive(Debug)]
pub enum Error {
    /// The secrets lack a private key of the device.
    MissingKeys,
    /// The secrets hold a private key of the device that is not one.
    UnreadableKeys,
    /// The secrets hold the keys of another device than the one the state knows.
    OtherKeys(DeviceId),
    /// A `name` the user gave is not 1 to 64 bytes.
    Name(String),
    /// The device is named already, and the user gave another `name`.
    Renamed(String),
    /// The server has the device's ID with other keys or `name`, or on another account.
    DeviceTaken,
    /// The user says it is the account's first device, and the account has a master key.
    NotFirst,
    /// The user says it is not the account's first device, and the account has no master key.
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
            Error::NotFirst => f.write_str(
                "the account has a master key already: someone else may have created it. This \
                 device does not link and makes no master key; if it is not the account's first \
                 device, run skald sign-in --first-device no",
            ),
            Error::NoMasterKey => f.write_str(
                "the server reports no master key for the account, so this device makes none \
                 and does not link; if it is the account's first device, run skald sign-in \
                 --first-device yes",
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
        let read = |name| -> Result<Option<Zeroizing<[u8; 32]>>, Error> {
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
        };
        match (read(SIGNING_KEY)?, read(HPKE_KEY)?) {
            (None, None) => Ok(None),
            (Some(signing), Some(hpke)) => Ok(Some(Keys {
                signing: SigningKey::from_bytes(&signing),
                hpke: HpkeKey::from_bytes(&hpke),
            })),
            _ => Err(Error::MissingKeys),
        }
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

/// Whether this is the account's first device, as the user answers on `input` the question
/// written to `output`; `None` once `input` ends without an answer.
pub fn ask_first(input: &mut impl BufRead, output: &mut impl Write) -> io::Result<Option<bool>> {
    loop {
        write!(
            output,
            "Is this the account's first device? Answer yes only if no other device of the \
             account has signed in; yes creates the account's master key [yes/no]: "
        )?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            writeln!(output)?;
            return Ok(None);
        }
        match line.trim().to_lowercase().as_str() {
            "yes" | "y" => return Ok(Some(true)),
            "no" | "n" => return Ok(Some(false)),
            _ => writeln!(output, "Please answer yes or no.")?,
        }
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
}

/// `MasterKey` in the API spec.
#[derive(Deserialize)]
struct MasterKey {
    master: Option<String>,
}

/// Whether this is the account's first device, as the user said beforehand, or ask the user.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum First {
    Yes,
    No,
    Ask,
}

/// `device`, the machine's own device in `state`, with its keys `keys`, signed in to `account`
/// with the session `token`: it registers if it has not, then links as the account's first device
/// or waits for linking.
pub struct Setup<'a> {
    pub connection: &'a mut Connection,
    pub state: &'a State,
    pub token: &'a str,
    pub account: AccountId,
    pub device: state::Device,
    pub keys: &'a Keys,
    pub first: First,
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
                Ok(())
            }
            Link::Waiting => {
                waits();
                Ok(())
            }
            Link::Sent { master, cert } => self.give_master(master, cert),
            Link::New => self.link(),
        }
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

    /// Asks whether this is the account's first device, then the server whether the account has a
    /// master key, and acts on both as "Linking a device" says.
    fn link(self) -> Done {
        let first = match self.first {
            First::Yes => true,
            First::No => false,
            First::Ask => {
                let answer = ask_first(&mut io::stdin().lock(), &mut io::stdout())
                    .map_err(|err| sign_in::Error::Io(format!("cannot ask the user: {err}")))?;
                let Some(first) = answer else {
                    println!(
                        "This device is not linked yet: it needs to know whether it is the \
                         account's first device. Run skald sign-in --first-device yes|no."
                    );
                    return Ok(());
                };
                first
            }
        };
        let body = self.connection.get_as("/account/master", self.token)?;
        let master: MasterKey = serde_json::from_slice(&body).map_err(|_| {
            sign_in::Error::Unexpected("a master key answer without its key".to_owned())
        })?;
        match (first, master.master) {
            (true, None) => {
                let (master, cert) = self.create_master()?;
                self.state.set_link(&Link::Sent {
                    master,
                    cert: cert.clone(),
                })?;
                self.give_master(master, cert)
            }
            (true, Some(_)) => Err(Error::NotFirst.into()),
            (false, None) => Err(Error::NoMasterKey.into()),
            (false, Some(_)) => {
                self.state.set_link(&Link::Waiting)?;
                waits();
                Ok(())
            }
        }
    }

    /// Creates the account's master key, issues the device's certificate with it, issued now, and
    /// checks it; gives the master public key and the certificate in CBOR. The master private key
    /// is gone once it returns.
    fn create_master(&self) -> Result<([u8; 32], Vec<u8>), Error> {
        let issued = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let body = CertBody {
            account: self.account,
            device: self.keys.id(),
            kind: DeviceKind::Client,
            signing: self.keys.signing.public_key(),
            hpke: self.keys.hpke.public_key(),
            issuer: None,
            issued,
            ssh: None,
            name: self.device.name.clone(),
            unknown: Unknown::default(),
        };
        own_certificate(&body, self.keys)
    }

    /// `PUT /account/master`: gives the master public key and the certificate, as often as it takes
    /// for the server to take them; drops them if the account took another device's master key.
    fn give_master(self, master: [u8; 32], cert: Vec<u8>) -> Done {
        let request = MasterKeyRequest {
            master: URL_SAFE_NO_PAD.encode(master),
            cert: URL_SAFE_NO_PAD.encode(&cert),
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
                     it: the device holds its certificate. The master private key is not kept."
                );
                Ok(())
            }
            Err(connection::Error::Refused { status: 409, .. }) => {
                self.state.set_link(&Link::Waiting)?;
                println!(
                    "Another device gave the account's master key first: this device dropped its \
                     own."
                );
                waits();
                Ok(())
            }
            Err(err) => Err(err.into()),
        }
    }
}

/// A new master key, and the certificate of `body` that it issues and `keys` prove, checked against
/// it as "Device checks" says: the master public key and the certificate in CBOR. The master
/// private key is gone once it returns.
fn own_certificate(body: &CertBody, keys: &Keys) -> Result<([u8; 32], Vec<u8>), Error> {
    let master = SigningKey::generate();
    let public = master.public_key();
    let cert = skald_crypto::issue(body, &master, &keys.signing)
        .map_err(|err| Error::Cert(err.to_string()))?;
    drop(master);
    skald_crypto::check_device(&public, std::slice::from_ref(&cert), body.device)
        .map_err(|err| Error::Cert(err.to_string()))?;
    Ok((public, cert.to_cbor()))
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

    fn asked(input: &str) -> (Option<bool>, String) {
        let mut output = Vec::new();
        let answer = ask_first(&mut input.as_bytes(), &mut output).expect("asked");
        (answer, String::from_utf8(output).expect("UTF-8"))
    }

    #[test]
    fn question_takes_yes_or_no_and_asks_again_otherwise() {
        for (input, answer) in [
            ("yes\n", Some(true)),
            ("Y\n", Some(true)),
            (" no \n", Some(false)),
            ("n", Some(false)),
            ("maybe\nyes\n", Some(true)),
            ("", None),
            ("maybe\n", None),
        ] {
            let (got, shown) = asked(input);
            assert_eq!(got, answer, "{input:?}");
            assert!(shown.contains("first device"), "{shown}");
        }
        assert!(asked("maybe\nno\n").1.contains("Please answer yes or no."));
    }

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
            name: "ada".to_owned(),
            unknown: Unknown::default(),
        };
        let (master, cert) = own_certificate(&body, &keys).expect("issued");
        let cert = skald_proto::Cert::from_cbor(&cert).expect("a certificate");
        assert_eq!(*cert.body(), body);
        assert_eq!(
            skald_crypto::check_device(&master, std::slice::from_ref(&cert), keys.id()),
            Ok(&cert)
        );
        let (other, _) = own_certificate(&body, &keys).expect("issued");
        assert_ne!(other, master, "a new master key each time");
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
        assert!(Error::NotFirst.to_string().contains("someone else"));
        assert!(Error::NoMasterKey.to_string().contains("no master key"));
    }
}
