//! The device's KeyPackages on the server, as `docs/protocol/membership.md`, "KeyPackages", says:
//! with fewer than 10 unused that expire in more than 7 days, it uploads enough for 20; it
//! replaces its last-resort one once it was handed out or within 7 days of its expiry. Their
//! private keys join the device's MLS state, which is saved sealed before they are uploaded.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use skald_crypto::{MlsDevice, SealedState, StateKey};

use crate::connection::{Connection, Method};
use crate::device::{self, Keys};
use crate::devices::text;
use crate::secrets::Secrets;
use crate::state::State;

/// How many unused KeyPackages the device keeps.
const KEPT: u64 = 20;
/// Below how many unused ones it uploads more.
const LOW: u64 = 10;

/// `KeyPackageCount` in the API spec.
#[derive(Deserialize)]
struct Count {
    unused: u64,
    replace_last_resort: bool,
}

/// `KeyPackageUpload` in the API spec.
#[derive(Serialize)]
struct Upload {
    key_packages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_resort: Option<String>,
}

/// How many KeyPackages to make, and whether to make a last-resort one, for a device whose server
/// holds `count`.
fn wanted(count: &Count) -> (u64, bool) {
    let others = if count.unused < LOW {
        KEPT - count.unused
    } else {
        0
    };
    (others, count.replace_last_resort)
}

/// The key the MLS state `sealed` is sealed under, from the secrets in `secrets`; for a device
/// without an MLS state and without one, a new one, saved to them.
pub fn state_key(secrets: &Secrets, sealed: &SealedState) -> Result<StateKey, String> {
    let mut stored = secrets.load().map_err(text)?;
    let held = sealed.items().next().is_some();
    let (key, new) = device::state_key(&mut stored, held).map_err(text)?;
    if new {
        secrets.save(&stored).map_err(text)?;
    }
    Ok(key)
}

/// The device of `keys` keeps its KeyPackages on the server of `connection`, as the session
/// `token`: it asks how many the server holds and, if it needs more, makes them in its MLS state in
/// `state`, sealed under the state key in `secrets`, saves that state, then uploads them.
pub fn keep(
    connection: &mut Connection,
    token: &str,
    state: &State,
    keys: &Keys,
    secrets: &Secrets,
) -> Result<(), String> {
    let path = format!("/devices/{}/key-packages", keys.id());
    let answer = connection.get_as(&path, token).map_err(text)?;
    let count: Count = serde_json::from_slice(&answer)
        .map_err(|_| "the server's count of KeyPackages is malformed".to_owned())?;
    let (others, last) = wanted(&count);
    if others == 0 && !last {
        return Ok(());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let upload = state.mls_step(|sealed| {
        let fails = |err: skald_crypto::MlsError| format!("cannot make KeyPackages: {err}");
        // Under the state's write lock, so that runs at once never seal under two new keys.
        let state_key = state_key(secrets, sealed)?;
        let mut device = MlsDevice::load(keys.signing(), &state_key, sealed).map_err(fails)?;
        let key_packages = (0..others)
            .map(|_| {
                device
                    .key_package(now)
                    .map(|made| URL_SAFE_NO_PAD.encode(made.message))
            })
            .collect::<Result<_, _>>()
            .map_err(fails)?;
        let last_resort = last
            .then(|| device.last_resort_key_package(now))
            .transpose()
            .map_err(fails)?
            .map(|made| URL_SAFE_NO_PAD.encode(made.message));
        let upload = Upload {
            key_packages,
            last_resort,
        };
        Ok((Some(device.seal()), upload))
    })?;
    let body = serde_json::to_vec(&upload).expect("an upload is JSON");
    connection
        .send_json_as(Method::Post, &path, token, &body)
        .map_err(text)?;
    Ok(())
}

/// Warns that the device's KeyPackages are not kept, for the reason `err`.
pub fn warn(err: &str) {
    eprintln!("skald: warning: this device's KeyPackages on the server are not kept: {err}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(unused: u64, replace_last_resort: bool) -> Count {
        Count {
            unused,
            replace_last_resort,
        }
    }

    #[test]
    fn fewer_than_ten_are_topped_up_to_twenty() {
        assert_eq!(wanted(&count(0, true)), (20, true));
        assert_eq!(wanted(&count(9, false)), (11, false));
        assert_eq!(wanted(&count(10, false)), (0, false));
        assert_eq!(wanted(&count(25, true)), (0, true));
    }
}
