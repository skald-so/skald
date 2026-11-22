//! The queue key objects of each channel on this device (`docs/protocol/queues.md`, "Queue key"
//! and "Queue key object").
//!
//! - Once its Commit is stored, the device uploads the object of the new epoch with its link and
//!   wraps, if it judged the epoch clean: the `pk` of the epoch's queue key, signed by the device.
//! - After its catch-up, of each channel of [`key_chain::own_channels`] whose current epoch it
//!   judged clean, it reads the server's object of that epoch: it uploads its own if the server
//!   has none, and warns its user of one whose `key` is not the `pk` it derives.
//! - It reads an epoch's object no more once it found one there, so it warns once per epoch, nor
//!   once the server refused its upload. An object the server took is read again by the next run,
//!   as the server may keep another in its place.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use skald_crypto::{MlsDevice, sign_queue_key};
use skald_proto::{Cbor as _, ChannelId, HistoryItem, QueueKey, QueueKeyBody, SpaceId, Unknown};

use crate::connection::{self, printable};
use crate::device::Keys;
use crate::devices::text;
use crate::groups::{self, Link};
use crate::key_chain::{self, Left};
use crate::spaces::{Follower, Scope};

/// The problem the server names when it keeps no object of the epoch.
const NO_QUEUE_KEY: &str = "no-queue-key";

/// The path of the queue key object of `epoch` of the channel `at`.
fn path((space, channel): (SpaceId, ChannelId), epoch: u64) -> String {
    format!("/spaces/{space}/channels/{channel}/queue-keys/{epoch}")
}

