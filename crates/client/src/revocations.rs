//! Revocations on this device (`docs/protocol/membership.md`, "Revocation", "Device checks" and
//! "Group changes").
//!
//! - `skald revoke-device`: on a device with a valid certificate, it revokes a device of its
//!   account that has one, itself included, by a revocation it signs without `keep` or `also`. The
//!   revocation waits in the local state until the server has taken it: each later run that
//!   connects sends it again, with the same bytes.
//! - Every device check applies the revocations the server gives with the certificates and those
//!   the device holds. The device keeps each one that counts, so a server that later hides it
//!   restores no device: only voiding undoes a revocation.
//! - A Commit with the Remove of a revoked device carries the revocations by which its sender holds
//!   it revoked; a device that processes one keeps those that count.

use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use skald_crypto::{DeviceCheckError, check_device, check_valid_device, counts, sign_revocation};
use skald_proto::{
    AccountId, Cbor as _, Cert, DeviceId, NonEmpty, Revocation, RevocationBody, Revocations,
    Unknown, VersionRange,
};

use crate::connection::{self, Connection, Method};
use crate::devices::{self, signed_in_with_secrets, text, to_session};
use crate::follow::SpaceDevices;
use crate::secrets;
use crate::spaces::now;
use crate::state::{self, Link, State};

/// `skald revoke-device <device>`: revokes `device`, a device of the account of the machine whose
/// state is in `dir`, its secrets in `storage`.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    device: &str,
) -> Result<(), String> {
    let target: DeviceId = device
        .parse()
        .map_err(|_| format!("\"{}\" is not a device ID", connection::printable(device)))?;
    let (state, server, token, stored, _) = signed_in_with_secrets(dir, storage)?;
    let device = state.device().map_err(text)?;
    let Some(state::Device {
        id: this,
        link: Link::Certified { master, .. },
        ..
    }) = &device
    else {
        return Err(NO_CERTIFICATE.to_owned());
    };
    let (this, master) = (*this, *master);
    let mut connection = to_session(&server, speaks, timeout)?;
    send_pending(&mut connection, &token, &state);
    let devices = devices::own(&mut connection, &token, &state, &master)?;
    let own = check_valid_device(&master, &devices.certs, &devices.revocations, this)
        .map_err(|_| NO_CERTIFICATE.to_owned())?
        .body()
        .account;
    // The account's devices only, as the server lists them.
    let cert = check_device(&master, &devices.certs, target).map_err(|_| {
        format!(
            "the server lists no device {target} of this account with a certificate chained to \
             the account's master key this machine pinned; nothing is revoked"
        )
    })?;
    let keys = crate::device::known_keys(&stored, device.as_ref())
        .map_err(text)?
        .ok_or_else(|| crate::device::Error::MissingKeys.to_string())?;
    let body = RevocationBody {
        account: own,
        device: target,
        keep: None,
        signer: Some(this),
        timestamp: now(),
        also: None,
        unknown: Unknown::default(),
    };
    let revocation = sign_revocation(&body, keys.signing())
        .map_err(|err| format!("the revocation does not encode: {err}"))?
        .to_cbor();
    state
        .add_revocations(std::slice::from_ref(&revocation), true)
        .map_err(text)?;
    let name = connection::printable(&cert.body().name);
    store(&mut connection, &token, &state, &revocation)?;
    println!("Revoked the device {target} ({name}).");
    Ok(())
}

/// What `skald revoke-device` says on a device without a valid certificate.
const NO_CERTIFICATE: &str = "this device has no valid certificate, so it revokes no device";

/// `POST /account/revocations`: has the server store `revocation`, in CBOR, which the device
/// signed and `state` holds as pending, as the session `token`. Taken, it no longer waits;
/// otherwise it waits for a later run.
fn store(
    connection: &mut Connection,
    token: &str,
    state: &State,
    revocation: &[u8],
) -> Result<(), String> {
    let request =
        serde_json::json!({ "revocation": URL_SAFE_NO_PAD.encode(revocation) }).to_string();
    connection
        .send_json_as(
            Method::Post,
            "/account/revocations",
            token,
            request.as_bytes(),
        )
        .map_err(|err| {
            format!(
                "{err}; the revocation waits for the server to take it: the next skald command \
                 that connects sends it again"
            )
        })?;
    state.revocation_taken(revocation).map_err(text)
}

