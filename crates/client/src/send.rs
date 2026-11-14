//! `skald send`: the device sends a text to a channel of a space it follows; as an agent, only in
//! the space of its folder. It first accepts the space's latest revisions, then seals one `hpke`
//! envelope, with `report` true, to each admitted `client` device of the channel but itself
//! (`docs/protocol/membership.md`, "Admitted devices"), with the HPKE key of its certificate. The
//! content is from the person address or the agent's, with `hops` 0, a `counter` per recipient
//! device and an A2A message with the text (`docs/protocol/README.md`, "Content").
//!
//! The envelopes and their counters go into the outbox first. The device then sends every envelope
//! of the outbox, those an earlier run left first, over its connection as its device, and sends
//! the unanswered ones again with backoff and the same bytes until each gets `stored` or
//! `send-refused`, for as long as the request timeout (`docs/protocol/delivery.md`, "Send"). What
//! is left waits in the outbox for the next run.

use std::collections::BTreeMap;
use std::io;
use std::num::NonZeroU64;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use skald_crypto::seal_message;
use skald_proto::{
    Address, Body, Cbor as _, ChannelId, Content, DeviceId, DeviceKind, Envelope, Frame, Header,
    MessageId, Name, NonEmpty, RawEnvelope, Roster, Scheme, SendKey, Unknown, VersionRange,
};
use tungstenite::Message;
use tungstenite::protocol::CloseFrame;
use tungstenite::stream::MaybeTlsStream;

use crate::agents::{NO_SERVER_NAME, server_name};
use crate::connection::{WebSocket, printable};
use crate::devices::{self, text};
use crate::folders;
use crate::follow::{ATTEMPTS, Own, SpaceDevices};
use crate::secrets;
use crate::spaces::{self, Follower, Scope, Synced};
use crate::state::{Outgoing, State};

/// The first pause between attempts, and the longest.
const FIRST_PAUSE: Duration = Duration::from_millis(500);
const LONGEST_PAUSE: Duration = Duration::from_secs(8);

/// What the server answered to an envelope.
#[derive(Debug, Clone, PartialEq)]
enum Answer {
    Stored,
    Refused(String),
}

/// An envelope of the outbox, as it goes out.
struct Queued {
    outgoing: Outgoing,
    channel: ChannelId,
    /// Its `send` frame.
    frame: Vec<u8>,
}

impl Queued {
    fn key(&self) -> (MessageId, DeviceId) {
        (self.outgoing.msg_id, self.outgoing.recipient)
    }
}

/// What sending the outbox gave: the server's answers, and why the last attempt failed, if it did.
struct Delivered {
    answers: BTreeMap<(MessageId, DeviceId), Answer>,
    failed: Option<String>,
}

/// Sends `text` to the channel whose ID is `channel`, by the state in `dir`, its secrets in
/// `storage`, as `agent` if given; retries for `timeout`.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    channel: &str,
    text: &str,
    agent: Option<&Name>,
) -> Result<(), String> {
    let channel: ChannelId = channel
        .parse()
        .map_err(|_| format!("\"{}\" is not a channel ID", printable(channel)))?;
    let scope = match agent {
        Some(_) => Scope::Only(folders::working_space(dir)?.id),
        None => Scope::All,
    };
    let Synced {
        mut follower,
        roster,
        devices,
        ..
    } = spaces::of_channel(dir, storage, speaks, timeout, channel, scope)?;
    let recipients = admitted(&follower.own, follower.this, &roster, &devices, channel);
    if recipients.is_empty() {
        return Err(format!(
            "no device can receive the message: the channel {channel} has no admitted device with \
             a valid certificate but this one; nothing is sent"
        ));
    }
    let from = sender(&mut follower, &roster, agent)?;
    enqueue(&follower, &roster, channel, &recipients, &from, text)?;
    let queued = queued(&follower.state, scope)?;
    let delivered = deliver(&mut follower, &queued, timeout);
    report(&queued, &delivered, timeout)
}

