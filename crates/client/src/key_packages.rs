//! The device's KeyPackages on the server, as `docs/protocol/membership.md`, "KeyPackages", says:
//! with fewer than 10 unused that expire in more than 7 days, it uploads enough for 20; it
//! replaces its last-resort one once it was handed out or within 7 days of its expiry. Their
//! private keys join the device's MLS state, which is saved sealed before they are uploaded.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use skald_crypto::{MlsDevice, SealedState, StateKey};
use skald_proto::DeviceId;

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

/// Whether the server of `connection` holds an unused KeyPackage of `device`, a device of the
/// account of the session `token`.
pub fn held(connection: &mut Connection, token: &str, device: DeviceId) -> Result<bool, String> {
    let path = format!("/devices/{device}/key-packages");
    Ok(count(connection, token, &path)?.unused > 0)
}

/// The count of KeyPackages the server of `connection` gives at `path`, as the session `token`.
fn count(connection: &mut Connection, token: &str, path: &str) -> Result<Count, String> {
    let answer = connection.get_as(path, token).map_err(text)?;
    serde_json::from_slice(&answer)
        .map_err(|_| "the server's count of KeyPackages is malformed".to_owned())
}

/// The count of KeyPackages the server of `connection` gives at `path` for an upload of none, as
/// the service token `token`, which cannot `GET` it.
fn counted_by_upload(
    connection: &mut Connection,
    token: &str,
    path: &str,
) -> Result<Count, String> {
    let none = serde_json::to_vec(&Upload {
        key_packages: Vec::new(),
        last_resort: None,
    })
    .expect("an upload is JSON");
    let answer = (connection.send_json_as(Method::Post, path, token, &none)).map_err(text)?;
    serde_json::from_slice(&answer)
        .map_err(|_| "the server's count of KeyPackages is malformed".to_owned())
}

/// The device of `keys` keeps its KeyPackages on the server of `connection`, as the session
/// `token`: it asks how many the server holds and, if it needs more, makes them in its MLS state in
/// `state`, sealed under the state key in `secrets`, saves that state, then uploads them. A
/// `service` device, whose token counts none, asks with an upload of none.
pub fn keep(
    connection: &mut Connection,
    token: &str,
    (state, secrets): (&State, &Secrets),
    keys: &Keys,
    service: bool,
) -> Result<(), String> {
    let path = format!("/devices/{}/key-packages", keys.id());
    let count = if service {
        counted_by_upload(connection, token, &path)?
    } else {
        count(connection, token, &path)?
    };
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
