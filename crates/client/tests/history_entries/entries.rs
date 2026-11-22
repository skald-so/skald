//! What the tests share: the entries of a channel the server gives and those Ada's machine
//! uploads, and opening them with the history keys of Bob's device.

use skald_proto::{
    Cbor as _, ChannelId, Content, HistoryBody, HistoryEntry, HistoryItem, MessageId, Roster,
    SpaceId,
};

use crate::chain::{adds_to_channel, key_of, listed, uploads_in};
use crate::cloud::Cloud;
use crate::fixtures::{cert_of, kill_once};
use crate::front::Front;
use crate::kit::{RECV, rosters};
use crate::played::Played;
use crate::talk::{Talk, commit_sent, one_message_sent};
use crate::world::{now, revisions_request, serve_revisions};

/// What the CLI says of a message of its device that is not in the channel's history.
pub const NOT_IN_HISTORY: &str = "not in the history";

/// How far an entry's `timestamp` may be from the test's clock, in seconds.
const CLOCK_SLACK: u64 = 600;

/// From now on, the relay gives Ada's machine the revisions of the talk's space the server holds
/// now, and no later one.
pub fn hide_later_revisions(talk: &Talk) {
    let space = talk.space.id;
    let now: Vec<Roster> = rosters(talk.cloud(), &talk.ada.token, space);
    serve_revisions(talk.cloud(), space, &now.iter().collect::<Vec<&Roster>>());
}

/// While the relay hides the later revisions of the talk's space from Ada's machine, a revision
/// Bob's device, an admin's, signs adds Carol to the channel, and Bob's device adds `carol`'s:
/// epoch 2. The relay holds the request for the channel's GroupInfo Ada's `skald recv` makes once
/// it entered epoch 2 and judged it unclean, as it does not admit Carol's device; the test kills
/// the run there, before it commits Carol's Remove. The relay then lets the revisions and the
/// GroupInfo requests through. Gives the number of the front's sends and uploads before the run.
pub fn killed_in_unclean_epoch(talk: &mut Talk, carol: &Played) -> (usize, usize) {
    let (cloud, at) = (&talk.world.cloud, talk.at());
    adds_to_channel(cloud, &talk.played[0].person, at, &carol.person);
    talk.played[0].welcomes(cloud, at, &carol.person);
    let (sends, uploads) = (talk.front.sent().len(), talk.front.uploads().len());
    let group_info = format!("GET /spaces/{}/channels/{}/group-info", at.0, at.1);
    // Held until the next run asks too.
    cloud.relay.hold(&group_info, 2);
    let started = talk.ada.machine.start(&[RECV]);
    kill_once(started, "a GroupInfo request after epoch 2", || {
        cloud.relay.held() > 0
    });
    assert!(
        !commit_sent(&talk.front, at.1, sends),
        "Carol's device removed before the kill"
    );
    cloud.relay.rewrite(&revisions_request(at.0), str::to_owned);
    // Requests for the GroupInfo go on at once from now on.
    cloud.relay.hold(&group_info, 0);
    (sends, uploads)
}

/// The entries of `at`'s channel the server gives the session `token`, as their bytes.
#[track_caller]
pub fn entries(cloud: &Cloud, token: &str, at: (SpaceId, ChannelId)) -> Vec<Vec<u8>> {
    listed(cloud, token, at, "entries")
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
    (uploads_in(front, channel, from).into_iter())
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
