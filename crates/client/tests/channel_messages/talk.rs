//! What the tests of channel messages share: a channel of Ada's whose group holds the devices of
//! the people the tests play, the messages those devices write and read, and what the front saw
//! Ada's machine send and the CLI show.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_crypto::{MlsContent, MlsFraming, seal_message};
use skald_proto::{
    Address, Body, Cbor as _, ChannelId, Content, DeviceId, Envelope, Header, MessageId,
    RawEnvelope, Role, Scheme, SpaceId, Unknown,
};
use skald_server::Settings;

use crate::cloud::{Cloud, WAIT};
use crate::fixtures::{fronted_with, person_address, retrying_for_the_timeout, send, sent_after};
use crate::front::{Front, Sent};
use crate::kit::{Space, assert_members, create_space, received, with_channel};
use crate::machine::{Finished, Machine};
use crate::played::{Played, mls_envelope};
use crate::world::{ACME, Person, World, revisions_request};

/// The host names of the people's machines, their certificates' `name`.
pub const ADA_HOST: &str = "ada-workstation";
pub const BOB_HOST: &str = "bob-workstation";
pub const CAROL_HOST: &str = "carol-laptop";

/// What the CLI shows of a recipient device that got the device's own message, and says when no
/// device can receive a message.
pub const DELIVERED: &str = "delivered";
pub const NO_DEVICE: &str = "no device";

/// Ada, whose machine runs the CLI and created `acme`, the people whose devices the tests play,
/// Bob's and, if any, Carol's, and a channel of `acme` with all of them, which revision 3 adds
/// together with them, who join the default channel too. Each device is in both groups.
pub struct Talk {
    pub world: World,
    pub front: Front,
    pub ada: Person,
    /// Bob's device, then Carol's, if any.
    pub played: Vec<Played>,
    pub space: Space,
    pub channel: ChannelId,
}

impl Talk {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and the people the next ones sign up; Ada's
    /// machine retries for the tests' timeout if `retrying`.
    pub fn new(ids: &[u64], logins: &[&str], retrying: bool) -> Self {
        Self::in_space(ACME, ids, logins, retrying)
    }

    /// As [`Talk::new`], in a space named `name` in place of `acme`.
    pub fn in_space(name: &str, ids: &[u64], logins: &[&str], retrying: bool) -> Self {
        Self::with(name, ids, logins, retrying, (Role::Member, |_| {}))
    }

    /// As [`Talk::in_space`], revision 3 giving the people the tests play `role`, and the server's
    /// settings changed by `change`.
    pub fn with(
        name: &str,
        ids: &[u64],
        logins: &[&str],
        retrying: bool,
        (role, change): (Role, impl FnOnce(&mut Settings)),
    ) -> Self {
        let (world, front) = fronted_with(change);
        let ada = world.sign_up(ids[0], logins[0], ADA_HOST);
        let cloud = &world.cloud;
        let mut played: Vec<Played> = ids[1..]
            .iter()
            .zip(&logins[1..])
            .zip([BOB_HOST, CAROL_HOST])
            .map(|((id, login), host)| Played::new(cloud, world.sign_up(*id, login, host)))
            .collect();
        let space = create_space(cloud, &ada, name);
        let added: Vec<(&Person, Role)> = played.iter().map(|one| (&one.person, role)).collect();
        let channel = with_channel(cloud, space.id, &ada, &added);
        received(&ada.machine);
        let ada = if retrying {
            retrying_for_the_timeout(ada)
        } else {
            ada
        };
        let mut devices = vec![ada.device];
        devices.extend(played.iter().map(Played::device));
        for one in &mut played {
            one.take_in(cloud);
            for group in [space.default, channel] {
                assert_members(&one.members(group), &devices);
            }
        }
        Talk {
            world,
            front,
            ada,
            played,
            space,
            channel,
        }
    }

    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// The channel's space and ID.
    pub fn at(&self) -> (SpaceId, ChannelId) {
        (self.space.id, self.channel)
    }

    /// Runs `skald send` of `text` to the channel on Ada's machine.
    pub fn send(&self, text: &str) -> Finished {
        send(&self.ada.machine, self.channel, text)
    }

    /// `person`'s person address in the channel's space.
    pub fn address(&self, person: &Person) -> String {
        person_address(&self.space.name, person)
    }

    /// Runs `skald recv` on Ada's machine after the proxy put one envelope into her catch-up; fails
    /// if it shows `text`.
    #[track_caller]
    pub fn ada_drops(&self, text: &str) -> Finished {
        let ran = received(&self.ada.machine);
        assert_eq!(self.front.injected(), 1, "not put into catch-up: {ran:?}");
        assert!(!ran.shown().contains(text), "{ran:?}");
        ran
    }