/// The admitted `client` devices of `channel` in `roster`, the space's latest revision, but `this`
/// one, each with the HPKE key of its valid certificate among `devices`: those of the accounts of
/// the channel's members that their member's `excluded` does not list. The certificates of `own`'s
/// account are checked against the master key it pinned, any other's against the revision's.
fn admitted(
    own: &Own,
    this: DeviceId,
    roster: &Roster,
    devices: &SpaceDevices,
    channel: ChannelId,
) -> BTreeMap<DeviceId, [u8; 32]> {
    let body = roster.body();
    let mut admitted = BTreeMap::new();
    let Some(channel) = body.channels.iter().find(|it| it.id == channel) else {
        return admitted;
    };
    for entry in &channel.members {
        let Some(member) = body.member(entry.account) else {
            continue;
        };
        let master = if member.account == own.account {
            own.master
        } else {
            member.master
        };
        let excluded: Vec<DeviceId> = member
            .excluded
            .iter()
            .flat_map(|ids| ids.iter())
            .copied()
            .collect();
        for cert in devices::check(&master, &devices.ids, &devices.certs).linked {
            let cert = cert.body();
            if cert.account == member.account
                && cert.kind == DeviceKind::Client
                && cert.device != this
                && !excluded.contains(&cert.device)
            {
                admitted.insert(cert.device, cert.hpke);
            }
        }
    }
    admitted
}

/// The address the message is from: the person's, or `agent`'s, of the device's account in
/// `roster`'s space, on the server of the machine's session.
fn sender(
    follower: &mut Follower,
    roster: &Roster,
    agent: Option<&Name>,
) -> Result<Address, String> {
    let body = roster.body();
    let member = body.member(follower.own.account).ok_or_else(|| {
        format!(
            "this account is no member of the space {} in its last accepted revision; nothing is \
             sent",
            body.name.as_str()
        )
    })?;
    let server = server_name(&mut follower.connection)?
        .ok_or_else(|| format!("{NO_SERVER_NAME}, so a message has no sender; nothing is sent"))?;
    let (space, owner) = (body.name.clone(), member.handle.clone());
    Ok(match agent {
        Some(agent) => Address::new(space, owner, agent.clone(), server),
        None => Address::person(space, owner, server),
    })
}

/// Seals `message` from `from` to each of `recipients`, admitted devices of `channel` in `roster`,
/// with the next counter to it, and adds the envelopes to the outbox with their counters.
fn enqueue(
    follower: &Follower,
    roster: &Roster,
    channel: ChannelId,
    recipients: &BTreeMap<DeviceId, [u8; 32]>,
    from: &Address,
    message: &str,
) -> Result<(), String> {
    let space = roster.body().space;
    let keys = follower.keys()?;
    let msg_id = MessageId::generate();
    let a2a = serde_json::json!({
        "messageId": msg_id.to_string(),
        "role": "ROLE_USER",
        "parts": [{ "text": message }],
    })
    .to_string();
    let ids: Vec<DeviceId> = recipients.keys().copied().collect();
    for _ in 0..ATTEMPTS {
        let counters = follower.state.counters(channel, &ids).map_err(text)?;
        let mut outgoing = Vec::new();
        for ((&device, hpke), last) in recipients.iter().zip(counters) {
            let counter = last
                .checked_add(1)
                .and_then(NonZeroU64::new)
                .ok_or_else(|| format!("the counter to device {device} has no next"))?;
            let header = Header {
                v: follower.connection.version(),
                space,
                channel,
                msg_id,
                scheme: Scheme::Hpke,
                recipients: Some(NonEmpty::new(device)),
                ttl: None,
                unknown: Unknown::default(),
            };
            let content = Content {
                from: from.clone(),
                to: None,
                counter: Some(counter),
                hops: 0,
                reply_to: None,
                body: Some(Body::A2a(a2a.clone())),
                delegation: None,
                grant: None,
                attachments: None,
                unknown: Unknown::default(),
            };
            let payload = seal_message(&header, &content, keys.signing(), hpke)
                .map_err(|err| format!("the message cannot be sealed to device {device}: {err}"))?;
            let envelope = Outgoing {
                msg_id,
                recipient: device,
                space,
                envelope: Envelope { header, payload }.to_cbor(),
            };
            outgoing.push((envelope, counter.get()));
        }
        if follower.state.enqueue(channel, &outgoing).map_err(text)? {
            return Ok(());
        }
    }
    Err("other skald runs kept sending to the channel meanwhile; run the command again".to_owned())
}

