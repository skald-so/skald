//! What the tests share: the talk of Ada and Bob with the service agent `ci/deploy-bot` that Ada's
//! CLI creates, the agent's mailbox as the test client reads it, its tokens in the CLI, and the
//! copies the front saw Ada's machine send and the server store.

use serde_json::Value;
use skald_proto::{
    Body, Cert, ChannelId, Content, DeviceId, Frame, RawEnvelope, Role, Scheme, SpaceId,
};

use crate::agents::{
    BOT, CHANNEL, CREATE_SERVICE_TOKEN, REVOKE_SERVICE_TOKEN, Secrets, create_agent, id_of,
    own_record, secrets_of, token_of,
};
use crate::cloud::Cloud;
use crate::fixtures::{cert_of, send};
use crate::machine::Finished;
use crate::reader::{Copy, Reader};
use crate::talk::Talk;
use crate::world::{ACME, HOST, run};

/// Another service agent of Ada's account.
pub const OTHER_BOT: &str = "ci/review-bot";

/// Where the front and the relay were: how many `send` frames the front had seen and how many
/// answers of the server it had passed on.
#[derive(Debug, Clone, Copy)]
pub struct Mark {
    sent: usize,
    answered: usize,
}

/// The talk of Ada and Bob in `acme` and the service agent `ci/deploy-bot` that Ada's CLI created,
/// with the key secret and the token it printed.
pub struct Scene {
    pub talk: Talk,
    pub secrets: Secrets,
    pub reader: Reader,
    /// The certificate of Ada's device, which signs the copies.
    pub ada_cert: Cert,
    /// The agent's address, `acme/ci/deploy-bot@<server>`.
    pub address: String,
}

impl Scene {
    /// The talk of Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones, with
    /// `role`; Ada's CLI creates `ci/deploy-bot` with a grant for the channels `channels` picks and
    /// the options `extra`, which must succeed.
    pub fn with(
        ids: [u64; 2],
        logins: [&str; 2],
        role: Role,
        channels: impl FnOnce(&Talk) -> Vec<ChannelId>,
        extra: &[&str],
    ) -> Self {
        let talk = Talk::with(ACME, &ids, &logins, false, (role, |_| {}));
        let channels = channels(&talk);
        let created = create_agent(&talk.ada.machine, ACME, BOT, &channels, extra);
        let secrets = secrets_of(&created);
        let reader = Reader::of(&secrets.key);
        let ada_cert = cert_of(talk.cloud(), &talk.ada);
        let address = format!("{}/{BOT}@{HOST}", talk.space.name);
        Scene {
            talk,
            secrets,
            reader,
            ada_cert,
            address,
        }
    }

