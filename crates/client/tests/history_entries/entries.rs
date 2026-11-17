//! What the tests share: the entries of a channel the server gives and those Ada's machine
//! uploads, and opening them with the history keys of Bob's device.

use skald_proto::{
    Cbor as _, ChannelId, Content, HistoryBody, HistoryEntry, HistoryItem, MessageId, SpaceId,
};

use crate::chain::key_of;
use crate::cloud::{Cloud, unb64};
use crate::fixtures::cert_of;
use crate::front::Front;
use crate::talk::{Talk, one_message_sent};
use crate::world::now;

/// What the CLI says of a message of its device that is not in the channel's history.
pub const NOT_IN_HISTORY: &str = "not in the history";

/// How far an entry's `timestamp` may be from the test's clock, in seconds.
const CLOCK_SLACK: u64 = 600;

/// The entries of `at`'s channel the server gives the session `token`, as their bytes.
#[track_caller]
pub fn entries(cloud: &Cloud, token: &str, (space, channel): (SpaceId, ChannelId)) -> Vec<Vec<u8>> {
    let path = format!("/spaces/{space}/channels/{channel}/history/entries");
    let (status, body) = cloud.get_as(&path, token);
    assert_eq!(status, 200, "GET {path}: {body}");
    body["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("GET {path}: {body}"))
        .iter()
        .map(|entry| unb64(entry.as_str().expect("an entry in base64url")))
        .collect()
}

/// As [`entries`], of the talk's channel, for the session of Ada's machine.
#[track_caller]
pub fn talk_entries(talk: &Talk) -> Vec<Vec<u8>> {
    entries(&talk.world.cloud, &talk.ada.token, talk.at())
}

/// `bytes`, an encoded entry, decoded; fails unless it decodes.
#[track_caller]
pub fn decoded(bytes: &[u8]) -> HistoryEntry {
    HistoryEntry::from_cbor(bytes)
        .unwrap_or_else(|err| panic!("an entry that does not decode: {err}"))
}

/// An entry a machine sent in a `history` frame: the `msg_id` the frame names, its bytes, and
/// whether the front forwarded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uploaded {
    pub msg_id: MessageId,
    pub bytes: Vec<u8>,
    pub forwarded: bool,
}

/// The entries of `channel` the front saw the machines send after its first `from` uploads, in
/// order.
pub fn entries_sent(front: &Front, channel: ChannelId, from: usize) -> Vec<Uploaded> {
    front
        .uploads()
        .split_off(from)
        .into_iter()
        .filter(|upload| upload.channel == channel)
        .filter_map(|upload| match upload.item {
            HistoryItem::Entry { msg_id, entry } => Some(Uploaded {
                msg_id,
                bytes: entry,
                forwarded: upload.forwarded,
            }),
            _ => None,
        })
        .collect()
}

/// Of [`entries_sent`], those whose frame names `msg_id`.
pub fn entries_of(
    front: &Front,
    channel: ChannelId,
    from: usize,
    msg_id: MessageId,
) -> Vec<Uploaded> {
    entries_sent(front, channel, from)
        .into_iter()
        .filter(|sent| sent.msg_id == msg_id)
        .collect()
}

/// The `msg_id` of the one message Ada's machine sent in the talk's channel after the front's first
/// `from` sends; fails unless there is one.
#[track_caller]
pub fn ada_s_message(talk: &Talk, from: usize) -> MessageId {
    one_message_sent(&talk.front, talk.channel, from)
}

/// Bob's device, the talk's first played one, takes in its mailbox; gives the content of the
/// message `msg_id` of the talk's channel it read; fails unless it read it.
#[track_caller]
pub fn bob_reads(talk: &mut Talk, msg_id: MessageId) -> Content {
    talk.played[0].take_in(&talk.world.cloud);
    let read = talk.played[0].read_in(talk.channel);
    let (_, decrypted) = read
        .iter()
        .find(|(id, _)| *id == msg_id)
        .unwrap_or_else(|| panic!("Bob's device did not read {msg_id}"));
    assert_eq!(decrypted.sender, talk.ada.device, "the sender of {msg_id}");
    Content::from_cbor(&decrypted.plaintext)
        .unwrap_or_else(|err| panic!("{msg_id} holds no content: {err:?}"))
}

