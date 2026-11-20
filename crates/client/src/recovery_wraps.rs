//! The history keys wrapped to the recovery key (`docs/protocol/membership.md`, "Key chain",
//! "Wraps" and "Access"): a device linking with the recovery key, before it lets the key go, opens
//! the wraps to it of each channel of the spaces it follows whose last revision it accepts lists
//! its account.
//!
//! - It takes a wrap only if it is of that channel and to its account's recovery key, opens with the
//!   key's HPKE key pair and is signed by its `signer`, whose certificate, revoked or not, passes
//!   checks 1 and 2 of "Device checks" with the master key of an account that a revision of the
//!   space it accepts lists as a member. It drops any other with a warning.
//! - It keeps each key it takes in its MLS state, which it keeps only sealed, and records its epoch:
//!   each such epoch before its first judged one in the channel counts as judged clean
//!   ([`crate::key_chain`]), so it reads the history before it is in the channel, and links its
//!   first epoch there to them.

use std::collections::BTreeSet;

use skald_crypto::{HistoryKey, HpkeKey, MlsDevice, check_device, open_wrap};
use skald_proto::{Cbor as _, Cert, ChannelId, DeviceId, HistoryWrap, Roster, WrapRecipient};

use crate::connection::Connection;
use crate::device::Keys;
use crate::devices::text;
use crate::follow::{self, Own, SpaceDevices, master_of};
use crate::key_chain;
use crate::key_packages;
use crate::secrets::Secrets;
use crate::state::{MlsChanges, State};

/// Where the device takes the wraps from and keeps their keys: its connection and session, its
/// local state, its secrets and its keys.
pub struct Taking<'a> {
    pub connection: &'a mut Connection,
    pub token: &'a str,
    pub state: &'a State,
    pub secrets: &'a Secrets,
    pub keys: &'a Keys,
}

/// A key a wrap gave: of `epoch` of `channel`.
struct Taken {
    channel: ChannelId,
    epoch: u64,
    key: HistoryKey,
}

impl Taking<'_> {
    /// Opens with `recovery`, the recovery HPKE key pair of `own`'s account, the wraps to it of
    /// each channel of the spaces the device follows, and keeps their keys. Fails if the server
    /// does not give a space's revisions, devices or a channel's wraps, or the state fails.
    pub fn take(&mut self, own: &Own, recovery: &HpkeKey) -> Result<(), String> {
        let mut taken = Vec::new();
        for space in self.state.spaces().map_err(text)? {
            let held = follow::read(self.state, space.id, &space.name)?;
            let last = follow::decode(&held.last, &space.name)?;
            let given = follow::given(self.connection, self.token, space.id)?;
            let devices = follow::space_devices(self.connection, self.token, self.state, space.id)?;
            let followed = follow::follow(&last, &held.hashes, &given, &devices, own);
            let latest = followed.accepted.last().unwrap_or(&last);
            let mut hashes: BTreeSet<[u8; 32]> = held.hashes.values().copied().collect();
            hashes.extend(followed.earlier.iter().map(|(_, hash)| *hash));
            hashes.extend(followed.accepted.iter().map(Roster::hash));
            let revisions: Vec<&Roster> = std::iter::once(&last)
                .chain(given.iter().filter(|given| hashes.contains(&given.hash())))
                .collect();
            let channels = (latest.body().channels.iter())
                .filter(|channel| {
                    (channel.members.iter()).any(|entry| entry.account == own.account)
                })
                .map(|channel| channel.id);
            for channel in channels {
                let wraps = key_chain::listed_to(
                    self.connection,
                    self.token,
                    (space.id, channel),
                    key_chain::WRAPS,
                )?;
                let checks = Checks {
                    own,
                    revisions: &revisions,
                    devices: &devices,
                    channel,
                };
                taken.extend(wraps.iter().filter_map(|bytes| {
                    checks.open(bytes, recovery).unwrap_or_else(|why| {
                        eprintln!(
                            "skald: warning: dropped a wrap of the channel {channel} to the \
                             recovery key: {why}"
                        );
                        None
                    })
                }));
            }
        }
        if taken.is_empty() {
            return Ok(());
        }
        self.keep(taken)
    }

    /// Keeps the keys of `taken` in the device's sealed MLS state and records their epochs.
    fn keep(&self, taken: Vec<Taken>) -> Result<(), String> {
        let (state, secrets, keys) = (self.state, self.secrets, self.keys);
        state.mls_change(|sealed| {
            let state_key = key_packages::state_key(secrets, sealed)?;
            let mut device = MlsDevice::load(keys.signing(), &state_key, sealed)
                .map_err(|err| format!("this device's MLS state does not load: {err}"))?;
            let mut changes = MlsChanges::default();
            for Taken {
                channel,
                epoch,
                key,
            } in taken
            {
                device.keep_history_key(channel, epoch, key);
                changes.recovered.push((channel, epoch));
            }
            Ok((Some(device.seal()), changes, ()))
        })
    }
}

/// What the device checks the wraps of `channel` against: its account and pinned master key, the
/// revisions of the space it accepts and the devices of the space's accounts.
struct Checks<'c> {
    own: &'c Own,
    revisions: &'c [&'c Roster],
    devices: &'c SpaceDevices,
    channel: ChannelId,
}

impl Checks<'_> {
    /// The key of `bytes`, an encoded wrap, opened with `recovery`; none if it is not to this
    /// account's recovery key or of another channel; why it is dropped otherwise.
    fn open(&self, bytes: &[u8], recovery: &HpkeKey) -> Result<Option<Taken>, String> {
        let Ok(wrap) = HistoryWrap::from_cbor(bytes) else {
            return Err("it does not decode".to_owned());
        };
        let body = wrap.body();
        if body.recipient != WrapRecipient::Recovery(self.own.account) {
            return Ok(None);
        }
        if body.channel != self.channel {
            return Err("it names another channel".to_owned());
        }
        let epoch = body.epoch;
        let opened = open_wrap(&wrap, recovery)
            .map_err(|_| format!("the wrap of the epoch {epoch} does not open with it"))?;
        let signer = opened.signer();
        let cert = self.signer(signer).ok_or_else(|| {
            format!(
                "the signer of the wrap of the epoch {epoch}, the device {signer}, has no \
                 certificate chained to the master key of a member of the space"
            )
        })?;
        let key = opened.verify(cert).map_err(|_| {
            format!(
                "the signature of the wrap of the epoch {epoch} does not verify with its signer's \
                 certificate"
            )
        })?;
        Ok(Some(Taken {
            channel: self.channel,
            epoch,
            key,
        }))
    }

    /// The certificate of `signer`, revoked or not, that passes checks 1 and 2 with the master key
    /// of an account a revision of [`Checks::revisions`] lists as a member, of that account.
    fn signer(&self, signer: DeviceId) -> Option<&Cert> {
        self.revisions.iter().find_map(|roster| {
            roster.body().members.iter().find_map(|member| {
                let master = master_of(roster, self.own, member.account)?;
                let cert = check_device(&master, &self.devices.certs, signer).ok()?;
                (cert.body().account == member.account).then_some(cert)
            })
        })
    }
}