    /// As [`Scene::with`], Bob a member, the grant for the talk's channel, the token sending and
    /// receiving.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        Self::with(ids, logins, Role::Member, |talk| vec![talk.channel], &[])
    }

    pub fn cloud(&self) -> &Cloud {
        self.talk.cloud()
    }

    pub fn space(&self) -> SpaceId {
        self.talk.space.id
    }

    /// The service agent's device.
    pub fn bot(&self) -> DeviceId {
        self.reader.device
    }

    /// Runs `skald send --all --channel <channel> <text>` on Ada's machine; fails unless it
    /// succeeds.
    #[track_caller]
    pub fn send_all(&self, channel: ChannelId, text: &str) -> Finished {
        let finished = send(&self.talk.ada.machine, channel, text);
        assert!(finished.success, "{finished:?}");
        finished
    }

    /// Runs `skald send --to <to> --channel <channel> <text>` on Ada's machine; fails unless it
    /// succeeds.
    #[track_caller]
    pub fn send_to(&self, to: &str, channel: ChannelId, text: &str) -> Finished {
        let channel = channel.to_string();
        let args = ["send", "--to", to, "--channel", channel.as_str(), text];
        let finished = run(&self.talk.ada.machine, &args);
        assert!(finished.success, "{finished:?}");
        finished
    }

    /// The copies the agent takes in from its mailbox with `token`, opened, each signed by Ada's
    /// device; it acks them.
    #[track_caller]
    pub fn copies_with(&mut self, token: &str) -> Vec<Copy> {
        let cloud = &self.talk.world.cloud;
        self.reader.copies(cloud, token, &self.ada_cert)
    }

    /// As [`Scene::copies_with`], with the token the creation printed.
    #[track_caller]
    pub fn copies(&mut self) -> Vec<Copy> {
        let token = self.secrets.token.clone();
        self.copies_with(&token)
    }

    /// A new token of `ci/deploy-bot` in Ada's CLI, `skald create-service-token`, with a grant for
    /// `channels` and the options `extra`; fails unless it succeeds.
    #[track_caller]
    pub fn new_token(&self, channels: &[ChannelId], extra: &[&str]) -> String {
        let channels: Vec<String> = channels.iter().map(ToString::to_string).collect();
        let mut args = vec![CREATE_SERVICE_TOKEN, ACME, BOT];
        for channel in &channels {
            args.extend([CHANNEL, channel.as_str()]);
        }
        args.extend_from_slice(extra);
        token_of(&run(&self.talk.ada.machine, &args))
    }

    /// Revokes `token` in Ada's CLI, `skald revoke-service-token`; fails unless the server took it
    /// before and the run succeeds.
    #[track_caller]
    pub fn revoke(&self, token: &str) {
        let record = own_record(self.cloud(), token).expect("a live token");
        let id = id_of(&record);
        let revoked = run(&self.talk.ada.machine, &[REVOKE_SERVICE_TOKEN, ACME, &id]);
        assert!(revoked.success, "{revoked:?}");
    }

    /// Where the front is now.
    pub fn mark(&self) -> Mark {
        Mark {
            sent: self.talk.front.sent().len(),
            answered: self.talk.front.answered().len(),
        }
    }

    /// The `hpke` envelopes to the service agent's device the front saw Ada's machine send since
    /// `mark`, retries included.
    pub fn copies_sent(&self, mark: Mark) -> Vec<RawEnvelope> {
        let bot = self.bot();
        self.talk.front.sent()[mark.sent..]
            .iter()
            .map(|sent| sent.envelope.clone())
            .filter(|envelope| {
                let header = &envelope.envelope().header;
                header.scheme == Scheme::Hpke && header.recipients.as_deref() == Some(&[bot][..])
            })
            .collect()
    }

    /// How many `stored` answers for an envelope to the service agent's device the server gave
    /// Ada's machine since `mark`.
    pub fn copies_stored(&self, mark: Mark) -> usize {
        let bot = Some(self.bot());
        self.talk.front.answered()[mark.answered..]
            .iter()
            .filter(|answer| matches!(answer, Frame::Stored(stored) if stored.send_key.recipient == bot))
            .count()
    }

    /// Fails if Ada's machine sent the service agent a copy since `mark`; `what` names the case.
    #[track_caller]
    pub fn assert_none_sent(&self, mark: Mark, what: &str) {
        let sent = self.copies_sent(mark);
        assert!(sent.is_empty(), "{what}: copies sent: {sent:?}");
    }

    /// Fails if the server stored a copy for the service agent since `mark`; `what` names the case.
    #[track_caller]
    pub fn assert_none_stored(&self, mark: Mark, what: &str) {
        let stored = self.copies_stored(mark);
        assert_eq!(stored, 0, "{what}: copies stored");
    }

    /// Fails unless `copies` are Ada's messages `expected`, in order, each `(text, counter, to)`:
    /// in `channel` of `acme`, under `hpke` to the service agent's device alone, sent by Ada's
    /// device, with `from` Ada's person address, `hops` 0, and an A2A message of the text whose
    /// `messageId` is the envelope's `msg_id`.
    #[track_caller]
    pub fn assert_copies(
        &self,
        copies: &[Copy],
        channel: ChannelId,
        expected: &[(&str, u64, Option<&str>)],
    ) {
        let texts: Vec<String> = copies.iter().map(|copy| a2a(&copy.content).1).collect();
        let wanted: Vec<&str> = expected.iter().map(|(text, ..)| *text).collect();
        assert_eq!(texts, wanted, "the copies' texts");
        let from = self.talk.address(&self.talk.ada);
        for (copy, (text, counter, to)) in copies.iter().zip(expected) {
            let header = &copy.header;
            assert_eq!(
                (header.space, header.channel, header.scheme),
                (self.space(), channel, Scheme::Hpke),
                "{text}: {header:?}"
            );
            assert_eq!(
                header.recipients.as_deref(),
                Some(&[self.bot()][..]),
                "{text}: {header:?}"
            );
            assert_eq!(copy.sender, self.talk.ada.device, "{text}: the sender");
            let content = &copy.content;
            assert_eq!(content.from.to_string(), from, "{text}: {content:?}");
            assert_eq!(
                content.counter.map(std::num::NonZeroU64::get),
                Some(*counter),
                "{text}: the counter"
            );
            assert_eq!(content.hops, 0, "{text}: {content:?}");
            assert_eq!(
                content.to.as_ref().map(ToString::to_string).as_deref(),
                *to,
                "{text}: the addressee"
            );
            assert_eq!(
                a2a(content).0,
                header.msg_id.to_string(),
                "{text}: the A2A messageId"
            );
        }
    }
}

/// The A2A `messageId` and the text of the A2A message in `content`, its text parts joined.
#[track_caller]
pub fn a2a(content: &Content) -> (String, String) {
    let Some(Body::A2a(json)) = &content.body else {
        panic!("not an a2a body: {content:?}");
    };
    let message: Value = serde_json::from_str(json).expect("an A2A message in JSON");
    let text: String = message["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .collect();
    (message["messageId"].as_str().unwrap_or("").to_owned(), text)
}
