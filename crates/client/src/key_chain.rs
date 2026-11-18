//! The key chain of each channel on this device (`docs/protocol/membership.md`, "Clean epochs",
//! "Key chain" and "Uploads").
//!
//! - The device judges each epoch of a channel once, when it enters it ([`crate::groups`]): clean
//!   when every member device of it is admitted by the space's last revision it accepted. A
//!   revision it accepts later leaves the judgment; an epoch it holds no judgment of counts as
//!   unclean.
//! - The link of a clean epoch holds the key of `prev`, the latest epoch from 1 on before it that
//!   the device judged clean; with none, there is no link.
//! - Once its Commit is stored, the device uploads the link of the new epoch. After its catch-up,
//!   it uploads the links it can make that the server lacks, of the channels whose latest revision
//!   it accepted lists its account. It does not upload again a link the server took from it, listed
//!   to it or refused, so one retention deleted stays deleted.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::{MemberDevice, MlsDevice};
use skald_proto::{Cbor as _, Cert, ChannelId, DeviceId, Frame, HistoryItem, HistoryLink, SpaceId};
use tungstenite::Message;

use crate::devices::text;
use crate::groups::{self, Link};
use crate::send::{broke, closed, set_read_timeout, timed_out};
use crate::spaces::{Follower, Scope};
use crate::state::{self, State};

/// An epoch of a link and its `prev`.
type Pair = (u64, u64);

/// Whether an epoch whose member devices are `members` is clean: each is one of `admitted`, with
/// its certificate's signing key.
pub fn clean(admitted: &BTreeMap<DeviceId, Cert>, members: &[MemberDevice]) -> bool {
    members
        .iter()
        .all(|member| groups::admits(admitted, member))
}

/// The `prev` of the link of `epoch` by `judgments`: the latest epoch from 1 on before it judged
/// clean.
fn prev(judgments: &BTreeMap<u64, bool>, epoch: u64) -> Option<u64> {
    (judgments.range(1..epoch).rev())
        .find(|(_, clean)| **clean)
        .map(|(prev, _)| *prev)
}

/// The links of `channel` the device's judgments give that it may upload, each of an epoch
/// `wanted` holds: of each clean epoch with a `prev`, unless settled.
fn due(follower: &Follower, channel: ChannelId, wanted: impl Fn(u64) -> bool) -> Vec<Pair> {
    let Some((judgments, settled)) = judged(follower, channel, State::settled_links) else {
        return Vec::new();
    };
    (judgments.iter())
        .filter(|&(&epoch, &clean)| clean && wanted(epoch))
        .filter_map(|(&epoch, _)| Some((epoch, prev(&judgments, epoch)?)))
        .filter(|pair| !settled.contains(pair))
        .collect()
}

/// The device's judgments of the epochs of `channel` and what `settled` reads of it in the state;
/// none, with a warning, if the state fails.
pub fn judged<T>(
    follower: &Follower,
    channel: ChannelId,
    settled: impl FnOnce(&State, ChannelId) -> Result<T, state::Error>,
) -> Option<(BTreeMap<u64, bool>, T)> {
    let read = || {
        Ok((
            follower.state.judgments(channel)?,
            settled(&follower.state, channel)?,
        ))
    };
    read()
        .map_err(|err: state::Error| eprintln!("skald: warning: {err}"))
        .ok()
}

/// Each of `due`, as a link of its channel sealed with the keys `device` holds; those it lacks a
/// key of are left out.
fn seal(device: &MlsDevice, due: &[(SpaceId, ChannelId, Pair)]) -> Vec<(SpaceId, HistoryLink)> {
    (due.iter())
        .filter_map(|&(space, channel, (epoch, prev))| {
            let key = device.history_key(channel, epoch)?;
            let prev_key = device.history_key(channel, prev)?;
            let link = key.seal_link(channel, epoch, prev, prev_key).ok()?;
            Some((space, link))
        })
        .collect()
}

/// Uploads the link of `epoch` of `channel` in `space`, which this device's Commit there started
/// once the server stored it, if the device judged it clean; warns of what fails.
pub fn upload_new(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    epoch: u64,
) {
    let due: Vec<_> = (due(follower, channel, |of| of == epoch).into_iter())
        .map(|pair| (space, channel, pair))
        .collect();
    if due.is_empty() {
        return;
    }
    match groups::read(follower, &link.keys, |device| seal(device, &due)) {
        Ok(sealed) => upload(follower, link, sealed),
        Err(err) => eprintln!("skald: warning: {err}"),
    }
}