/// The `msg_id` of `bytes`, an entry, if it opens with Bob's key of `epoch` and its signature
/// verifies with the certificate of Ada's device.
pub fn msg_id_in(talk: &Talk, bytes: &[u8], epoch: u64) -> Option<MessageId> {
    let key = talk.played[0].mls.history_key(talk.channel, epoch)?;
    let message = key.open_entry(&HistoryEntry::from_cbor(bytes).ok()?).ok()?;
    let cert = cert_of(&talk.world.cloud, &talk.ada);
    message.verify(&cert).ok().map(|body| body.msg_id)
}

/// Fails unless `bytes`, an entry, is of the talk's channel and `epoch`, opens with Bob's key of
/// `epoch`, and holds a body signed by Ada's device: her device as `sender`, the talk's channel,
/// `msg_id`, a `timestamp` near the test's clock, and the content of the message `msg_id` that Bob's
/// device reads. Gives the body.
#[track_caller]
pub fn assert_ada_s_entry(
    talk: &mut Talk,
    bytes: &[u8],
    (msg_id, epoch): (MessageId, u64),
) -> HistoryBody {
    let content = bob_reads(talk, msg_id);
    let entry = decoded(bytes);
    assert_eq!(
        entry.channel, talk.channel,
        "the channel of the entry of {msg_id}"
    );
    assert_eq!(entry.epoch, epoch, "the epoch of the entry of {msg_id}");
    let message = key_of(&talk.played[0], talk.channel, epoch)
        .open_entry(&entry)
        .unwrap_or_else(|err| panic!("the entry of {msg_id} does not open: {err:?}"));
    assert_eq!(
        message.sender(),
        talk.ada.device,
        "the sender of the entry of {msg_id}"
    );
    let cert = cert_of(&talk.world.cloud, &talk.ada);
    let body = message.verify(&cert).unwrap_or_else(|err| {
        panic!("the entry of {msg_id} is not signed by Ada's device: {err:?}")
    });
    assert_eq!(body.sender, talk.ada.device, "{body:?}");
    assert_eq!(body.channel, talk.channel, "{body:?}");
    assert_eq!(body.msg_id, msg_id, "{body:?}");
    assert!(
        body.timestamp.abs_diff(now()) <= CLOCK_SLACK,
        "the entry of {msg_id} states another time: {body:?}"
    );
    assert_eq!(
        body.content, content,
        "the content of the entry of {msg_id}"
    );
    body
}

/// Fails unless the server holds one entry of the talk's channel, which every entry of `msg_id`
/// Ada's machine sent after the front's first `from` uploads is, byte for byte, and which
/// [`assert_ada_s_entry`] passes with `epoch`.
#[track_caller]
pub fn assert_one_entry(talk: &mut Talk, from: usize, (msg_id, epoch): (MessageId, u64)) {
    let held = talk_entries(talk);
    assert_eq!(held.len(), 1, "not one entry on the server: {held:?}");
    let sent = entries_of(&talk.front, talk.channel, from, msg_id);
    assert!(!sent.is_empty(), "no entry of {msg_id} sent");
    for one in &sent {
        assert_eq!(
            one.bytes, held[0],
            "an entry of {msg_id} sent with other bytes than the server holds"
        );
    }
    assert_ada_s_entry(talk, &held[0], (msg_id, epoch));
}

/// The lines of `shown` that name `msg_id` and say it is not in the history.
pub fn said_not_in_history(shown: &str, msg_id: MessageId) -> Vec<String> {
    let msg_id = msg_id.to_string();
    shown
        .lines()
        .filter(|line| line.contains(&msg_id) && line.to_lowercase().contains(NOT_IN_HISTORY))
        .map(str::to_owned)
        .collect()
}
