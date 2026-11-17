//! The wraps of the history keys of each channel on this device to the linked browsers of its
//! members (`docs/protocol/membership.md`, "Wraps" and "Uploads").
//!
//! - The device wraps the key of each epoch it judged clean ([`crate::key_chain`]) to each
//!   `browser` device with a valid certificate of an account among the channel's members, unless
//!   that member's `excluded` in the space's last revision the device accepted lists it: sealed to
//!   the browser's `hpke` key and signed by the device.
//! - Once its Commit is stored, the device uploads the new epoch's wraps. After its catch-up, it
//!   uploads the wraps of the current epoch that the server lacks, of the channels whose last
//!   accepted revision lists its account, so a browser linked later gets them.
//! - It does not upload again a wrap the server took from it, listed to it or refused.

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{MlsDevice, SigningKey};
use skald_proto::{
    Cbor as _, Cert, ChannelId, DeviceId, DeviceKind, HistoryItem, HistoryWrap, SpaceId,
    WrapRecipient,
};

use crate::connection::printable;
use crate::devices::text;
use crate::groups::{self, Link};
use crate::key_chain;
use crate::spaces::{Follower, Scope};

/// A wrap of an epoch's key the device may upload: of `epoch` of `channel` in `space`, to the
/// browser `browser` with its certificate's `hpke` key.
struct Due {
    space: SpaceId,
    channel: ChannelId,
    epoch: u64,
    browser: DeviceId,
    hpke: [u8; 32],
}

/// A wrap by its epoch and recipient.
type Held = (u64, WrapRecipient);

/// The browsers that get wraps of `channel` in `space`, each with its certificate's `hpke` key, by
/// the space's last revision the run accepted: those with a valid certificate of the accounts of
/// the channel's members that their member's `excluded` does not list. None if the run accepted
/// no revision of the space.
fn browsers(follower: &Follower, space: SpaceId, channel: ChannelId) -> Vec<(DeviceId, [u8; 32])> {
    let Some((roster, devices)) = follower.accepted.get(&space) else {
        return Vec::new();
    };
    let body = roster.body();
    let Some(listed) = body.channels.iter().find(|listed| listed.id == channel) else {
        return Vec::new();
    };
    (listed.members.iter())
        .filter_map(|entry| body.member(entry.account))
        .flat_map(|member| devices.of(roster, &follower.own, member))
        .map(Cert::body)
        .filter(|cert| cert.kind == DeviceKind::Browser)
        .map(|cert| (cert.device, cert.hpke))
        .collect()
}

/// The wraps of `epoch` of `channel` in `space` the device may upload: to each of [`browsers`] if
/// the device judged the epoch clean, unless settled.
fn due(follower: &Follower, (space, channel): (SpaceId, ChannelId), epoch: u64) -> Vec<Due> {
    let read = || -> Result<_, crate::state::Error> {
        Ok((
            follower.state.judgments(channel)?,
            follower.state.settled_wraps(channel)?,
        ))
    };
    let (judgments, settled) = match read() {
        Ok(read) => read,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return Vec::new();
        }
    };
    if judgments.get(&epoch) != Some(&true) {
        return Vec::new();
    }
    (browsers(follower, space, channel).into_iter())
        .filter(|(browser, _)| !settled.contains(&(epoch, WrapRecipient::Browser(*browser))))
        .map(|(browser, hpke)| Due {
            space,
            channel,
            epoch,
            browser,
            hpke,
        })
        .collect()
}

/// Each of `due`, sealed with the key `device` holds of its epoch and signed with `signer`; those
/// it lacks a key of are left out, and those that fail with a warning.
fn seal(device: &MlsDevice, signer: &SigningKey, due: &[Due]) -> Vec<(SpaceId, HistoryWrap)> {
    (due.iter())
        .filter_map(|due| {
            let key = device.history_key(due.channel, due.epoch)?;
            let recipient = WrapRecipient::Browser(due.browser);
            match key.wrap(due.channel, due.epoch, recipient, &due.hpke, signer) {
                Ok(wrap) => Some((due.space, wrap)),
                Err(err) => {
                    eprintln!(
                        "skald: warning: no wrap of the epoch {} in the channel {} to {}: {err}",
                        due.epoch,
                        due.channel,
                        whom(recipient)
                    );
                    None
                }
            }
        })
        .collect()
}

/// Uploads the wraps of `epoch` of `channel` in `space`, which this device's Commit there started
/// once the server stored it, if the device judged it clean; warns of what fails.
pub fn upload_new(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    epoch: u64,
) {
    let due = due(follower, (space, channel), epoch);
    upload_due(follower, link, &due);
}

