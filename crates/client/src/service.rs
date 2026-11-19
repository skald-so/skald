//! A service agent's runs (`docs/service-agents.md`; `docs/protocol/membership.md`, "Service
//! device"): with its key secret and token in [`KEY_VARIABLE`] and [`TOKEN_VARIABLE`], `skald send`
//! and `skald recv` run as its `service` device, a member of the channels whose `services` list it.
//!
//! - Its keys come from the key secret, never from key storage; the token is its session, and the
//!   device's proof goes with each request that needs one.
//! - It pins the key secret's `master` as its account's master key and follows its space from the
//!   key secret's `anchor`, the start of the roster ([`crate::follow`]).
//! - Its state goes to the state directory, sealed under a key derived from the key secret
//!   ([`skald_crypto::StateKey::of_service`]), so the directory alone opens nothing; nothing goes to
//!   the key storage.
//! - Once the device has been in a group, a run without its earlier state refuses, so its counters
//!   do not restart.
//! - Its messages carry the grant of its token, which it checks against the space's last accepted
//!   revision first.

use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{StateKey, check_grant};
use skald_proto::{
    Address, Cbor as _, ChannelId, DeviceId, Grant, Roster, ServerName, ServiceKey, SpaceId,
};
use zeroize::Zeroizing;

use crate::connection::{Connection, printable};
use crate::device::Keys;
use crate::devices::text;
use crate::follow::{Own, SpaceDevices};
use crate::groups;
use crate::secrets::{self, Keyring, KeyringError, Secrets};
pub use crate::service_agents::{KEY_VARIABLE, TOKEN_VARIABLE};
use crate::spaces::now;

/// The token's own record, `GET`.
const OWN_RECORD: &str = "/service-token";

/// A service agent's run: its key secret and token.
pub struct Agent {
    pub key: ServiceKey,
    pub token: Zeroizing<String>,
}

impl Agent {
    /// The device's keys.
    pub fn keys(&self) -> Keys {
        Keys::of_service(&self.key.signing, &self.key.hpke)
    }

    /// Whom the device trusts its space's revisions by: its account and the master key it pins.
    pub fn own(&self) -> Own {
        Own {
            account: self.key.account,
            master: self.key.master,
        }
    }

    /// The secrets of the device of `keys` in the state directory `dir`: its keys and its state
    /// key, held in memory only.
    pub fn secrets(&self, dir: &std::path::Path, keys: &Keys) -> (secrets::Map, Secrets) {
        let state_key = StateKey::of_service(&self.key.signing, &self.key.hpke);
        let mut stored = secrets::Map::new();
        keys.store_with(&mut stored, &state_key);
        let held = Held(Mutex::new(Zeroizing::new(
            serde_json::to_string(&stored).expect("a map of strings is JSON"),
        )));
        (stored, Secrets::with(dir, Some(Box::new(held))))
    }
}

/// The service agent the run is, if [`KEY_VARIABLE`] and [`TOKEN_VARIABLE`] are set; fails if only
/// one is, or the key secret does not decode.
pub fn from_env() -> Result<Option<Agent>, String> {
    // A value that is no Unicode reads as none of a key secret or token.
    let read = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(|value| Zeroizing::new(value.into_string().unwrap_or_default()))
    };
    let (key, token) = match (read(KEY_VARIABLE), read(TOKEN_VARIABLE)) {
        (None, None) => return Ok(None),
        (Some(key), Some(token)) => (key, token),
        (Some(_), None) => return Err(alone(KEY_VARIABLE, TOKEN_VARIABLE)),
        (None, Some(_)) => return Err(alone(TOKEN_VARIABLE, KEY_VARIABLE)),
    };
    let invalid = || format!("{KEY_VARIABLE} holds no key secret of a service agent");
    let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(key.trim()).map_err(|_| invalid())?);
    let key = ServiceKey::from_cbor(&bytes).map_err(|_| invalid())?;
    if token.trim().is_empty() {
        return Err(format!("{TOKEN_VARIABLE} holds no service token"));
    }
    let token = Zeroizing::new(token.trim().to_owned());
    Ok(Some(Agent { key, token }))
}

