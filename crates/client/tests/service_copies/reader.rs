//! The service agent as the tests play it: a test client with its key secret and a token that
//! reads its mailbox over the delivery protocol, proving the key of its device, and opens each
//! `hpke` envelope with the key secret's HPKE key.

use skald_crypto::{HpkeKey, SigningKey, open_message};
use skald_proto::{Cert, Content, DeviceId, Header, Push, RawEnvelope};

use crate::agents::key_of;
use crate::cloud::Cloud;
use crate::peer::{Peer, last_seq};

/// A copy the service agent took from its mailbox, opened and its signature checked.
#[derive(Debug)]
pub struct Copy {
    pub header: Header,
    /// The device the payload authenticates as its sender.
    pub sender: DeviceId,
    pub content: Content,
}

/// The service agent's device, played by the test.
pub struct Reader {
    pub device: DeviceId,
    signing: SigningKey,
    hpke: HpkeKey,
    /// The highest `seq` it acked.
    cursor: u64,
}

impl Reader {
    /// The device whose keys `key`, a key secret as the CLI printed it, holds.
    #[track_caller]
    pub fn of(key: &str) -> Self {
        let secret = key_of(key);
        let signing = SigningKey::from_bytes(&secret.signing);
        let device = DeviceId::from_signing_key(&signing.public_key());
        Reader {
            device,
            signing,
            hpke: HpkeKey::from_bytes(&secret.hpke),
            cursor: 0,
        }
    }

    /// Connects with `token` and the proof of the device's key, takes every entry after its
    /// cursor and acks them; gives the `message` entries' envelopes, in order. Fails unless the
    /// server takes the connection.
    #[track_caller]
    pub fn take_in(&mut self, cloud: &Cloud, token: &str) -> Vec<RawEnvelope> {
        let mut peer = Peer::connect_on(cloud, token, self.device, &self.signing);
        let entries = peer.catch_up(self.cursor);
        if let Some(seq) = last_seq(&entries) {
            peer.ack(seq);
            self.cursor = seq;
        }
        entries
            .into_iter()
            .filter_map(|entry| match entry {
                Push::Message { envelope, .. } => Some(envelope),
                _ => None,
            })
            .collect()
    }

    /// As [`Reader::take_in`]; each envelope opened with the device's HPKE key and its signature
    /// checked with `sender`, the certificate of the device that must have sent it.
    #[track_caller]
    pub fn copies(&mut self, cloud: &Cloud, token: &str, sender: &Cert) -> Vec<Copy> {
        let envelopes = self.take_in(cloud, token);
        envelopes.iter().map(|one| self.open(one, sender)).collect()
    }

    /// `envelope` opened with the device's HPKE key, its signature checked with `sender`.
    #[track_caller]
    pub fn open(&self, envelope: &RawEnvelope, sender: &Cert) -> Copy {
        let envelope = envelope.envelope();
        let opened = open_message(&envelope.header, &envelope.payload, self.device, &self.hpke)
            .unwrap_or_else(|err| panic!("a copy that does not open: {err:?}"));
        let from = opened.sender();
        let content = opened
            .verify(sender)
            .unwrap_or_else(|err| panic!("a copy from {from} whose signature fails: {err:?}"));
        Copy {
            header: envelope.header.clone(),
            sender: from,
            content,
        }
    }
}
