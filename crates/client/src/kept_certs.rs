//! The certificate of each device this device trusted (`docs/protocol/membership.md`, "Device
//! certificate", kept certificates). Given that device with another certificate, the device warns
//! once of a key change until the server gives the kept one again. One of another body that passes
//! the proof check is kept too, and both join every list of the device's certificates, so neither
//! is valid ("two bodies") also once the server gives only one.

use skald_crypto::passes_proof_check;
use skald_proto::{Cbor as _, Cert, DeviceId};

use crate::devices::text;
use crate::key_changes;
use crate::state::State;

/// Keeps the certificates of `trusted`, each of a device that passed checks 1 and 2, of the devices
/// it keeps none of yet.
pub fn keep(state: &State, trusted: &[&Cert]) -> Result<(), String> {
    for cert in trusted {
        (state.keep_cert(cert.body().device, &cert.to_cbor(), true)).map_err(text)?;
    }
    Ok(())
}

/// Checks `given`, the certificates the server gives, against those `state` keeps: keeps each of a
/// kept device with another body that passes the proof check, and gives the warning of each kept
/// device given with another certificate in place of the first kept one, unless it warned of that
/// certificate last, and keeps it as a key change. Gives `given` with every kept certificate of each
/// device with two bodies, and the warnings.
pub fn settle(state: &State, mut given: Vec<Cert>) -> Result<(Vec<Cert>, Vec<String>), String> {
    let mut devices: Vec<DeviceId> = Vec::new();
    for cert in &given {
        if !devices.contains(&cert.body().device) {
            devices.push(cert.body().device);
        }
    }
    let mut warnings = Vec::new();
    let mut added = Vec::new();
    for device in devices {
        let kept = kept(state, device)?;
        let Some(first) = kept.first() else {
            continue;
        };
        let first_bytes = first.to_cbor();
        let mut swapped = Vec::new();
        let mut back = false;
        for cert in given.iter().filter(|cert| cert.body().device == device) {
            let bytes = cert.to_cbor();
            back |= bytes == first_bytes;
            if kept.iter().any(|kept| kept.to_cbor() == bytes) {
                continue;
            }
            if cert.body_bytes() != first.body_bytes() && passes_proof_check(cert) {
                state.keep_cert(device, &bytes, false).map_err(text)?;
            } else {
                swapped.push(bytes);
            }
        }
        // Beside the kept certificate, the others count for nothing.
        if back {
            swapped.clear();
        }
        if state.swapped_certs(device, &swapped).map_err(text)? {
            let warning = format!(
                "the server gives device {device} a certificate other than the one this device \
                 trusted, so it is not trusted until the server gives that one back: its key may \
                 have been swapped"
            );
            key_changes::keep(state, &warning)?;
            warnings.push(warning);
        }
        let kept = self::kept(state, device)?;
        if kept.len() > 1 {
            added.extend(kept);
        }
    }
    given.extend(added);
    Ok((given, warnings))
}

/// The certificates `state` keeps of `device` that decode.
fn kept(state: &State, device: DeviceId) -> Result<Vec<Cert>, String> {
    Ok((state.kept_certs(device).map_err(text)?.iter())
        .filter_map(|bytes| Cert::from_cbor(bytes).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use skald_crypto::{SigningKey, check_device, issue};
    use skald_proto::{AccountId, CertBody, DeviceKind};

    use super::*;
    use crate::test_dir::TestDir;

    /// A device certified by `master`, with its key.
    fn device(master: &SigningKey, account: AccountId, name: &str) -> (SigningKey, Cert) {
        let key = SigningKey::generate();
        let body = body(&key, account, name);
        let cert = issue(&body, master, &key).expect("a certificate");
        (key, cert)
    }

    fn body(key: &SigningKey, account: AccountId, name: &str) -> CertBody {
        let signing = key.public_key();
        CertBody {
            account,
            device: DeviceId::from_signing_key(&signing),
            kind: DeviceKind::Client,
            signing,
            hpke: [7; 32],
            issuer: None,
            issued: 1,
            ssh: None,
            github: None,
            name: name.to_owned(),
            unknown: Default::default(),
        }
    }

    /// A certificate given as the kept one passes as it is; another with a broken signature in its
    /// place warns once, until the kept one is given again, and beside it not at all.
    #[test]
    fn a_swapped_certificate_warns_once_until_the_kept_one_is_back() {
        let dir = TestDir::new("kept-certs-swap");
        let state = State::open(&dir).unwrap();
        let master = SigningKey::generate();
        let (_, cert) = device(&master, AccountId::generate(), "laptop");
        let mut signature = *cert.signature();
        signature[0] ^= 1;
        let broken =
            Cert::from_parts(cert.body_bytes().to_vec(), signature, *cert.proof()).unwrap();

        let (certs, warnings) = settle(&state, vec![broken.clone()]).unwrap();
        assert_eq!((certs.len(), warnings.len()), (1, 0), "nothing kept yet");
        keep(&state, &[&cert]).unwrap();
        let (certs, warnings) = settle(&state, vec![cert.clone()]).unwrap();
        assert_eq!((certs, warnings.len()), (vec![cert.clone()], 0));
        for warned in [1, 0] {
            let (certs, warnings) = settle(&state, vec![broken.clone()]).unwrap();
            assert_eq!(certs, vec![broken.clone()]);
            assert_eq!(warnings.len(), warned, "{warnings:?}");
            assert!(
                warnings
                    .iter()
                    .all(|w| w.contains(&cert.body().device.to_string()))
            );
        }
        assert!(settle(&state, vec![cert.clone()]).unwrap().1.is_empty());
        let beside = vec![broken.clone(), cert.clone()];
        assert!(settle(&state, beside).unwrap().1.is_empty());
        assert_eq!(settle(&state, vec![broken]).unwrap().1.len(), 1);
    }

    /// A second body that passes the proof check is kept, and with the first it makes the device
    /// invalid whichever the server gives.
    #[test]
    fn a_second_body_leaves_the_device_invalid() {
        let dir = TestDir::new("kept-certs-bodies");
        let state = State::open(&dir).unwrap();
        let master = SigningKey::generate();
        let (key, cert) = device(&master, AccountId::generate(), "laptop");
        let id = cert.body().device;
        let second = issue(&body(&key, cert.body().account, "stolen"), &master, &key).unwrap();
        keep(&state, &[&cert]).unwrap();
        let master = master.public_key();

        let (certs, warnings) = settle(&state, vec![second.clone()]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(check_device(&master, &certs, id).is_err());
        let (certs, _) = settle(&state, vec![cert.clone()]).unwrap();
        assert!(check_device(&master, &certs, id).is_err());
        keep(&state, &[&cert]).unwrap();
        assert_eq!(state.kept_certs(id).unwrap().len(), 2);
    }
}