/// What a run says with `set` set without `unset`.
fn alone(set: &str, unset: &str) -> String {
    format!(
        "{set} is set without {unset}: a service agent's run needs both, its key secret and its \
         token"
    )
}

/// The secrets of a service agent's run, in memory only: the key storage of [`Secrets`] that
/// keeps nothing.
struct Held(Mutex<Zeroizing<String>>);

impl Keyring for Held {
    fn get(&self) -> Result<Option<String>, KeyringError> {
        let held = self.0.lock().map_err(|_| KeyringError::Unavailable)?;
        Ok(Some(held.to_string()))
    }

    fn set(&self, value: &str) -> Result<(), KeyringError> {
        let mut held = self.0.lock().map_err(|_| KeyringError::Unavailable)?;
        *held = Zeroizing::new(value.to_owned());
        Ok(())
    }
}

/// Fails, sending nothing, if the device `this` has been in the group of a channel of `latest`,
/// the latest revision the server gives of its space, that lists it in its `services`: one whose
/// GroupInfo on the server lists it. A run without the device's earlier state would restart its
/// counters there.
pub fn refuse_without_state(
    connection: &mut Connection,
    token: &str,
    latest: &Roster,
    this: DeviceId,
) -> Result<(), String> {
    let space = latest.body().space;
    for channel in (latest.body().channels.iter()).filter(|it| it.services.contains(&this)) {
        if groups::group_info_lists(connection, token, (space, channel.id), this)? {
            return Err(format!(
                "this service device has been in the group of the channel {}, and its state \
                 directory holds no earlier state of it: keep the state directory of the agent's \
                 runs, or replace the agent's key; nothing is sent",
                channel.id
            ));
        }
    }
    Ok(())
}

/// The fields of `OwnServiceToken` in the API spec the CLI uses.
#[derive(Deserialize)]
struct OwnRecord {
    grant: String,
}

/// The grant of the token `token`, as its own record gives it.
pub fn grant(connection: &mut Connection, token: &str) -> Result<Grant, String> {
    let body = connection.get_as(OWN_RECORD, token).map_err(text)?;
    let unexpected = || "unexpected answer from the server: a token's record without its grant";
    let record: OwnRecord = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    let bytes = URL_SAFE_NO_PAD
        .decode(&record.grant)
        .map_err(|_| unexpected())?;
    Grant::from_cbor(&bytes).map_err(|_| unexpected().to_owned())
}

/// The address of the service agent of the device `this` in `roster`, its space's last accepted
/// revision, on `server`, and `grant`, its token's, which passes the checks of "Service grant"
/// against `roster` and `devices`, its accounts' devices, for `channel`. Fails, saying a new token
/// is needed, if the grant fails them, and if `roster` lists no entry of the device.
pub fn sender(
    (this, channel): (DeviceId, ChannelId),
    (roster, devices): (&Roster, &SpaceDevices),
    own: &Own,
    server: ServerName,
    grant: &Grant,
) -> Result<Address, String> {
    let body = roster.body();
    let name = body.name.as_str();
    let entry = (body.service_agents.iter())
        .find(|entry| entry.device == this)
        .ok_or_else(|| {
            format!(
                "the space {name} lists no service agent of this device in its revision {}; \
                 nothing is sent",
                body.revision
            )
        })?;
    check_grant(
        grant,
        (this, channel),
        roster,
        &devices.certs,
        &devices.revocations,
        (own.account, &own.master),
        now(),
    )
    .map_err(|err| {
        format!(
            "the grant of the service token fails its checks in the channel {channel}: {}; the \
             agent needs a new token, from skald create-service-token; nothing is sent",
            printable(&err.to_string())
        )
    })?;
    Ok(Address::new(
        body.name.clone(),
        entry.owner.clone(),
        entry.agent.clone(),
        server,
    ))
}

/// What a service agent's run follows: its space and the hash of the key secret's anchor.
#[derive(Clone, Copy)]
pub struct Member {
    pub space: SpaceId,
    pub anchor: [u8; 32],
}