/// The envelopes of the outbox of `state` within `scope`, in the order they go out.
fn queued(state: &State, scope: Scope) -> Result<Vec<Queued>, String> {
    let outbox = state.outbox().map_err(text)?;
    outbox
        .into_iter()
        .filter(|outgoing| !scope.excludes(outgoing.space))
        .map(|outgoing| {
            let envelope = RawEnvelope::from_bytes(outgoing.envelope.clone()).map_err(|err| {
                format!("the local state is malformed: an envelope in the outbox: {err}")
            })?;
            let channel = envelope.envelope().header.channel;
            let frame = Frame::Send {
                envelope,
                report: true,
            }
            .to_cbor();
            Ok(Queued {
                outgoing,
                channel,
                frame,
            })
        })
        .collect()
}

/// Sends each envelope of `queued` that the server has not answered, in order, over a connection
/// as the device, again with backoff while some are unanswered, for `timeout`; takes each answered
/// one out of the outbox.
fn deliver(follower: &mut Follower, queued: &[Queued], timeout: Duration) -> Delivered {
    let deadline = Instant::now() + timeout;
    let mut delivered = Delivered {
        answers: BTreeMap::new(),
        failed: None,
    };
    let keys = match follower.keys() {
        Ok(keys) => keys,
        Err(err) => {
            delivered.failed = Some(err);
            return delivered;
        }
    };
    let mut socket: Option<WebSocket> = None;
    let mut pause = FIRST_PAUSE;
    loop {
        let unanswered: Vec<&Queued> = queued
            .iter()
            .filter(|queued| !delivered.answers.contains_key(&queued.key()))
            .collect();
        let now = Instant::now();
        if unanswered.is_empty() || now >= deadline {
            return delivered;
        }
        let until = (now + pause).min(deadline);
        pause = (pause * 2).min(LONGEST_PAUSE);
        let opened = match socket.take() {
            Some(socket) => Ok(socket),
            None => follower
                .connection
                .open_websocket_as("/ws", &follower.token, keys.id(), keys.signing())
                .map_err(text),
        };
        let attempt = opened.and_then(|mut opened| {
            for queued in &unanswered {
                opened
                    .send(Message::Binary(queued.frame.clone().into()))
                    .map_err(broke)?;
            }
            answers(&mut opened, follower, queued, &mut delivered.answers, until)?;
            Ok(opened)
        });
        match attempt {
            Ok(opened) => {
                socket = Some(opened);
                delivered.failed = None;
            }
            Err(err) => {
                delivered.failed = Some(err);
                thread::sleep(until.saturating_duration_since(Instant::now()));
            }
        }
    }
}

/// Reads the server's answers to `queued` on `socket` into `answers` until each one is answered or
/// `until`, taking each answered envelope out of the outbox; fails if the connection ends.
fn answers(
    socket: &mut WebSocket,
    follower: &Follower,
    queued: &[Queued],
    answers: &mut BTreeMap<(MessageId, DeviceId), Answer>,
    until: Instant,
) -> Result<(), String> {
    while queued
        .iter()
        .any(|queued| !answers.contains_key(&queued.key()))
    {
        let wait = until.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            return Ok(());
        }
        set_read_timeout(socket, wait).map_err(|err| broke(err.into()))?;
        let (send_key, answer) = match socket.read() {
            Ok(Message::Binary(bytes)) => match Frame::from_cbor(&bytes) {
                Ok(Frame::Stored(stored)) => (stored.send_key, Answer::Stored),
                Ok(Frame::SendRefused { send_key, problem }) => {
                    (send_key, Answer::Refused(problem))
                }
                // A frame it does not know or that does not decode, ignored.
                _ => continue,
            },
            Ok(Message::Close(frame)) => return Err(closed(frame.as_ref())),
            Ok(_) => continue,
            Err(tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(());
            }
            Err(err) => return Err(broke(err)),
        };
        let SendKey {
            msg_id,
            recipient: Some(recipient),
        } = send_key
        else {
            continue;
        };
        let key = (msg_id, recipient);
        if answers.contains_key(&key) || !queued.iter().any(|queued| queued.key() == key) {
            continue;
        }
        if let Err(err) = follower.state.remove_outgoing(msg_id, recipient) {
            eprintln!(
                "skald: warning: the outbox keeps the message {msg_id} to device {recipient}, \
                 which the server answered, so a later run sends it again: {err}"
            );
        }
        answers.insert(key, answer);
    }
    Ok(())
}

