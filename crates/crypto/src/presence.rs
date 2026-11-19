//! Presence values under an epoch's presence key: `docs/protocol/agents.md`, "Presence". A value
//! holds its body signed by its device, padded with zeros to a multiple of 4096 bytes and sealed
//! with AES-128-GCM under a random nonce and the spec's associated data.

use std::{collections::HashSet, fmt};

use aes_gcm::{
    Aes128Gcm, KeyInit as _, Nonce,
    aead::{Aead as _, Payload},
};
use hkdf::Hkdf;
use sha2::Sha256;
use skald_proto::{
    Address, Cbor, DecodeError, PRESENCE_LABEL, Presence, PresenceBody, PresencePlaintext, QueueId,
};
use zeroize::Zeroizing;

use crate::history::HistoryKey;
use crate::sign::{SigningKey, verify};

/// The plaintext is padded to a multiple of this many bytes.
const BLOCK: usize = 4096;

/// The most blocks a value holds: the server refuses a longer `ct`.
const MAX_BLOCKS: usize = 16;

impl HistoryKey {
    /// The presence key of this key's epoch:
    /// `HKDF-SHA256(salt = "", IKM = this key, info = "skald presence", L = 16)`.
    pub fn presence_key(&self) -> PresenceKey {
        let mut key = Zeroizing::new([0; 16]);
        Hkdf::<Sha256>::new(Some(b""), self.to_bytes().as_ref())
            .expand(PRESENCE_LABEL.as_bytes(), key.as_mut())
            .expect("16 bytes are within HKDF-SHA256's output bound");
        PresenceKey(key)
    }
}

/// The presence key of an epoch. Its bytes are zeroed on drop and never shown.
pub struct PresenceKey(Zeroizing<[u8; 16]>);

impl PresenceKey {
    /// The value of `body` under this key, which is `body.epoch`'s: `body` signed by `device`,
    /// the signing key of `body.device`.
    ///
    /// # Errors
    /// [`PresenceError::Malformed`] if `body` breaks its CDDL; [`PresenceError::NotDevice`] if
    /// `device` is not `body.device`'s; [`PresenceError::Person`],
    /// [`PresenceError::RepeatedAgent`] or [`PresenceError::RepeatedQueue`] if a reader would drop
    /// it for its agents or queues; [`PresenceError::TooLarge`] if it is over 16 blocks.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn seal(
        &self,
        body: &PresenceBody,
        device: &SigningKey,
    ) -> Result<Presence, PresenceError> {
        if !body.device.is_derived_from(&device.public_key()) {
            return Err(PresenceError::NotDevice);
        }
        check_lists(body)?;
        let bytes = body.to_cbor();
        let signature = device.sign(PRESENCE_LABEL, &[&bytes]);
        let mut plaintext = Zeroizing::new(
            PresencePlaintext::from_parts(bytes, signature)
                .map_err(|_| PresenceError::Malformed)?
                .to_cbor(),
        );
        let padded = plaintext.len().div_ceil(BLOCK) * BLOCK;
        if padded > MAX_BLOCKS * BLOCK {
            return Err(PresenceError::TooLarge);
        }
        plaintext.resize(padded, 0);

        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).expect("the random generator failed");
        let ct = self
            .cipher()
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: &Presence::associated_data(body.device, body.channel, body.epoch),
                },
            )
            .expect("a plaintext within AES-GCM's length bound");
        Ok(Presence {
            device: body.device,
            channel: body.channel,
            epoch: body.epoch,
            nonce,
            ct,
            unknown: Default::default(),
        })
    }

    /// The body of `value` under this key, which is `value.epoch`'s, once it passes the reader
    /// checks that need nothing of the reader's state: check 1; of check 2, the body's `device`,
    /// `channel` and `epoch` being the value's and its signature by `signing`, the signing key of
    /// the certificate of `value.device`, which the caller has checked; check 4 at `now`, seconds
    /// since the Unix epoch; and of check 5, no agent a person or repeated and no queue repeated.
    ///
    /// # Errors
    /// The first check that fails; [`PresenceError::Expired`], with the body, only when the others
    /// pass.
    pub fn open(
        &self,
        value: &Presence,
        signing: &[u8; 32],
        now: u64,
    ) -> Result<PresenceBody, PresenceError> {
        // Check 1.
        let plaintext = self
            .cipher()
            .decrypt(
                &Nonce::from(value.nonce),
                Payload {
                    msg: &value.ct,
                    aad: &Presence::associated_data(value.device, value.channel, value.epoch),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| PresenceError::Open)?;
        let end = match PresencePlaintext::from_cbor(&plaintext) {
            Ok(_) => plaintext.len(),
            Err(DecodeError::TrailingBytes(padding)) => plaintext.len() - padding,
            Err(_) => return Err(PresenceError::Malformed),
        };
        if plaintext[end..].iter().any(|&byte| byte != 0) {
            return Err(PresenceError::Padding);
        }
        let signed = PresencePlaintext::from_cbor(&plaintext[..end])
            .map_err(|_| PresenceError::Malformed)?;

        // Check 2.
        let body = signed.body();
        if (body.device, body.channel, body.epoch) != (value.device, value.channel, value.epoch) {
            return Err(PresenceError::OtherValue);
        }
        if !value.device.is_derived_from(signing) {
            return Err(PresenceError::NotDevice);
        }
        verify(
            signing,
            PRESENCE_LABEL,
            &[signed.body_bytes()],
            signed.signature(),
        )
        .map_err(|_| PresenceError::Signature)?;

        // Check 5.
        check_lists(body)?;

        // Check 4.
        if now > body.expires {
            return Err(PresenceError::Expired {
                body: Box::new(body.clone()),
            });
        }
        Ok(body.clone())
    }

    fn cipher(&self) -> Aes128Gcm {
        Aes128Gcm::new((&*self.0).into())
    }
}

