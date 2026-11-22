//! Content from a `service` device as a recipient's CLI judges it (`docs/protocol/README.md`,
//! "Envelope", "Content" and "Service grant"; `docs/protocol/membership.md`, "Roster";
//! `docs/protocol/delivery.md`, "Mailbox"):
//! - its `from` is the device's `space/owner/agent` entry in the latest revision the client
//!   accepted, and the device's certificate is valid on a member's account; a client that finds no
//!   entry for the device first fetches and accepts the space's latest revisions;
//! - its grant names the device, lists the channel, is signed by an admin's device in that revision
//!   and has not expired;
//! - under `hpke` it may come without `counter`, and is then taken once per sending device and
//!   `msg_id`;
//! - content with a grant from any other kind of device is refused.
//!
//! A refused message gets the warning other refused messages get; an accepted one is shown as other
//! accounts' messages are.
//!
//! As the tests of receiving (`receiving/main.rs`): the CLI runs on Ada's machine, which created
//! `acme`; Bob's device is played by the tests with an MLS state of their own (`channel_groups/
//! played.rs`). The service agent `ci/deploy-bot` is a `service` device the test writes to the
//! server's database on Ada's account, its certificate issued by Ada's device, as the tests of
//! linked browsers write theirs, until the CLI creates service agents; a revision Ada's device
//! signs lists it. Its envelopes are sealed by the test with its keys and put into Ada's catch-up by
//! the front, or sent through Bob's device's connection, as a service device cannot connect yet.
//!
//! The CLI, as these tests use it: `skald recv` and `skald recv --wait`, which show each message
//! with its text once, the channel's ID, `from` and the sending device's certificate `name`, and for
//! each message they drop for failing a check a warning line naming its `msg_id` and the sending
//! device, as for other dropped messages, never the dropped text.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod accepted;
mod refetches;
mod refused;
mod revisions;
mod waiting;

// The modules the tests of the channel history share (`history_common/mod.rs`): the server, its
// relay and front, people in spaces, played devices, revocations and the talk of channel messages.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

use std::num::NonZeroU64;

use skald_crypto::{SigningKey, seal_message};
use skald_proto::{
    Cbor as _, ChannelId, Content, DeviceId, DeviceKind, Envelope, Grant, GrantBody, MessageId,
    NonEmpty, RawEnvelope, Roster, Scheme, ServiceAgent, SpaceId, Unknown,
};

use crate::cloud::Cloud;
use crate::fixtures::{Issuer, Keyed, add_device_of_kind, cert_of};
use crate::kit::{latest, warnings_naming};
use crate::machine::Finished;
use crate::played::mls_envelope;
use crate::talk::{Line, Talk, assert_shows_once};
use crate::world::{HOST, Person, next_with, now, store};

/// The service agent's owner and agent, and its device's certificate `name`.
pub const OWNER: &str = "ci";
pub const AGENT: &str = "deploy-bot";
pub const BOT_HOST: &str = "deploy-bot-runner";

/// How long the grants the tests sign last, in seconds: a day.
pub const GRANTED: u64 = 86_400;

/// A `service` device written to the server's database on `owner`'s account, its certificate
/// issued by `owner`'s device, named `name`.
pub fn service_device(cloud: &Cloud, owner: &Person, name: &str) -> Keyed {
    add_device_of_kind(
        cloud,
        (owner.account, DeviceKind::Service),
        name,
        &Issuer::Device(owner),
    )
}

/// The revision after the latest of `space`, signed by `signer`'s device, that lists `device` as
/// the service agent `owner/agent`; the server stores it.
pub fn list_agent(
    cloud: &Cloud,
    space: SpaceId,
    signer: &Person,
    (device, owner, agent): (DeviceId, &str, &str),
) -> Roster {
    let previous = latest(cloud, &signer.token, space);
    let next = next_with(&previous, (signer.device, &signer.key), |body| {
        body.service_agents.push(ServiceAgent {
            device,
            owner: owner.parse().expect("an owner"),
            agent: agent.parse().expect("an agent name"),
            unknown: Unknown::default(),
        });
    });
    store(cloud, &signer.token, &next);
    next
}

/// The grant `signer`, a device and its signing key, signs for the device `service` to send in
/// `channels` until `expires`, over `["skald grant", body]`.
pub fn grant(
    (signer, key): (DeviceId, &SigningKey),
    service: DeviceId,
    channels: &[ChannelId],
    expires: u64,
) -> Grant {
    let body = GrantBody {
        service,
        channels: NonEmpty::from_vec(channels.to_vec()).expect("a channel"),
        signer,
        expires,
        unknown: Unknown::default(),
    };
    let signature = key.sign("skald grant", &[&body.to_cbor()]);
    Grant { body, signature }
}

/// Content a test seals as a sending device would, or as a compromised server or member could.
#[derive(Clone)]
pub struct Note<'a> {
    pub msg_id: MessageId,
    /// `from`, in text form.
    pub from: String,
    pub counter: Option<u64>,
    pub grant: Option<Grant>,
    pub text: &'a str,
}

impl<'a> Note<'a> {
    /// A note of `text` from `from` with `grant`, without `counter`, and a new `msg_id`.
    pub fn new(from: &str, grant: Option<Grant>, text: &'a str) -> Self {
        Note {
            msg_id: MessageId::generate(),
            from: from.to_owned(),
            counter: None,
            grant,
            text,
        }
    }

