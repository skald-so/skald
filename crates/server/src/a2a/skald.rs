//! Live grant and device proof on the binding's configured Skald origin.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use skald_crypto::SigningKey;
use skald_proto::{
    Cbor as _, ChannelId, DeviceId, Grant, PROOF_HEADER, PROTOCOL_VERSION, QueueId, REQUEST_LABEL,
    RequestProof, VERSION_HEADER,
};
use ureq::Agent;
use zeroize::Zeroizing;

use super::{Binding, Destination};
use crate::http_client::json_body;

/// Skald service identity held in gateway memory.
pub struct ServiceSecret {
    token: Zeroizing<String>,
    key: SigningKey,
}

impl ServiceSecret {
    pub fn new(token: String, key: SigningKey) -> Self {
        Self {
            token: Zeroizing::new(token),
            key,
        }
    }
}

#[derive(Deserialize)]
struct TokenRecord {
    device: String,
    permissions: Vec<String>,
    channels: Vec<String>,
    grant: String,
}

impl TokenRecord {
    fn permits(&self, channel: ChannelId, queue: bool) -> Option<DeviceId> {
        let device = self.device.parse::<DeviceId>().ok()?;
        let grant = Grant::from_cbor(&URL_SAFE_NO_PAD.decode(&self.grant).ok()?).ok()?;
        let permission = |name| self.permissions.iter().any(|permission| permission == name);
        (grant.body.service == device
            && grant.body.channels.iter().any(|id| *id == channel)
            && self
                .channels
                .iter()
                .any(|listed| listed == &channel.to_string())
            && permission("send")
            && permission("receive")
            && (!queue || permission("queues")))
        .then_some(device)
    }
}

#[derive(Deserialize)]
struct Challenge {
    challenge: String,
}

#[derive(Deserialize)]
struct Queues {
    queues: Vec<Queue>,
}

#[derive(Deserialize)]
struct Queue {
    id: String,
}

impl Binding {
    /// Check the live token's grant, then let the existing channel endpoint verify its device
    /// proof and send scope. For a queue binding, require the configured queue in that channel.
    pub(super) fn granted(&self, agent: &Agent) -> bool {
        self.check(agent).is_some()
    }

    fn check(&self, agent: &Agent) -> Option<()> {
        let secret = self
            .secret
            .as_ref()
            .filter(|secret| !secret.token.is_empty())?;
        let authorization = Zeroizing::new(format!("Bearer {}", secret.token.as_str()));
        let version = PROTOCOL_VERSION.to_string();
        let record: TokenRecord = json_body(
            agent
                .get(self.skald_origin.at("/service-token"))
                .header("authorization", authorization.as_str())
                .header(VERSION_HEADER, version.as_str())
                .call(),
        )
        .ok()?;
        let queue: Option<QueueId> = match self.destination {
            Destination::Queue(queue) => Some(queue),
            Destination::Member { .. } => None,
        };
        let device = record.permits(self.channel, queue.is_some())?;
        let challenge: Challenge = json_body(
            agent
                .post(self.skald_origin.at("/session/challenge"))
                .header("authorization", authorization.as_str())
                .header(VERSION_HEADER, version.as_str())
                .send_empty(),
        )
        .ok()?;
        let challenge = URL_SAFE_NO_PAD.decode(challenge.challenge).ok()?;
        let signature = secret.key.sign(
            REQUEST_LABEL,
            &[&challenge, self.skald_origin.origin().as_bytes()],
        );
        let proof = URL_SAFE_NO_PAD.encode(
            RequestProof {
                challenge,
                device,
                signature,
            }
            .to_cbor(),
        );
        let path = format!("/spaces/{}/channels/{}/queues", self.space, self.channel);
        let found: Queues = json_body(
            agent
                .get(self.skald_origin.at(&path))
                .header("authorization", authorization.as_str())
                .header(VERSION_HEADER, version.as_str())
                .header(PROOF_HEADER, proof)
                .call(),
        )
        .ok()?;
        queue
            .is_none_or(|queue| {
                let id = queue.to_string();
                found.queues.iter().any(|listed| listed.id == id)
            })
            .then_some(())
    }
}