/// Sends the revocations the device signed that wait for the server, as the session `token`;
/// warns of each the server has not taken.
pub fn send_pending(connection: &mut Connection, token: &str, state: &State) {
    let pending = match state.revocations() {
        Ok(held) => held,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    for (revocation, _) in pending.into_iter().filter(|(_, pending)| *pending) {
        if let Err(err) = store(connection, token, state, &revocation) {
            eprintln!("skald: warning: {err}");
            return;
        }
    }
}

/// `given`, revocations the server gives, with those `state` holds that it does not give.
pub fn with_held(
    state: &State,
    given: impl IntoIterator<Item = Revocation>,
) -> Result<Vec<Revocation>, String> {
    let mut all: Vec<Revocation> = given.into_iter().collect();
    for (bytes, _) in state.revocations().map_err(text)? {
        let Ok(held) = Revocation::from_cbor(&bytes) else {
            continue;
        };
        if !all.iter().any(|given| given.to_cbor() == bytes) {
            all.push(held);
        }
    }
    Ok(all)
}

/// Keeps in `state` each revocation of `devices` it does not hold that counts against the master
/// key `master` gives its account and the certificates of `devices`.
pub fn keep(
    state: &State,
    devices: &SpaceDevices,
    master: impl Fn(AccountId) -> Option<[u8; 32]>,
) -> Result<(), String> {
    let held: Vec<Vec<u8>> = (state.revocations().map_err(text)?.into_iter())
        .map(|(bytes, _)| bytes)
        .collect();
    let new: Vec<Vec<u8>> = (devices.revocations.iter())
        .filter(|revocation| {
            master(revocation.body().account).is_some_and(|master| {
                counts(&master, &devices.certs, &devices.revocations, revocation)
            })
        })
        .map(Revocation::to_cbor)
        .filter(|bytes| !held.contains(bytes))
        .collect();
    if new.is_empty() {
        return Ok(());
    }
    state.add_revocations(&new, false).map_err(text)
}

/// The revocations of `devices` that count against the master key `master` gives `device`'s
/// account and by which `device` is revoked: those that name it, or the issuer in its chain that
/// check 4 fails at. None if its certificate fails checks 1 and 2, or it is not revoked.
pub fn revoking(
    devices: &SpaceDevices,
    master: impl Fn(AccountId) -> Option<[u8; 32]>,
    device: DeviceId,
) -> Vec<Revocation> {
    let SpaceDevices {
        certs, revocations, ..
    } = devices;
    let Some(cert) = certs.iter().find(|cert| cert.body().device == device) else {
        return Vec::new();
    };
    let account = cert.body().account;
    let Some(master) = master(account) else {
        return Vec::new();
    };
    let Err(DeviceCheckError::Revoked { device: named }) =
        check_valid_device(&master, certs, revocations, device)
    else {
        return Vec::new();
    };
    let mut by: Vec<Revocation> = Vec::new();
    for revocation in revocations {
        let body = revocation.body();
        let names = body.device == named
            || body
                .also
                .iter()
                .flat_map(|also| also.iter())
                .any(|id| *id == named);
        if body.account == account
            && names
            && counts(&master, certs, revocations, revocation)
            && !by.contains(revocation)
        {
            by.push(revocation.clone());
        }
    }
    by
}

/// What a Commit's `authenticated_data` carries for the Remove of `devices`: the encoded
/// `[+ revocation]` of the revocations by which each is revoked ([`revoking`]); empty if none is.
pub fn carried(
    known: &SpaceDevices,
    master: impl Fn(AccountId) -> Option<[u8; 32]>,
    devices: &[DeviceId],
) -> Vec<u8> {
    let mut carried: Vec<Revocation> = Vec::new();
    for &device in devices {
        for revocation in revoking(known, &master, device) {
            if !carried.contains(&revocation) {
                carried.push(revocation);
            }
        }
    }
    NonEmpty::from_vec(carried).map_or_else(Vec::new, |carried| Revocations(carried).to_cbor())
}

/// The revocations a Commit's `authenticated_data` carries that `known` lacks and that count
/// against the master key `master` gives their account, the certificates of `known` and the
/// revocations of both; none if it carries no `[+ revocation]`.
pub fn learned(
    authenticated_data: &[u8],
    known: &SpaceDevices,
    master: impl Fn(AccountId) -> Option<[u8; 32]>,
) -> Vec<Revocation> {
    let Ok(Revocations(carried)) = Revocations::from_cbor(authenticated_data) else {
        return Vec::new();
    };
    let all: Vec<Revocation> = (known.revocations.iter())
        .chain(carried.iter())
        .cloned()
        .collect();
    let mut learned: Vec<Revocation> = Vec::new();
    for revocation in carried.iter() {
        let counting = master(revocation.body().account)
            .is_some_and(|master| counts(&master, &known.certs, &all, revocation));
        if counting
            && !all[..known.revocations.len()].contains(revocation)
            && !learned.contains(revocation)
        {
            learned.push(revocation.clone());
        }
    }
    learned
}

/// Fails, saying so, if this device, of `cert` on the account of the pinned `master` key, is revoked
/// by the revocations the server gives with its account's devices, as the session `token`, or
/// those `state` holds; keeps those the server gives that count.
pub fn refuse_if_revoked(
    connection: &mut Connection,
    token: &str,
    state: &State,
    (master, cert): (&[u8; 32], &Cert),
) -> Result<(), String> {
    let this = cert.body().device;
    let devices = devices::own(connection, token, state, master)?;
    match check_valid_device(master, &devices.certs, &devices.revocations, this) {
        Err(DeviceCheckError::Revoked { .. }) => Err(format!(
            "this device {this} is revoked: it sends and receives nothing; sign in on a device \
             that is not"
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;
    use skald_crypto::{SigningKey, issue};
    use skald_proto::{CertBody, DeviceKind};

    /// An account with its master key and three `client` devices: the first and the third
    /// certified by the master key, the second by the first. The third revokes the first, which
    /// counts; a revocation of the third by the second, signed with the first's key, does not.
    struct Account {
        id: AccountId,
        master: SigningKey,
        keys: Vec<SigningKey>,
        certs: Vec<Cert>,
        counting: Revocation,
        forged: Revocation,
    }

    impl Account {
        fn new() -> Self {
            let (id, master) = (AccountId::generate(), SigningKey::generate());
            let keys: Vec<SigningKey> = (0..3).map(|_| SigningKey::generate()).collect();
            let device = |key: &SigningKey| DeviceId::from_signing_key(&key.public_key());
            let cert = |key: &SigningKey, issuer: Option<usize>| {
                let body = CertBody {
                    account: id,
                    device: device(key),
                    kind: DeviceKind::Client,
                    signing: key.public_key(),
                    hpke: [3; 32],
                    issuer: issuer.map(|at| device(&keys[at])),
                    issued: 1_800_000_000,
                    ssh: None,
                    name: "laptop".to_owned(),
                    unknown: Unknown::default(),
                };
                let signer = issuer.map_or(&master, |at| &keys[at]);
                issue(&body, signer, key).expect("a certificate")
            };
            let certs = vec![
                cert(&keys[0], None),
                cert(&keys[1], Some(0)),
                cert(&keys[2], None),
            ];
            let revocation = |revoked: usize, signer: usize, key: &SigningKey| {
                let body = RevocationBody {
                    account: id,
                    device: device(&keys[revoked]),
                    keep: None,
                    signer: Some(device(&keys[signer])),
                    timestamp: 1_800_000_000,
                    also: None,
                    unknown: Unknown::default(),
                };
                sign_revocation(&body, key).expect("a revocation")
            };
            let counting = revocation(0, 2, &keys[2]);
            let forged = revocation(2, 1, &keys[0]);
            Account {
                id,
                master,
                keys,
                certs,
                counting,
                forged,
            }
        }

        fn device(&self, at: usize) -> DeviceId {
            DeviceId::from_signing_key(&self.keys[at].public_key())
        }

        /// Its devices, with `revocations`.
        fn devices(&self, revocations: &[&Revocation]) -> SpaceDevices {
            SpaceDevices {
                ids: (0..3).map(|at| self.device(at)).collect(),
                certs: self.certs.clone(),
                revocations: revocations.iter().map(|&one| one.clone()).collect(),
            }
        }

        /// Its master key for its own account only.
        fn master(&self) -> impl Fn(AccountId) -> Option<[u8; 32]> + Copy {
            let (id, master) = (self.id, self.master.public_key());
            move |account| (account == id).then_some(master)
        }
    }

    #[test]
    fn device_is_revoked_by_the_counting_revocations_of_it_or_of_its_revoked_issuer() {
        let ada = Account::new();
        let devices = ada.devices(&[&ada.forged, &ada.counting, &ada.counting]);
        let counting = vec![ada.counting.clone()];

        assert_eq!(revoking(&devices, ada.master(), ada.device(0)), counting);
        assert_eq!(revoking(&devices, ada.master(), ada.device(1)), counting);
        assert_eq!(revoking(&devices, ada.master(), ada.device(2)), vec![]);
        let unknown = DeviceId::from_signing_key(&[9; 32]);
        assert_eq!(revoking(&devices, ada.master(), unknown), vec![]);
        assert_eq!(revoking(&devices, |_| None, ada.device(0)), vec![]);
        let other = SigningKey::generate().public_key();
        assert_eq!(revoking(&devices, |_| Some(other), ada.device(0)), vec![]);
    }

    #[test]
    fn commit_carries_each_revocation_of_the_removed_devices_once_or_nothing() {
        let ada = Account::new();
        let devices = ada.devices(&[&ada.forged, &ada.counting]);
        let one = Revocations(NonEmpty::new(ada.counting.clone())).to_cbor();

        let removed = [ada.device(0), ada.device(1), ada.device(2)];
        assert_eq!(carried(&devices, ada.master(), &removed), one);
        assert_eq!(
            carried(&devices, ada.master(), &removed[2..]),
            Vec::<u8>::new()
        );
        assert_eq!(carried(&devices, ada.master(), &[]), Vec::<u8>::new());
    }

    #[test]
    fn device_learns_from_a_commit_only_the_new_revocations_that_count() {
        let ada = Account::new();
        let carried = |revocations: Vec<Revocation>| {
            Revocations(NonEmpty::from_vec(revocations).expect("revocations")).to_cbor()
        };
        let (counting, forged) = (ada.counting.clone(), ada.forged.clone());
        let both = carried(vec![forged.clone(), counting.clone(), counting.clone()]);

        assert_eq!(
            learned(&both, &ada.devices(&[]), ada.master()),
            vec![counting.clone()]
        );
        assert_eq!(
            learned(&both, &ada.devices(&[&counting]), ada.master()),
            vec![]
        );
        assert_eq!(learned(&both, &ada.devices(&[]), |_| None), vec![]);
        for nothing in [&b""[..], b"\x80", &counting.to_cbor()] {
            assert_eq!(learned(nothing, &ada.devices(&[]), ada.master()), vec![]);
        }
    }

    #[test]
    fn device_keeps_the_revocations_given_that_count_and_checks_with_those_it_holds() {
        let dir = TestDir::new("revocations-kept");
        let state = State::open(&dir).expect("a state");
        let ada = Account::new();
        let (counting, forged) = (ada.counting.to_cbor(), ada.forged.to_cbor());
        // One this device signed, which waits for the server.
        let body = RevocationBody {
            account: ada.id,
            device: ada.device(1),
            keep: None,
            signer: Some(ada.device(2)),
            timestamp: 1_800_000_000,
            also: None,
            unknown: Unknown::default(),
        };
        let signed = sign_revocation(&body, &ada.keys[2]).expect("a revocation");
        state
            .add_revocations(std::slice::from_ref(&signed.to_cbor()), true)
            .unwrap();

        keep(
            &state,
            &ada.devices(&[&ada.forged, &ada.counting, &signed]),
            ada.master(),
        )
        .unwrap();
        keep(&state, &ada.devices(&[&ada.counting]), |_| None).unwrap();

        assert_eq!(
            state.revocations().unwrap(),
            vec![(signed.to_cbor(), true), (counting.clone(), false)]
        );
        assert_eq!(
            with_held(&state, [ada.forged.clone(), ada.counting.clone()]).unwrap(),
            vec![ada.forged.clone(), ada.counting.clone(), signed.clone()]
        );
        assert!(
            !state
                .revocations()
                .unwrap()
                .iter()
                .any(|(bytes, _)| *bytes == forged)
        );
    }
}