    /// Bob's device takes in its mailbox; fails unless the messages of the channel it read are
    /// Ada's `expected`, as [`assert_read`] says.
    #[track_caller]
    pub fn bob_reads_from_ada(&mut self, expected: &[(MessageId, u64, &str)]) {
        let from_ada = self.address(&self.ada);
        let bob = &mut self.played[0];
        bob.take_in(&self.world.cloud);
        assert_read(bob, self.channel, (self.ada.device, &from_ada), expected);
    }
}

/// From now on, the relay answers the first request for `space`'s revisions with no revisions in
/// the form the API gives them, as a request that fails once.
pub fn revisions_fail_once(cloud: &Cloud, space: SpaceId) {
    (cloud.relay).rewrite_once(&revisions_request(space), |_| "{}".to_owned());
}

/// A line of text a person's device says in a channel.
pub struct Line<'a> {
    /// `from`, in text form.
    pub from: String,
    pub counter: u64,
    /// The A2A `messageId`; `None`, the envelope's `msg_id`.
    pub message_id: Option<MessageId>,
    pub text: &'a str,
    /// `to`, in text form; `None`, none.
    pub to: Option<String>,
    /// The body; `None`, an A2A message of the text.
    pub body: Option<Body>,
}

impl<'a> Line<'a> {
    pub fn new(from: &str, counter: u64, text: &'a str) -> Self {
        Line {
            from: from.to_owned(),
            counter,
            message_id: None,
            text,
            to: None,
            body: None,
        }
    }

    /// The content of the line in the envelope `msg_id`.
    pub fn content(&self, msg_id: MessageId) -> Content {
        let message = json!({
            "role": "ROLE_USER",
            "messageId": self.message_id.unwrap_or(msg_id).to_string(),
            "parts": [{ "text": self.text }],
        });
        Content {
            body: Some(self.body.clone().unwrap_or(Body::A2a(message.to_string()))),
            counter: std::num::NonZeroU64::new(self.counter),
            from: self.from.parse::<Address>().expect("an address"),
            hops: 0,
            to: (self.to.as_ref()).map(|to| to.parse::<Address>().expect("an address")),
            reply_to: None,
            attachments: None,
            delegation: None,
            grant: None,
            unknown: Unknown::default(),
        }
    }
}

/// A new header of an envelope of `at`'s channel under `scheme`, to `recipients`, none for the
/// whole channel.
fn header(at: (SpaceId, ChannelId), scheme: Scheme, recipients: &[DeviceId]) -> Header {
    let mut header = mls_envelope(at, &[], recipients).envelope().header.clone();
    header.scheme = scheme;
    header
}

fn envelope(header: Header, payload: Vec<u8>) -> RawEnvelope {
    RawEnvelope::from_bytes(Envelope { header, payload }.to_cbor()).expect("an envelope")
}

/// The `mls` envelope of `at`'s channel in which `played` says `line`, encrypted in the group of
/// `group`: the channel's own, or another one's, as a compromised server could put it.
pub fn encrypted(
    played: &mut Played,
    at: (SpaceId, ChannelId),
    group: ChannelId,
    line: &Line,
) -> RawEnvelope {
    let header = header(at, Scheme::Mls, &[]);
    let content = line.content(header.msg_id);
    let payload = (played.mls)
        .encrypt(group, &content.to_cbor())
        .unwrap_or_else(|err| panic!("encrypt in the group of {group}: {err:?}"));
    envelope(header, payload)
}

/// The `hpke` envelope of `at`'s channel in which `sender`'s device says `line` to the device `to`,
/// sealed to its HPKE key `key`.
pub fn sealed(
    sender: &Person,
    at: (SpaceId, ChannelId),
    (to, key): (DeviceId, &[u8; 32]),
    line: &Line,
) -> RawEnvelope {
    let header = header(at, Scheme::Hpke, &[to]);
    let content = line.content(header.msg_id);
    let payload = seal_message(&header, &content, &sender.key, key).expect("a sealed payload");
    envelope(header, payload)
}

/// `played` sends `envelope`; fails unless the server stores it.
#[track_caller]
pub fn stored(cloud: &Cloud, played: &Played, envelope: &RawEnvelope) {
    played.connect(cloud).stored(envelope);
}

/// Whether `envelope` holds a channel message: an `mls` application message, or anything under
/// `hpke`.
pub fn is_message(envelope: &Envelope) -> bool {
    envelope.header.scheme == Scheme::Hpke
        || matches!(
            MlsFraming::read(&envelope.payload),
            Some(MlsFraming::Message {
                content: MlsContent::Application,
                ..
            })
        )
}

/// Whether `envelope` holds a Commit.
pub fn is_commit(envelope: &Envelope) -> bool {
    matches!(
        MlsFraming::read(&envelope.payload),
        Some(MlsFraming::Message {
            content: MlsContent::Commit,
            ..
        })
    )
}