/// After the catch-up of a run within `scope`: uploads the links of the channels of each space the
/// run accepted the revisions of, those whose last accepted revision lists the device's account,
/// that the device can make and the server lacks, settling those the server lists; warns of what
/// fails.
pub fn upload_missing(follower: &mut Follower, link: &mut Link, scope: Scope) {
    let due: Vec<(SpaceId, ChannelId, Pair)> = (own_channels(follower, scope).into_iter())
        .flat_map(|(space, channel)| {
            (due(follower, channel, |_| true).into_iter()).map(move |pair| (space, channel, pair))
        })
        .collect();
    if due.is_empty() {
        return;
    }
    let sealed = match groups::read(follower, &link.keys, |device| seal(device, &due)) {
        Ok(sealed) => sealed,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    let mut listed: BTreeMap<ChannelId, Option<BTreeSet<Pair>>> = BTreeMap::new();
    let mut missing = Vec::new();
    for (space, made) in sealed {
        let channel = made.channel;
        let held = listed.entry(channel).or_insert_with(|| {
            match held_links(follower, (space, channel)) {
                Ok(held) => Some(held),
                Err(err) => {
                    eprintln!(
                        "skald: warning: the links of the channel {channel} the server holds are \
                         not checked: {err}; the next run checks them"
                    );
                    None
                }
            }
        });
        if held
            .as_ref()
            .is_some_and(|held| !held.contains(&(made.epoch, made.prev)))
        {
            missing.push((space, made));
        }
    }
    upload(follower, link, missing);
}

/// The channels within `scope` of each space the run accepted the revisions of, those whose last
/// accepted revision lists the device's account.
pub fn own_channels(follower: &Follower, scope: Scope) -> Vec<(SpaceId, ChannelId)> {
    let account = follower.own.account;
    (follower.accepted.iter())
        .filter(|(space, _)| !scope.excludes(**space))
        .flat_map(|(space, (roster, _))| {
            (roster.body().channels.iter())
                .filter(|channel| channel.members.iter().any(|entry| entry.account == account))
                .map(|channel| (*space, channel.id))
        })
        .collect()
}

/// The links of `channel` in `space` the server lists, by epoch and `prev`, each settled.
fn held_links(follower: &mut Follower, at: (SpaceId, ChannelId)) -> Result<BTreeSet<Pair>, String> {
    let held: BTreeSet<Pair> = (links(follower, at)?.iter())
        .map(|link| (link.epoch, link.prev))
        .collect();
    let pairs: Vec<Pair> = held.iter().copied().collect();
    follower.state.settle_links(at.1, &pairs).map_err(text)?;
    Ok(held)
}

/// The links of `channel` in `space` the server lists that decode as links of it.
pub fn links(
    follower: &mut Follower,
    (space, channel): (SpaceId, ChannelId),
) -> Result<Vec<HistoryLink>, String> {
    Ok((listed(follower, (space, channel), LINKS)?.iter())
        .filter_map(|bytes| HistoryLink::from_cbor(bytes).ok())
        .filter(|link| link.channel == channel)
        .collect())
}

/// The kinds of history items the server lists, as their paths and lists name them.
pub const LINKS: &str = "links";
pub const WRAPS: &str = "wraps";
pub const ENTRIES: &str = "entries";

/// `HistoryLinks`, `HistoryWraps` and `HistoryEntries` in the API spec: the list named by its kind.
#[derive(Deserialize)]
struct Listed(BTreeMap<String, Vec<String>>);

/// The items of `kind`, [`LINKS`], [`WRAPS`] or [`ENTRIES`], of `channel` in `space` the server
/// lists, each as its bytes; those that are no base64url are left out.
pub fn listed(
    follower: &mut Follower,
    (space, channel): (SpaceId, ChannelId),
    kind: &str,
) -> Result<Vec<Vec<u8>>, String> {
    let path = format!("/spaces/{space}/channels/{channel}/history/{kind}");
    let body = follower
        .connection
        .get_as(&path, &follower.token)
        .map_err(text)?;
    let unexpected = || format!("unexpected answer from the server: a channel's history {kind}");
    let Listed(mut given) = serde_json::from_slice(&body).map_err(|_| unexpected())?;
    let items = given.remove(kind).ok_or_else(unexpected)?;
    Ok((items.iter())
        .filter_map(|item| URL_SAFE_NO_PAD.decode(item).ok())
        .collect())
}

/// Uploads `links`, each of its space, in `history` frames on the device's connection, and settles
/// each one the server stores or refuses; warns of a refusal and of what fails.
fn upload(follower: &mut Follower, link: &mut Link, links: Vec<(SpaceId, HistoryLink)>) {
    let items = (links.into_iter())
        .map(|(space, made)| {
            let pair = (made.epoch, made.prev);
            let item = HistoryItem::Link(made.to_cbor());
            ((space, made.channel), item, (made.channel, pair))
        })
        .collect();
    let unanswered = upload_items(
        follower,
        link,
        items,
        |follower, (channel, pair), refused| {
            if let Some(problem) = refused {
                eprintln!(
                    "skald: warning: the server refused this device's link of the epoch {} in the \
                 channel {channel}: {}",
                    pair.0,
                    crate::connection::printable(problem)
                );
            }
            follower.state.settle_links(channel, &[pair]).map_err(text)
        },
    );
    for (channel, (epoch, _)) in unanswered {
        eprintln!(
            "skald: warning: the server has not answered this device's link of the epoch {epoch} \
             in the channel {channel}; the next run uploads it again"
        );
    }
}

/// Uploads `items`, each a history item of its space and channel with what the caller knows it by,
/// in `history` frames on the device's connection, then reads the server's answers for the run's
/// timeout, giving each answered one to `answered` with the problem of a refusal, if any. Gives
/// those the server has not answered; warns of what fails.
pub fn upload_items<T>(
    follower: &mut Follower,
    link: &mut Link,
    items: Vec<((SpaceId, ChannelId), HistoryItem, T)>,
    answered: impl FnMut(&Follower, T, Option<&str>) -> Result<(), String>,
) -> Vec<T> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut waiting: BTreeMap<u64, T> = BTreeMap::new();
    let mut frames = Vec::new();
    for ((space, channel), item, known) in items {
        link.uploads += 1;
        waiting.insert(link.uploads, known);
        let frame = Frame::History {
            upload: link.uploads,
            space,
            channel,
            item,
        };
        frames.push(frame.to_cbor());
    }
    if let Err(err) = answers(follower, link, &frames, &mut waiting, answered) {
        eprintln!("skald: warning: {err}");
    }
    waiting.into_values().collect()
}