impl fmt::Debug for PresenceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PresenceKey(..)")
    }
}

/// Of check 5: no agent of `body` is a person or repeated, and no queue is repeated.
fn check_lists(body: &PresenceBody) -> Result<(), PresenceError> {
    let mut agents = HashSet::new();
    for status in &body.agents {
        if status.agent.is_person() {
            return Err(PresenceError::Person(status.agent.clone()));
        }
        if !agents.insert(&status.agent) {
            return Err(PresenceError::RepeatedAgent(status.agent.clone()));
        }
    }
    let mut queues = HashSet::new();
    for served in &body.serves {
        if !queues.insert(served.queue) {
            return Err(PresenceError::RepeatedQueue(served.queue));
        }
    }
    Ok(())
}

/// Why a presence value is not sealed or a reader drops it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresenceError {
    /// Check 1: it does not decrypt with the key.
    Open,
    /// Check 1: a nonzero byte follows the plaintext.
    Padding,
    /// Check 1: it holds no presence plaintext, or a body breaks its CDDL.
    Malformed,
    /// Check 2: the body names another device, channel or epoch than the value.
    OtherValue,
    /// Check 2: the signing key is not the device's.
    NotDevice,
    /// Check 2: the signature does not verify with the device's key.
    Signature,
    /// Check 4: `expires` has passed; holds the body, which passes every other check.
    Expired { body: Box<PresenceBody> },
    /// Check 5: an agent is a person.
    Person(Address),
    /// Check 5: an agent is listed twice.
    RepeatedAgent(Address),
    /// Check 5: a queue is listed twice.
    RepeatedQueue(QueueId),
    /// The padded plaintext is over 16 blocks, more than the server takes.
    TooLarge,
}

impl fmt::Display for PresenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => f.write_str("it does not decrypt with the presence key"),
            Self::Padding => f.write_str("a nonzero byte follows its plaintext"),
            Self::Malformed => {
                f.write_str("it holds no presence plaintext, or its body is malformed")
            }
            Self::OtherValue => {
                f.write_str("its body names another device, channel or epoch than the value")
            }
            Self::NotDevice => f.write_str("the signing key is not the device's"),
            Self::Signature => f.write_str("its signature does not verify with the device's key"),
            Self::Expired { body } => write!(f, "it expired at {}", body.expires),
            Self::Person(agent) => write!(f, "it lists the person {agent} as an agent"),
            Self::RepeatedAgent(agent) => write!(f, "it lists the agent {agent} twice"),
            Self::RepeatedQueue(queue) => write!(f, "it lists the queue {queue} twice"),
            Self::TooLarge => f.write_str("it is over 16 blocks of 4096 bytes"),
        }
    }
}

