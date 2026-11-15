//! What the tests of gap detection share: the messages Bob's device says in a channel of the talk,
//! sent or, as a server that drops them would have it, never sent, and what the CLI shows of
//! missing messages.

use skald_proto::{ChannelId, DeviceId, MessageId, RawEnvelope};

use crate::talk::{Line, Talk, encrypted, stored};
use crate::world::WARNING;

/// What the CLI says of missing messages.
pub const MISSING: &str = "missing";

/// The `mls` envelope in which Bob's device says `text` with `counter` in `channel` of the talk's
/// space, encrypted in the channel's group; not sent.
pub fn bob_says_in(talk: &mut Talk, channel: ChannelId, counter: u64, text: &str) -> RawEnvelope {
    let at = (talk.space.id, channel);
    let from = talk.address(&talk.played[0].person);
    encrypted(
        &mut talk.played[0],
        at,
        channel,
        &Line::new(&from, counter, text),
    )
}

/// As [`bob_says_in`], in the talk's channel.
pub fn bob_says(talk: &mut Talk, counter: u64, text: &str) -> RawEnvelope {
    let channel = talk.channel;
    bob_says_in(talk, channel, counter, text)
}

/// Bob's device sends `envelope`; fails unless the server stores it. Gives its `msg_id`.
#[track_caller]
pub fn sends(talk: &Talk, envelope: &RawEnvelope) -> MessageId {
    stored(talk.cloud(), &talk.played[0], envelope);
    msg_id_of(envelope)
}

/// As [`bob_says`], and Bob's device sends it; gives its `msg_id`.
#[track_caller]
pub fn bob_sends(talk: &mut Talk, counter: u64, text: &str) -> MessageId {
    let envelope = bob_says(talk, counter, text);
    sends(talk, &envelope)
}

/// Bob's device in the talk: his address, the channel, and his device.
pub fn bob_in(talk: &Talk) -> (String, ChannelId, DeviceId) {
    let bob = &talk.played[0];
    (talk.address(&bob.person), talk.channel, bob.device())
}

/// Bob's device sends `says[0]` and `says[2]` to the talk's channel with the counters 1 and 3;
/// `says[1]`, with the counter 2, never reaches the server. Gives the `msg_id` of the first and the
/// third.
#[track_caller]
pub fn second_dropped(talk: &mut Talk, says: [&str; 3]) -> (MessageId, MessageId) {
    let first = bob_sends(talk, 1, says[0]);
    bob_says(talk, 2, says[1]);
    (first, bob_sends(talk, 3, says[2]))
}

pub fn msg_id_of(envelope: &RawEnvelope) -> MessageId {
    envelope.envelope().header.msg_id
}

/// The warning lines of `shown`, in lowercase, that report missing messages.
pub fn reports(shown: &str) -> Vec<String> {
    shown
        .lines()
        .map(str::to_lowercase)
        .filter(|line| line.contains(WARNING) && line.contains(MISSING))
        .collect()
}

/// The reports of `shown` of messages missing in `channel` from `device`.
pub fn reports_of(shown: &str, channel: ChannelId, device: DeviceId) -> Vec<String> {
    let (channel, device) = (channel.to_string(), device.to_string());
    reports(shown)
        .into_iter()
        .filter(|line| line.contains(&channel) && line.contains(&device))
        .collect()
}

/// Fails unless `shown` reports messages missing in `channel` from `device` once, and nothing else
/// missing.
#[track_caller]
pub fn assert_reported_once(shown: &str, channel: ChannelId, device: DeviceId) {
    assert_eq!(
        reports_of(shown, channel, device).len(),
        1,
        "not one report of messages missing in {channel} from {device}: {shown}"
    );
    assert_eq!(reports(shown).len(), 1, "other reports: {shown}");
}

/// Fails if `shown` reports a missing message.
#[track_caller]
pub fn assert_none_reported(shown: &str) {
    let reported = reports(shown);
    assert!(
        reported.is_empty(),
        "missing messages reported: {reported:?}"
    );
}

/// Whether `shown` marks the message `msg_id` as following missing ones: a line holding its ID that
/// is no warning says `missing`. Fails unless such a line shows it.
#[track_caller]
pub fn marked(shown: &str, msg_id: MessageId) -> bool {
    let id = msg_id.to_string().to_lowercase();
    let lines: Vec<String> = shown
        .lines()
        .map(str::to_lowercase)
        .filter(|line| !line.contains(WARNING) && line.contains(&id))
        .collect();
    assert!(!lines.is_empty(), "{msg_id} is not shown: {shown}");
    lines.iter().any(|line| line.contains(MISSING))
}