    /// The note's content: its text as an A2A message whose `messageId` is `msg_id`.
    pub fn content(&self) -> Content {
        let mut content = Line::new(&self.from, 0, self.text).content(self.msg_id);
        content.counter = self
            .counter
            .map(|counter| NonZeroU64::new(counter).expect("above 0"));
        content.grant = self.grant.clone();
        content
    }

    /// The `hpke` envelope of the note in `at`'s channel, signed by `sender`'s key and sealed to
    /// the device `to`'s HPKE key `key`: a new seal on each call.
    pub fn sealed(
        &self,
        at: (SpaceId, ChannelId),
        sender: &SigningKey,
        (to, key): (DeviceId, &[u8; 32]),
    ) -> RawEnvelope {
        let mut header = mls_envelope(at, &[], &[to]).envelope().header.clone();
        header.scheme = Scheme::Hpke;
        header.msg_id = self.msg_id;
        let payload = seal_message(&header, &self.content(), sender, key).expect("a sealed note");
        RawEnvelope::from_bytes(Envelope { header, payload }.to_cbor()).expect("an envelope")
    }
}

/// The talk of Ada and Bob in `acme`, and the service agent `ci/deploy-bot`: a `service` device on
/// Ada's account that a revision Ada's device signs lists.
pub struct Bot {
    pub talk: Talk,
    pub device: Keyed,
    /// Its address, `acme/ci/deploy-bot@<server>`.
    pub address: String,
    /// The HPKE key of Ada's device, which the tests seal to.
    pub ada_key: [u8; 32],
}

impl Bot {
    /// The talk of Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones, Bob an admin
    /// if `role` says so, and its service agent, which the server's latest revision lists. Ada's
    /// machine has accepted that revision if `accepted`.
    pub fn new(ids: [u64; 2], logins: [&str; 2], bob: skald_proto::Role, accepted: bool) -> Self {
        let talk = Talk::with(crate::world::ACME, &ids, &logins, false, (bob, |_| {}));
        let bot = Self::beside(talk);
        bot.list();
        if accepted {
            crate::kit::received(&bot.talk.ada.machine);
        }
        bot
    }

    /// The service agent's device in `talk`, which no revision lists yet.
    pub fn beside(talk: Talk) -> Self {
        let device = service_device(talk.cloud(), &talk.ada, BOT_HOST);
        let address = format!("{}/{OWNER}/{AGENT}@{HOST}", talk.space.name);
        let ada_key = cert_of(talk.cloud(), &talk.ada).body().hpke;
        Bot {
            talk,
            device,
            address,
            ada_key,
        }
    }

    /// A revision Ada's device signs lists the service agent; the server stores it.
    pub fn list(&self) -> Roster {
        let at = (self.device.id, OWNER, AGENT);
        list_agent(self.talk.cloud(), self.talk.space.id, &self.talk.ada, at)
    }

    pub fn cloud(&self) -> &Cloud {
        self.talk.cloud()
    }

    /// The grant Ada's device signs for the service agent to send in the talk's channel for a day.
    pub fn granted(&self) -> Grant {
        let ada = &self.talk.ada;
        let channels = [self.talk.channel];
        grant(
            (ada.device, &ada.key),
            self.device.id,
            &channels,
            now() + GRANTED,
        )
    }

    /// A note of `text` from the service agent with [`Bot::granted`], without `counter`.
    pub fn note<'a>(&self, text: &'a str) -> Note<'a> {
        Note::new(&self.address, Some(self.granted()), text)
    }

    /// `note` in the talk's channel, sealed by the device `sender` to Ada's device.
    pub fn sealed_by(&self, note: &Note, sender: &SigningKey) -> RawEnvelope {
        let to = (self.talk.ada.device, &self.ada_key);
        note.sealed(self.talk.at(), sender, to)
    }

    /// `note` in the talk's channel, sealed by the service agent's device to Ada's device.
    pub fn sealed(&self, note: &Note) -> RawEnvelope {
        self.sealed_by(note, &self.device.signing)
    }

    /// Runs `skald recv` on Ada's machine after the front put `envelopes` into her catch-up; fails
    /// unless it succeeds and the front put them all.
    #[track_caller]
    pub fn ada_receives(&self, envelopes: Vec<RawEnvelope>) -> Finished {
        let count = envelopes.len() + self.talk.front.injected();
        self.talk.front.inject(self.talk.ada.device, envelopes);
        let finished = crate::kit::received(&self.talk.ada.machine);
        assert_eq!(
            self.talk.front.injected(),
            count,
            "not put into catch-up: {finished:?}"
        );
        finished
    }

    /// Fails unless `finished` shows `text` once, from the service agent's address and device.
    #[track_caller]
    pub fn assert_shown(&self, finished: &Finished, text: &str) {
        let from = (self.talk.channel, self.address.as_str(), BOT_HOST);
        assert_shows_once(&finished.shown(), text, from);
    }

    /// Fails if `finished` shows the text of `note`, or unless a warning names `note`'s `msg_id`
    /// and `device`, its sending device.
    #[track_caller]
    pub fn assert_dropped(&self, finished: &Finished, note: &Note, device: DeviceId) {
        let shown = finished.shown();
        assert!(
            !shown.contains(note.text),
            "{:?} is shown: {shown}",
            note.text
        );
        let msg_id = note.msg_id.to_string();
        let warned = warnings_naming(finished, device);
        assert!(
            warned.iter().any(|line| line.contains(&msg_id)),
            "no warning names {msg_id} from {device}: {shown}"
        );
    }
}