impl std::error::Error for PresenceError {}

#[cfg(test)]
mod tests {
    use skald_proto::{AgentState, AgentStatus, ChannelId, DeviceId};

    use super::*;

    const EXPIRES: u64 = 1_800_000_900;

    fn device() -> SigningKey {
        SigningKey::from_bytes(&[61; 32])
    }

    fn key() -> PresenceKey {
        HistoryKey::from_bytes(&[0x41; 16]).presence_key()
    }

    fn status(agent: &str) -> AgentStatus {
        AgentStatus {
            agent: agent.parse().unwrap(),
            state: AgentState::Available,
            since: EXPIRES - 900,
            note: Some("n".repeat(128)),
            runtime: None,
            unknown: Default::default(),
        }
    }

    /// The body of [`device`] with `agents` agents.
    fn body(agents: usize) -> PresenceBody {
        PresenceBody {
            device: DeviceId::from_signing_key(&device().public_key()),
            channel: ChannelId::generate(),
            epoch: 1,
            revision: 1,
            expires: EXPIRES,
            agents: (0..agents)
                .map(|n| status(&format!("acme/alice/agent-{n}@skald.example")))
                .collect(),
            serves: Vec::new(),
            unknown: Default::default(),
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_value_opens_until_the_second_of_its_expires() {
        let value = key().seal(&body(1), &device()).unwrap();
        let public = device().public_key();
        assert!(key().open(&value, &public, EXPIRES).is_ok());
        assert!(matches!(
            key().open(&value, &public, EXPIRES + 1),
            Err(PresenceError::Expired { .. })
        ));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_body_is_sealed_only_by_its_device_and_with_its_lists_as_a_reader_takes_them() {
        let other = SigningKey::from_bytes(&[62; 32]);
        assert_eq!(key().seal(&body(1), &other), Err(PresenceError::NotDevice));

        let mut person = body(1);
        person.agents.push(status("acme/alice/user@skald.example"));
        assert!(matches!(
            key().seal(&person, &device()),
            Err(PresenceError::Person(_))
        ));
        let mut twice = body(1);
        twice.agents.push(twice.agents[0].clone());
        assert!(matches!(
            key().seal(&twice, &device()),
            Err(PresenceError::RepeatedAgent(_))
        ));

        let malformed = PresenceBody {
            revision: 0,
            ..body(1)
        };
        assert_eq!(
            key().seal(&malformed, &device()),
            Err(PresenceError::Malformed)
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_value_of_16_blocks_is_sealed_and_a_longer_one_refused() {
        let blocks = |body: &PresenceBody| {
            let signed = device().sign(PRESENCE_LABEL, &[&body.to_cbor()]);
            PresencePlaintext::from_parts(body.to_cbor(), signed)
                .unwrap()
                .to_cbor()
                .len()
                .div_ceil(BLOCK)
        };
        let mut agents = 1;
        while blocks(&body(agents + 1)) <= MAX_BLOCKS {
            agents += 1;
        }
        let largest = body(agents);
        assert_eq!(blocks(&largest), MAX_BLOCKS);
        let value = key().seal(&largest, &device()).unwrap();
        assert_eq!(value.ct.len(), MAX_BLOCKS * BLOCK + 16);
        assert_eq!(
            key().seal(&body(agents + 1), &device()),
            Err(PresenceError::TooLarge)
        );
    }

    /// `body` signed by `signer` and sealed as the spec says, without the crate's checks, in a value
    /// of `device`, `body.channel` and `body.epoch`.
    fn forged(body: &PresenceBody, device: DeviceId, signer: &SigningKey) -> Presence {
        let bytes = body.to_cbor();
        let signature = signer.sign(PRESENCE_LABEL, &[&bytes]);
        let mut plaintext = PresencePlaintext::from_parts(bytes, signature)
            .unwrap()
            .to_cbor();
        plaintext.resize(plaintext.len().div_ceil(BLOCK) * BLOCK, 0);
        let nonce = [7; 12];
        let ct = key()
            .cipher()
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &plaintext,
                    aad: &Presence::associated_data(device, body.channel, body.epoch),
                },
            )
            .unwrap();
        Presence {
            device,
            channel: body.channel,
            epoch: body.epoch,
            nonce,
            ct,
            unknown: Default::default(),
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn past_expires_a_value_that_fails_another_check_gives_that_checks_error() {
        let late = EXPIRES + 1;
        let public = device().public_key();
        let body = body(2);
        assert_eq!(
            key().open(&forged(&body, body.device, &device()), &public, late),
            Err(PresenceError::Expired {
                body: Box::new(body.clone())
            }),
            "only check 4 fails: the opened body"
        );

        let other = SigningKey::from_bytes(&[62; 32]);
        assert_eq!(
            key().open(&forged(&body, body.device, &other), &public, late),
            Err(PresenceError::Signature)
        );
        let naming_other = PresenceBody {
            device: DeviceId::from_signing_key(&other.public_key()),
            ..body.clone()
        };
        assert_eq!(
            key().open(
                &forged(&naming_other, body.device, &device()),
                &public,
                late
            ),
            Err(PresenceError::OtherValue)
        );
        let person: Address = "acme/alice/user@skald.example".parse().unwrap();
        let mut with_person = body.clone();
        with_person.agents.push(status(&person.to_string()));
        assert_eq!(
            key().open(&forged(&with_person, body.device, &device()), &public, late),
            Err(PresenceError::Person(person))
        );
        let mut twice = body.clone();
        twice.agents.push(twice.agents[0].clone());
        assert_eq!(
            key().open(&forged(&twice, body.device, &device()), &public, late),
            Err(PresenceError::RepeatedAgent(body.agents[0].agent.clone()))
        );
        let other_key = HistoryKey::from_bytes(&[0x42; 16]).presence_key();
        assert_eq!(
            other_key.open(&forged(&body, body.device, &device()), &public, late),
            Err(PresenceError::Open)
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_value_signed_by_another_key_than_its_devices_is_refused_whatever_key_is_given() {
        let other = SigningKey::from_bytes(&[62; 32]);
        let body = body(1);
        let value = forged(&body, body.device, &other);
        assert_eq!(
            key().open(&value, &other.public_key(), EXPIRES),
            Err(PresenceError::NotDevice)
        );
        assert_eq!(
            key().open(&value, &device().public_key(), EXPIRES),
            Err(PresenceError::Signature)
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_presence_key_is_never_shown() {
        assert_eq!(format!("{:?}", key()), "PresenceKey(..)");
    }

    /// Each reason names the check that failed.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn each_error_says_why_the_value_fails() {
        let agent: Address = "acme/alice/user@skald.example".parse().unwrap();
        let queue: QueueId = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5eb".parse().unwrap();
        let reasons = [
            (
                PresenceError::Open,
                "it does not decrypt with the presence key".to_owned(),
            ),
            (
                PresenceError::Padding,
                "a nonzero byte follows its plaintext".to_owned(),
            ),
            (
                PresenceError::Malformed,
                "it holds no presence plaintext, or its body is malformed".to_owned(),
            ),
            (
                PresenceError::OtherValue,
                "its body names another device, channel or epoch than the value".to_owned(),
            ),
            (
                PresenceError::NotDevice,
                "the signing key is not the device's".to_owned(),
            ),
            (
                PresenceError::Signature,
                "its signature does not verify with the device's key".to_owned(),
            ),
            (
                PresenceError::Expired {
                    body: Box::new(body(0)),
                },
                format!("it expired at {EXPIRES}"),
            ),
            (
                PresenceError::Person(agent.clone()),
                format!("it lists the person {agent} as an agent"),
            ),
            (
                PresenceError::RepeatedAgent(agent.clone()),
                format!("it lists the agent {agent} twice"),
            ),
            (
                PresenceError::RepeatedQueue(queue),
                format!("it lists the queue {queue} twice"),
            ),
            (
                PresenceError::TooLarge,
                "it is over 16 blocks of 4096 bytes".to_owned(),
            ),
        ];
        for (error, reason) in reasons {
            assert_eq!(error.to_string(), reason);
        }
    }
}
