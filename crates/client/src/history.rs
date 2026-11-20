//! The history entries of the device's channel messages (`docs/protocol/membership.md`, "History
//! entry", "Clean epochs" and "Uploads").
//!
//! - The device writes an entry of each channel message with an `a2a` body it sends, under the key
//!   of the epoch it encrypts the message in, if it judged that epoch clean ([`crate::key_chain`]):
//!   the content, signed by the device. The entry goes into the outbox with the message's envelope.
//! - Once the server stored the envelope, the device uploads the entry, by that run or a later one,
//!   with the same bytes until the server stores or refuses it. A refused entry is not uploaded
//!   again, and the run says the message is not in the history; the entry of a refused envelope
//!   goes with it. An entry is never written again, so never under a later epoch's key.

use std::collections::BTreeMap;

use skald_crypto::MlsDevice;
use skald_proto::{
    Body, Cbor as _, ChannelId, Content, HistoryBody, HistoryItem, MessageId, SpaceId, Unknown,
};

use crate::connection::printable;
use crate::device::Keys;
use crate::devices::text;
use crate::groups::Link;
use crate::key_chain;
use crate::spaces::{Follower, Scope};
use crate::state::{self, HistoryOutgoing};

/// The entry of the message `msg_id` of `content` that `device`, with `keys`, encrypts in
/// `channel` of `space` at `epoch`, if the message has an `a2a` body and `judgments`, the device's
/// of the channel's epochs, hold `epoch` clean; none otherwise, or if the device holds no key of
/// the epoch.
pub fn entry(
    (device, keys): (&MlsDevice, &Keys),
    (space, channel, epoch): (SpaceId, ChannelId, u64),
    judgments: &BTreeMap<u64, bool>,
    msg_id: MessageId,
    content: &Content,
) -> Option<HistoryOutgoing> {
    if !written(content, judgments, epoch) {
        return None;
    }
    let body = HistoryBody {
        sender: keys.id(),
        channel,
        msg_id,
        timestamp: state::unix_now(),
        content: content.clone(),
        unknown: Unknown::default(),
    };
    let key = device.history_key(channel, epoch)?;
    match key.seal_entry(epoch, &body, keys.signing()) {
        Ok(entry) => Some(HistoryOutgoing {
            msg_id,
            space,
            channel,
            entry: entry.to_cbor(),
        }),
        Err(err) => {
            eprintln!("skald: warning: the message {msg_id} is not in the history: {err}");
            None
        }
    }
}

/// Whether a message of `content` sent at `epoch` has an entry: with an `a2a` body, in an epoch
/// `judgments` hold clean.
fn written(content: &Content, judgments: &BTreeMap<u64, bool>, epoch: u64) -> bool {
    matches!(content.body, Some(Body::A2a(_))) && judgments.get(&epoch) == Some(&true)
}

/// Uploads the entries within `scope` whose messages the server stored, in `history` frames on
/// the device's connection, and takes each one the server stores or refuses out of the outbox;
/// says of each refused one that its message is not in the history, and warns of what fails.
pub fn upload(follower: &mut Follower, link: &mut Link, scope: Scope) {
    let waiting = match follower.state.history_outbox() {
        Ok(waiting) => waiting,
        Err(err) => {
            eprintln!("skald: warning: {err}");
            return;
        }
    };
    let items = (waiting.into_iter())
        .filter(|entry| !scope.excludes(entry.space))
        .map(|entry| {
            let item = HistoryItem::Entry {
                msg_id: entry.msg_id,
                entry: entry.entry,
            };
            ((entry.space, entry.channel), item, entry.msg_id)
        })
        .collect();
    let unanswered = key_chain::upload_items(follower, link, items, |follower, msg_id, refused| {
        if let Some(problem) = refused {
            eprintln!(
                "skald: warning: the message {msg_id} is not in the history: the server refused \
                 its entry: {}",
                printable(problem)
            );
        }
        follower.state.remove_history_entry(msg_id).map_err(text)
    });
    for msg_id in unanswered.warned() {
        eprintln!(
            "skald: warning: the server has not answered the history entry of the message \
             {msg_id}; the next run uploads it again"
        );
    }
}

#[cfg(test)]
mod tests {
    use skald_proto::{Address, Name, ServerName};

    use super::*;

    /// Only a message with an `a2a` body in an epoch judged clean has an entry: none in an epoch
    /// judged unclean or not judged, and none of another body.
    #[test]
    fn only_a2a_messages_in_clean_epochs_have_entries() {
        let name = |name: &str| name.parse::<Name>().unwrap();
        let server: ServerName = "skald.example".parse().unwrap();
        let from = Address::person(name("acme"), name("ada"), server);
        let content = |body| Content {
            from: from.clone(),
            to: None,
            counter: None,
            hops: 0,
            reply_to: None,
            body,
            delegation: None,
            grant: None,
            attachments: None,
            unknown: Unknown::default(),
        };
        let a2a = content(Some(Body::A2a("{}".to_owned())));
        let judgments = BTreeMap::from([(1, true), (2, false), (3, true)]);

        assert!(written(&a2a, &judgments, 1));
        assert!(written(&a2a, &judgments, 3));
        assert!(!written(&a2a, &judgments, 2));
        assert!(!written(&a2a, &judgments, 4));
        assert!(!written(&content(None), &judgments, 1));
    }
}