/// After the catch-up of a run within `scope`: uploads the wraps of the current epoch of the
/// channels of [`key_chain::own_channels`] that the server lacks, settling those it lists; warns of
/// what fails.
pub fn upload_missing(follower: &mut Follower, link: &mut Link, scope: Scope) {
    let channels = key_chain::own_channels(follower, scope);
    let epochs = groups::read(follower, &link.keys, |device| {
        (channels.iter())
            .filter_map(|&at| Some((at, device.epoch(at.1).ok()?)))
            .collect::<Vec<_>>()
    });
    let epochs = match epochs {
        Ok(epochs) => epochs,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    let mut missing = Vec::new();
    for (at, epoch) in epochs {
        let due = due(follower, at, epoch);
        if due.is_empty() {
            continue;
        }
        match held_wraps(follower, at) {
            Ok(held) => {
                missing.extend(due.into_iter().filter(|due| {
                    !held.contains(&(due.epoch, WrapRecipient::Browser(due.browser)))
                }))
            }
            Err(err) => eprintln!(
                "skald: warning: the wraps of the channel {} the server holds are not checked: \
                 {err}; the next run checks them",
                at.1
            ),
        }
    }
    upload_due(follower, link, &missing);
}

/// Seals and uploads `due`; warns of what fails.
fn upload_due(follower: &mut Follower, link: &mut Link, due: &[Due]) {
    if due.is_empty() {
        return;
    }
    let keys = &link.keys;
    match groups::read(follower, keys, |device| seal(device, keys.signing(), due)) {
        Ok(sealed) => upload(follower, link, sealed),
        Err(err) => eprintln!("skald: warning: {err}"),
    }
}

/// `HistoryWraps` in the API spec.
#[derive(Deserialize)]
struct HistoryWraps {
    wraps: Vec<String>,
}

/// The wraps of `channel` in `space` the server lists, by epoch and recipient, each settled.
fn held_wraps(
    follower: &mut Follower,
    (space, channel): (SpaceId, ChannelId),
) -> Result<BTreeSet<Held>, String> {
    let path = format!("/spaces/{space}/channels/{channel}/history/wraps");
    let body = follower
        .connection
        .get_as(&path, &follower.token)
        .map_err(text)?;
    let unexpected = || "unexpected answer from the server: a channel's wraps".to_owned();
    let given: HistoryWraps = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    let held: BTreeSet<Held> = (given.wraps.iter())
        .filter_map(|wrap| {
            let wrap = HistoryWrap::from_cbor(&URL_SAFE_NO_PAD.decode(wrap).ok()?).ok()?;
            let body = wrap.body();
            (body.channel == channel).then_some((body.epoch, body.recipient))
        })
        .collect();
    let settled: Vec<Held> = held.iter().copied().collect();
    follower
        .state
        .settle_wraps(channel, &settled)
        .map_err(text)?;
    Ok(held)
}

/// Uploads `wraps`, each of its space, in `history` frames on the device's connection, and settles
/// each one the server stores or refuses; warns of a refusal and of what fails.
fn upload(follower: &mut Follower, link: &mut Link, wraps: Vec<(SpaceId, HistoryWrap)>) {
    let items = (wraps.into_iter())
        .map(|(space, wrap)| {
            let body = wrap.body();
            let known = (body.channel, (body.epoch, body.recipient));
            (
                (space, body.channel),
                HistoryItem::Wrap(wrap.to_cbor()),
                known,
            )
        })
        .collect();
    let unanswered = key_chain::upload_items(
        follower,
        link,
        items,
        |follower, (channel, (epoch, recipient)), refused| {
            if let Some(problem) = refused {
                eprintln!(
                    "skald: warning: the server refused this device's wrap of the epoch {epoch} in \
                     the channel {channel} to {}: {}",
                    whom(recipient),
                    printable(problem)
                );
            }
            (follower.state)
                .settle_wraps(channel, &[(epoch, recipient)])
                .map_err(text)
        },
    );
    let unanswered: BTreeSet<(ChannelId, u64)> = (unanswered.into_iter())
        .map(|(channel, (epoch, _))| (channel, epoch))
        .collect();
    for (channel, epoch) in unanswered {
        eprintln!(
            "skald: warning: the server has not answered this device's wraps of the epoch {epoch} \
             in the channel {channel}; a later run uploads those it lacks while the epoch is \
             current"
        );
    }
}

/// `recipient`, as a warning names it.
fn whom(recipient: WrapRecipient) -> String {
    match recipient {
        WrapRecipient::Browser(device) => format!("the browser {device}"),
        WrapRecipient::Recovery(account) => format!("the recovery key of the account {account}"),
    }
}