/// The distinct envelopes of `channel` the front saw sent after its first `from` sends, in the
/// order each was first sent: a retry with the same bytes is no other one.
pub fn sent_in(front: &Front, channel: ChannelId, from: usize) -> Vec<Sent> {
    let mut distinct: Vec<Sent> = Vec::new();
    for sent in sent_after(front, from) {
        if sent.envelope.envelope().header.channel == channel
            && (distinct.iter()).all(|seen| seen.envelope.bytes() != sent.envelope.bytes())
        {
            distinct.push(sent);
        }
    }
    distinct
}

/// Of [`sent_in`], the envelopes of messages.
pub fn messages_sent(front: &Front, channel: ChannelId, from: usize) -> Vec<Sent> {
    (sent_in(front, channel, from).into_iter())
        .filter(|sent| is_message(sent.envelope.envelope()))
        .collect()
}

/// The `msg_id` of the one message envelope in `channel` the front saw sent after its first `from`
/// sends; fails unless there is one.
#[track_caller]
pub fn one_message_sent(front: &Front, channel: ChannelId, from: usize) -> MessageId {
    match messages_sent(front, channel, from).as_slice() {
        [one] => one.envelope.envelope().header.msg_id,
        other => panic!("not one message in {channel}: {other:?}"),
    }
}

/// Whether the front saw a Commit of `channel` sent after its first `from` sends.
pub fn commit_sent(front: &Front, channel: ChannelId, from: usize) -> bool {
    sent_after(front, from).iter().any(|sent| {
        let envelope = sent.envelope.envelope();
        envelope.header.channel == channel && is_commit(envelope)
    })
}

/// The A2A `messageId` and the text parts of the A2A message in `content`.
fn a2a(content: &Content) -> (String, Vec<String>) {
    let Some(Body::A2a(json)) = &content.body else {
        panic!("not an a2a body: {content:?}");
    };
    let message: Value = serde_json::from_str(json).expect("an A2A message in JSON");
    let texts = message["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["text"].as_str().map(str::to_owned))
        .collect();
    (
        message["messageId"].as_str().unwrap_or("").to_owned(),
        texts,
    )
}

/// Fails unless the application messages of `channel` that `played` decrypted are `expected`, in
/// order, each `(msg_id, counter, text)`: sent by `sender`'s device, with `from` `sender`'s
/// address, `hops` 0, no `to`, and an A2A message whose `messageId` is the `msg_id`.
#[track_caller]
pub fn assert_read(
    played: &Played,
    channel: ChannelId,
    (sender, from): (DeviceId, &str),
    expected: &[(MessageId, u64, &str)],
) {
    let read = played.read_in(channel);
    let ids: Vec<MessageId> = read.iter().map(|(msg_id, _)| *msg_id).collect();
    let wanted: Vec<MessageId> = expected.iter().map(|(msg_id, ..)| *msg_id).collect();
    assert_eq!(ids, wanted, "the messages {} read", played.device());
    for ((msg_id, decrypted), (_, counter, text)) in read.iter().zip(expected) {
        assert_eq!(decrypted.sender, sender, "the sending device of {msg_id}");
        let content = Content::from_cbor(&decrypted.plaintext)
            .unwrap_or_else(|err| panic!("{msg_id} holds no content: {err:?}"));
        assert_eq!(content.from.to_string(), from, "{content:?}");
        assert_eq!(
            content.counter.map(std::num::NonZeroU64::get),
            Some(*counter),
            "{content:?}"
        );
        assert_eq!((content.hops, &content.to), (0, &None), "{content:?}");
        let (message_id, texts) = a2a(&content);
        assert_eq!(message_id, msg_id.to_string(), "the A2A messageId");
        assert_eq!(texts, [*text], "the text of {msg_id}");
    }
}

/// Fails unless `shown` holds `text` once, and the channel's ID, `from` and the sending device's
/// certificate `name` of `(channel, from, device)`.
#[track_caller]
pub fn assert_shows_once(
    shown: &str,
    text: &str,
    (channel, from, device): (ChannelId, &str, &str),
) {
    assert_eq!(shown.matches(text).count(), 1, "{text:?} not once: {shown}");
    for part in [channel.to_string().as_str(), from, device] {
        assert!(shown.contains(part), "{text:?} without {part}: {shown}");
    }
}

/// The lines of `shown` that say `delivered` for the message `msg_id` to `device`.
pub fn delivered(shown: &str, msg_id: MessageId, device: DeviceId) -> usize {
    let (msg_id, device) = (msg_id.to_string(), device.to_string());
    shown
        .lines()
        .filter(|line| line.contains(&msg_id) && line.contains(&device))
        .filter(|line| line.to_lowercase().contains(DELIVERED))
        .count()
}

/// What runs of `skald recv` on `machine` showed together once `enough` holds of it; runs again
/// every half second for up to [`WAIT`]. Fails if a run fails.
pub fn received_until(machine: &Machine, enough: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + WAIT;
    let mut shown = received(machine).shown();
    while !enough(&shown) {
        assert!(Instant::now() < deadline, "not enough shown: {shown}");
        thread::sleep(Duration::from_millis(500));
        shown += &received(machine).shown();
    }
    shown
}
