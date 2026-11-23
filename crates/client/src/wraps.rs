//! The wraps of the history keys of each channel on this device to the recovery keys and the linked
//! browsers of its members (`docs/protocol/membership.md`, "Wraps" and "Uploads").
//!
//! - The device wraps the key of each epoch it judged clean ([`crate::key_chain`]) to the recovery
//!   key of each account among the channel's members in the space's last revision the device
//!   accepted, sealed to the recovery HPKE key of the account record it took ([`crate::records`]),
//!   none for an account without one; and to each `browser` device with a valid certificate of
//!   those accounts, unless that member's `excluded` there lists it, sealed to the browser's `hpke`
//!   key; none to an account the server reports deleted. The device signs each wrap.
//! - Once its Commit is stored, the device uploads the new epoch's wraps. After its catch-up, it
//!   uploads the wraps of the current epoch that the server lacks, of the channels whose last
//!   accepted revision lists its account, so a browser linked later gets them.
//! - It does not upload again a wrap the server took from it, listed to it or refused. Off
//!   WebSocket, which alone carries them, the wraps wait for a later run on one, with no warning.

use std::collections::{BTreeMap, BTreeSet};

use skald_crypto::{MlsDevice, SigningKey};
use skald_proto::{
    Cbor as _, Cert, ChannelId, DeviceKind, HistoryItem, HistoryWrap, SpaceId, WrapRecipient,
};

use crate::connection::printable;
use crate::devices::text;
use crate::groups::{self, Link};
use crate::key_chain::{self, Left};
use crate::records;
use crate::spaces::{Follower, Scope};
use crate::state::State;

/// A wrap of an epoch's key the device may upload: of `epoch` of `channel` in `space`, to
/// `recipient`, sealed to its HPKE public key `hpke`.
struct Due {
    space: SpaceId,
    channel: ChannelId,
    epoch: u64,
    recipient: WrapRecipient,
    hpke: [u8; 32],
}

/// A wrap by its epoch and recipient.
type Held = (u64, WrapRecipient);

/// The recipients of the wraps of `channel` in `space`, each with its HPKE public key, by the
/// space's last revision the run accepted: of each account among the channel's members, its
/// recovery key, by the record the device takes of it, and its browsers with a valid certificate
/// that its member's `excluded` does not list; none to an account the server reports deleted. None
/// if the run accepted no revision of the space; warns of a record the state fails to give.
fn recipients(
    follower: &Follower,
    space: SpaceId,
    channel: ChannelId,
) -> BTreeMap<WrapRecipient, [u8; 32]> {
    let mut recipients = BTreeMap::new();
    let Some((roster, devices)) = follower.accepted.get(&space) else {
        return recipients;
    };
    let body = roster.body();
    let Some(listed) = body.channels.iter().find(|listed| listed.id == channel) else {
        return recipients;
    };
    let members = (listed.members.iter())
        .filter_map(|entry| body.member(entry.account))
        .filter(|member| !devices.deleted.contains(&member.account));
    for member in members {
        let account = member.account;
        match records::of(&follower.state, (roster, devices), &follower.own, account) {
            Ok(Some(record)) => {
                recipients.insert(WrapRecipient::Recovery(account), record.body().recovery);
            }
            Ok(None) => {}
            Err(err) => eprintln!("skald: warning: {err}"),
        }
        let browsers = (devices.of(roster, &follower.own, member).into_iter())
            .map(Cert::body)
            .filter(|cert| cert.kind == DeviceKind::Browser);
        for cert in browsers {
            recipients.insert(WrapRecipient::Browser(cert.device), cert.hpke);
        }
    }
    recipients
}

/// The wraps of `epoch` of `channel` in `space` the device may upload: to each of [`recipients`]
/// if the device judged the epoch clean, unless settled.
fn due(follower: &Follower, (space, channel): (SpaceId, ChannelId), epoch: u64) -> Vec<Due> {
    let Some((judgments, settled)) = key_chain::judged(follower, channel, State::settled_wraps)
    else {
        return Vec::new();
    };
    if judgments.get(&epoch) != Some(&true) {
        return Vec::new();
    }
    (recipients(follower, space, channel).into_iter())
        .filter(|(recipient, _)| !settled.contains(&(epoch, *recipient)))
        .map(|(recipient, hpke)| Due {
            space,
            channel,
            epoch,
            recipient,
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
            let recipient = due.recipient;
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
            Ok(held) => missing.extend(
                (due.into_iter()).filter(|due| !held.contains(&(due.epoch, due.recipient))),
            ),
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

/// The wraps of `channel` in `space` the server lists, by epoch and recipient, each settled.
fn held_wraps(
    follower: &mut Follower,
    (space, channel): (SpaceId, ChannelId),
) -> Result<BTreeSet<Held>, String> {
    let given = key_chain::listed(follower, (space, channel), key_chain::WRAPS)?;
    let held: BTreeSet<Held> = (given.iter())
        .filter_map(|bytes| {
            let wrap = HistoryWrap::from_cbor(bytes).ok()?;
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
    let left = key_chain::upload_items(
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
    // Off WebSocket they wait for a run on one unsaid: each run of the person's would say it, and
    // what a run says ends the person's `skald recv --wait`.
    let Left::Unanswered(unanswered) = left else {
        return;
    };
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