/// The bytes of the queue key object of `epoch` of the channel `at` that the server gives the
/// device of `follower`, proving it with `keys`; `None` when it keeps none.
pub fn held(
    follower: &mut Follower,
    keys: &Keys,
    at: (SpaceId, ChannelId),
    epoch: u64,
) -> Result<Option<Vec<u8>>, String> {
    let proof = (follower.connection)
        .request_proof(&follower.token, follower.this, keys.signing())
        .map_err(text)?;
    match (follower.connection).get_proved(&path(at, epoch), (&follower.token, &proof)) {
        Ok(answer) => {
            let given: serde_json::Value = serde_json::from_slice(&answer).unwrap_or_default();
            let bytes = (given["queue_key"].as_str())
                .and_then(|text| URL_SAFE_NO_PAD.decode(text).ok())
                .ok_or_else(|| {
                    "unexpected answer from the server: a queue key object".to_owned()
                })?;
            Ok(Some(bytes))
        }
        Err(connection::Error::Refused {
            status: 404, kind, ..
        }) if kind.rsplit('/').next() == Some(NO_QUEUE_KEY) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

/// The object of `epoch` of `channel` that `device` makes, with the `pk` of the key it holds of
/// it, signed with `link`'s keys; none if it holds no key of the epoch.
fn made(device: &MlsDevice, link: &Link, channel: ChannelId, epoch: u64) -> Option<QueueKey> {
    let key = device.history_key(channel, epoch)?.queue_key().public_key();
    object(link, channel, epoch, key)
}

/// The object of `epoch` of `channel` with `key`, signed with `link`'s keys by this device.
fn object(link: &Link, channel: ChannelId, epoch: u64, key: [u8; 32]) -> Option<QueueKey> {
    let body = QueueKeyBody {
        channel,
        epoch,
        key,
        signer: link.keys.id(),
        unknown: Unknown::default(),
    };
    sign_queue_key(&body, link.keys.signing())
        .map_err(|err| eprintln!("skald: warning: no queue key object of the epoch {epoch}: {err}"))
        .ok()
}

/// Uploads the object of `epoch` of `channel` in `space`, which this device's Commit there started
/// once the server stored it and the device judged clean; warns of what fails.
pub fn upload_new(
    follower: &mut Follower,
    link: &mut Link,
    (space, channel): (SpaceId, ChannelId),
    epoch: u64,
) {
    match groups::read(follower, &link.keys, |device| {
        made(device, link, channel, epoch)
    }) {
        Ok(Some(object)) => upload(follower, link, vec![(space, object)]),
        Ok(None) => {}
        Err(err) => eprintln!("skald: warning: {err}"),
    }
}

/// After the catch-up of a run within `scope`: of each channel of [`key_chain::own_channels`]
/// whose current epoch the device judged clean and has not settled, reads the server's object of
/// that epoch; uploads the device's own if the server has none, and warns of one whose `key` is
/// not the `pk` the device derives. Warns of what fails.
pub fn upload_missing(follower: &mut Follower, link: &mut Link, scope: Scope) {
    let mut due = Vec::new();
    for at in key_chain::own_channels(follower, scope) {
        let read = |state: &crate::state::State, channel| state.settled_queue_keys(channel);
        let Some((judgments, settled)) = key_chain::judged(follower, at.1, read) else {
            continue;
        };
        // The device judges each epoch as it enters it: its latest judged one is its current.
        if let Some((&epoch, true)) = judgments.last_key_value()
            && !settled.contains(&epoch)
        {
            due.push((at, epoch));
        }
    }
    if due.is_empty() {
        return;
    }
    let made = groups::read(follower, &link.keys, |device| {
        (due.iter())
            .filter(|&&(at, epoch)| device.epoch(at.1).ok() == Some(epoch))
            .filter_map(|&(at, epoch)| Some((at, made(device, link, at.1, epoch)?)))
            .collect::<Vec<_>>()
    });
    let made = match made {
        Ok(made) => made,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    let mut missing = Vec::new();
    for (at, own) in made {
        let epoch = own.body().epoch;
        match held(follower, &link.keys, at, epoch) {
            Ok(None) => missing.push((at.0, own)),
            Ok(Some(bytes)) => {
                if !matches(&bytes, own.body()) {
                    eprintln!(
                        "skald: warning: the server's queue key object of the epoch {epoch} in the \
                         channel {} holds another key than this device derives: tasks that service \
                         agents outside the channel's group enqueue in it may open on no device",
                        at.1
                    );
                }
                if let Err(err) = follower.state.settle_queue_key(at.1, epoch) {
                    eprintln!("skald: warning: {err}");
                }
            }
            Err(err) => eprintln!(
                "skald: warning: the queue key object of the epoch {epoch} in the channel {} is \
                 not checked: {err}; the next run checks it",
                at.1
            ),
        }
    }
    upload(follower, link, missing);
}

/// Whether `bytes` are an object of the channel and epoch of `own` with its `key`.
fn matches(bytes: &[u8], own: &QueueKeyBody) -> bool {
    QueueKey::from_cbor(bytes).is_ok_and(|given| {
        let body = given.body();
        body.channel == own.channel && body.epoch == own.epoch && body.key == own.key
    })
}

/// Uploads `objects`, each of its space, in `history` frames on the device's connection; settles
/// each one the server refuses, warning of it, and warns of what fails.
fn upload(follower: &mut Follower, link: &mut Link, objects: Vec<(SpaceId, QueueKey)>) {
    let items = (objects.into_iter())
        .map(|(space, object)| {
            let body = object.body();
            let known = (body.channel, body.epoch);
            (
                (space, body.channel),
                HistoryItem::QueueKey(object.to_cbor()),
                known,
            )
        })
        .collect();
    let left = key_chain::upload_items(
        follower,
        link,
        items,
        |follower, (channel, epoch), refused| {
            let Some(problem) = refused else {
                return Ok(());
            };
            eprintln!(
                "skald: warning: the server refused this device's queue key object of the epoch \
             {epoch} in the channel {channel}: {}",
                printable(problem)
            );
            follower
                .state
                .settle_queue_key(channel, epoch)
                .map_err(text)
        },
    );
    // Off WebSocket they wait for a run on one unsaid, as wraps do.
    let Left::Unanswered(unanswered) = left else {
        return;
    };
    for (channel, epoch) in unanswered {
        eprintln!(
            "skald: warning: the server has not answered this device's queue key object of the \
             epoch {epoch} in the channel {channel}; a later run uploads it if the server lacks it"
        );
    }
}