/// Sets how long a read of `socket` waits, at least a millisecond.
fn set_read_timeout(socket: &mut WebSocket, wait: Duration) -> io::Result<()> {
    let wait = Some(wait.max(Duration::from_millis(1)));
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(wait),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(wait),
        _ => Ok(()),
    }
}

/// Why a connection broke, by `err`.
fn broke(err: tungstenite::Error) -> String {
    format!("the connection to the server broke: {err}")
}

/// Why the server closed a connection, by its close `frame`.
fn closed(frame: Option<&CloseFrame>) -> String {
    match frame.map(|frame| frame.reason.as_str()) {
        Some(reason) if !reason.is_empty() => {
            format!("the server closed the connection: {}", printable(reason))
        }
        _ => "the server closed the connection".to_owned(),
    }
}

/// Says, for each message of `queued`, its ID and, per recipient device, whether the server stored
/// its envelope, refused it or has not answered it in `delivered`; fails unless every one is
/// stored.
fn report(queued: &[Queued], delivered: &Delivered, timeout: Duration) -> Result<(), String> {
    let mut messages: Vec<(MessageId, ChannelId, Vec<&Queued>)> = Vec::new();
    for queued in queued {
        match messages
            .iter_mut()
            .find(|(id, ..)| *id == queued.outgoing.msg_id)
        {
            Some((.., envelopes)) => envelopes.push(queued),
            None => messages.push((queued.outgoing.msg_id, queued.channel, vec![queued])),
        }
    }
    let (mut left, mut refused) = (Vec::new(), Vec::new());
    for (id, channel, envelopes) in &messages {
        println!("The message {id} to the channel {channel}:");
        let mut waiting = Vec::new();
        for queued in envelopes {
            let device = queued.outgoing.recipient;
            match delivered.answers.get(&queued.key()) {
                Some(Answer::Stored) => println!("  {device}  stored"),
                Some(Answer::Refused(problem)) => {
                    let problem = printable(problem);
                    println!("  {device}  refused: {problem}");
                    refused.push(format!("the message {id} to device {device}: {problem}"));
                }
                None => {
                    println!("  {device}  no answer yet: it waits in the outbox");
                    waiting.push(device.to_string());
                }
            }
        }
        if !waiting.is_empty() {
            left.push(format!("the message {id} to {}", waiting.join(", ")));
        }
    }
    if !left.is_empty() {
        let why = delivered
            .failed
            .as_ref()
            .map_or_else(String::new, |err| format!(" ({err})"));
        return Err(format!(
            "the server has not stored {} within {} s{why}; the outbox keeps it, and the next \
             skald send sends it first",
            left.join("; "),
            timeout.as_secs()
        ));
    }
    if !refused.is_empty() {
        return Err(format!(
            "the server refused {}; it is not sent again",
            refused.join("; ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use skald_crypto::{HpkeKey, SigningKey, issue};
    use skald_proto::{AccountId, Cert, CertBody, ChannelMember, HpkePayload, SpaceId};

    use super::*;
    use crate::spaces::tests::{first, member};
    use crate::test_dir::TestDir;

    /// An account and its master key.
    struct Account {
        id: AccountId,
        master: SigningKey,
    }

    impl Account {
        fn new() -> Self {
            Account {
                id: AccountId::generate(),
                master: SigningKey::generate(),
            }
        }

        /// A device of the account of `kind`, its certificate issued with `issuer`, the key of
        /// the device `by` or else the master key: the certificate, its HPKE key and its signing
        /// key.
        fn device(
            &self,
            kind: DeviceKind,
            issuer: &SigningKey,
            by: Option<DeviceId>,
        ) -> (Cert, [u8; 32], SigningKey) {
            let signing = SigningKey::generate();
            let hpke = HpkeKey::generate().public_key();
            let body = CertBody {
                account: self.id,
                device: DeviceId::from_signing_key(&signing.public_key()),
                kind,
                signing: signing.public_key(),
                hpke,
                issuer: by,
                issued: 1_800_000_000,
                ssh: None,
                name: "laptop".to_owned(),
                unknown: Unknown::default(),
            };
            let cert = issue(&body, issuer, &signing).expect("a certificate");
            (cert, hpke, signing)
        }

        fn client(&self) -> (Cert, [u8; 32], SigningKey) {
            self.device(DeviceKind::Client, &self.master, None)
        }
    }

    /// Admitted: the `client` devices with a valid certificate of the channel's members' accounts,
    /// this one, a browser, one the member's `excluded` lists, one with a certificate by another
    /// key and one of a member outside the channel aside.
    #[test]
    fn admitted_devices_are_the_channels_valid_client_devices_but_this_one() {
        let (ada, bob, carol) = (Account::new(), Account::new(), Account::new());
        let id = |cert: &Cert| cert.body().device;
        let (this, _, this_key) = ada.client();
        let (laptop, laptop_hpke, _) = ada.client();
        let (browser, ..) = ada.device(DeviceKind::Browser, &this_key, Some(id(&this)));
        let (phone, phone_hpke, _) = bob.client();
        let (barred, ..) = bob.client();
        let (forged, ..) = bob.device(DeviceKind::Client, &SigningKey::generate(), None);
        let (outside, ..) = carol.client();
        let certs = vec![this, laptop, browser, phone, barred, forged, outside];

        let mut body = first(ada.id).body().clone();
        let mut bobs = member(bob.id, "bob", bob.master.public_key());
        bobs.excluded = Some(NonEmpty::new(id(&certs[4])));
        body.members.push(bobs);
        body.members
            .push(member(carol.id, "carol", carol.master.public_key()));
        let channel = ChannelId::generate();
        body.channels = vec![skald_proto::Channel {
            id: channel,
            members: [ada.id, bob.id]
                .map(|account| ChannelMember {
                    account,
                    history: false,
                })
                .to_vec(),
            services: Vec::new(),
            unknown: Unknown::default(),
        }];
        let roster = skald_crypto::sign_roster(&body, &SigningKey::generate()).unwrap();
        let own = Own {
            account: ada.id,
            master: ada.master.public_key(),
        };
        let devices = SpaceDevices {
            ids: certs.iter().map(id).collect(),
            certs: certs.clone(),
        };

        let given = admitted(&own, id(&certs[0]), &roster, &devices, channel);

        assert_eq!(
            given,
            BTreeMap::from([(id(&certs[1]), laptop_hpke), (id(&certs[3]), phone_hpke)])
        );
        let elsewhere = admitted(
            &own,
            id(&certs[0]),
            &roster,
            &devices,
            ChannelId::generate(),
        );
        assert!(elsewhere.is_empty(), "{elsewhere:?}");
    }

    /// An agent's run sends only the envelopes of its space, in the order they went into the
    /// outbox; a person's sends every one.
    #[test]
    fn queued_envelopes_are_those_of_the_scope_in_order() {
        let dir = TestDir::new("send-queued");
        let state = State::open(&dir).unwrap();
        let (own, other) = (SpaceId::generate(), SpaceId::generate());
        let recipient = DeviceId::from_signing_key(&[1; 32]);
        let mut messages = Vec::new();
        for space in [own, other, own] {
            let channel = ChannelId::generate();
            let msg_id = MessageId::generate();
            let header = Header {
                v: NonZeroU64::MIN,
                space,
                channel,
                msg_id,
                scheme: Scheme::Hpke,
                recipients: Some(NonEmpty::new(recipient)),
                ttl: None,
                unknown: Unknown::default(),
            };
            let payload = HpkePayload {
                enc: [7; 32],
                ct: vec![1, 2, 3],
            }
            .to_cbor();
            let outgoing = Outgoing {
                msg_id,
                recipient,
                space,
                envelope: Envelope { header, payload }.to_cbor(),
            };
            assert!(state.enqueue(channel, &[(outgoing, 1)]).unwrap());
            messages.push((msg_id, channel));
        }
        let ids = |scope| -> Vec<(MessageId, ChannelId)> {
            queued(&state, scope)
                .unwrap()
                .iter()
                .map(|queued| (queued.outgoing.msg_id, queued.channel))
                .collect()
        };

        assert_eq!(ids(Scope::All), messages);
        assert_eq!(ids(Scope::Only(own)), [messages[0], messages[2]]);
    }
}