/// Sends `frames` on the device's connection, then reads the server's answers to the uploads of
/// `waiting` for the run's timeout, taking each answered one out and giving it to `answered`;
/// fails if the connection ends, or is not a WebSocket, which alone carries them, or `answered`
/// fails.
fn answers<T>(
    follower: &mut Follower,
    link: &mut Link,
    frames: &[Vec<u8>],
    waiting: &mut BTreeMap<u64, T>,
    mut answered: impl FnMut(&Follower, T, Option<&str>) -> Result<(), String>,
) -> Result<(), String> {
    let mut mailbox = match link.mailbox.take() {
        Some(mailbox) => mailbox,
        None => follower.connect(&link.keys)?,
    };
    let Some((socket, pushed)) = mailbox.websocket() else {
        link.mailbox = Some(mailbox);
        return Err(
            "history items go to the server over a WebSocket only, and this network lets none \
             through"
                .to_owned(),
        );
    };
    for frame in frames {
        socket
            .send(Message::Binary(frame.clone().into()))
            .map_err(broke)?;
    }
    let deadline = Instant::now() + follower.timeout;
    while !waiting.is_empty() {
        let wait = deadline.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            break;
        }
        set_read_timeout(socket, wait).map_err(|err| broke(err.into()))?;
        let (upload, refused) = match socket.read() {
            Ok(Message::Binary(bytes)) => match Frame::from_cbor(&bytes) {
                Ok(Frame::HistoryStored { upload }) => (upload, None),
                Ok(Frame::HistoryRefused { upload, problem }) => (upload, Some(problem)),
                Ok(Frame::Push(push)) => {
                    crate::mailbox::keep_push(pushed, push);
                    continue;
                }
                // Another frame, or one it does not know or that does not decode, ignored.
                _ => continue,
            },
            Ok(Message::Close(frame)) => return Err(closed(frame.as_ref())),
            Ok(_) => continue,
            Err(err) if timed_out(&err) => break,
            Err(err) => return Err(broke(err)),
        };
        let Some(known) = waiting.remove(&upload) else {
            continue;
        };
        answered(follower, known, refused.as_deref())?;
    }
    link.mailbox = Some(mailbox);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link's `prev` is the latest epoch from 1 on before it judged clean; epoch 0 never is one.
    #[test]
    fn prev_is_the_latest_clean_epoch_before() {
        let judgments = BTreeMap::from([(0, true), (1, true), (2, false), (3, true), (5, true)]);
        assert_eq!(prev(&judgments, 1), None);
        assert_eq!(prev(&judgments, 2), Some(1));
        assert_eq!(prev(&judgments, 3), Some(1));
        assert_eq!(prev(&judgments, 4), Some(3));
        assert_eq!(prev(&judgments, 5), Some(3));
        assert_eq!(prev(&judgments, 9), Some(5));
        assert_eq!(prev(&BTreeMap::from([(2, false), (3, true)]), 4), Some(3));
        assert_eq!(prev(&BTreeMap::from([(1, false), (2, false)]), 3), None);
    }
}
