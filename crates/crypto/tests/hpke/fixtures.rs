//! Devices, keys, certificates, the spec's header and content, and payloads made apart from the
//! sealing under test: with the single-shot seal, the proto types and ed25519-dalek.

use ed25519_dalek::Signer as _;
use skald_crypto::{HpkeKey, SigningKey, issue, open_message};
use skald_proto::{
    AccountId, Cbor, Cert, CertBody, Content, DeviceId, DeviceKind, Envelope, Header, HpkePayload,
    HpkePlaintext, Unknown, signed_data,
};

/// `info` of an `hpke` payload, and the label of its signature.
pub const INFO: &[u8] = b"skald hpke";
pub const LABEL: &str = "skald hpke";

/// The secret keys of RFC 8032, §7.1: "TEST 1" is Alice's device, "TEST 2" Bob's, "TEST 3"
/// Carol's, as in `docs/protocol/test-vectors.md`.
const ALICE: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const BOB: &str = "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb";
const CAROL: &str = "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7";

/// `docs/protocol/test-vectors.md`, "Envelope under `hpke`": its header names Bob's device in
/// `recipients`.
const ENVELOPE: &str = "
82a701010250019a6f1e2b3c7d4e8f5061a2b3c4d5e60350019a6f1e2b3c7d4e
8f5061a2b3c4d5e70450019a6f201a007c008a00000000000002050206815018
8e6ae45b2d8247b1e1583b84bd49c7071a000151805834825820e0e0e0e0e0e0
e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e050c0c1c2c3c4
c5c6c7c8c9cacbcccdcecf";

/// `docs/protocol/test-vectors.md`, "HPKE plaintext": Alice's device sends its content.
const PLAINTEXT: &str = "
835019ac5f784938823586a6c5809a18f6dd58e9a501782361636d652f616c69
63652f636c617564652d61706940736b616c642e6578616d706c6502781c6163
6d652f626f622f636f64657840736b616c642e6578616d706c65030104000678
9c7b226d6573736167654964223a2230313961366632302d316130302d376330
302d386130302d303030303030303030303032222c22636f6e74657874496422
3a2230313961366632302d303030302d373030302d383030302d303030303030
303063306465222c22726f6c65223a22524f4c455f55534552222c2270617274
73223a5b7b2274657874223a22526576696577205052203132227d5d7d58405a
5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a
5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

/// Bytes from hex digits; whitespace between them is skipped.
pub fn hex(digits: &str) -> Vec<u8> {
    let digits: String = digits.split_whitespace().collect();
    assert_eq!(digits.len() % 2, 0, "{digits}");
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&digits[at..at + 2], 16).unwrap())
        .collect()
}

pub fn hex32(digits: &str) -> [u8; 32] {
    hex(digits).try_into().unwrap()
}

/// The devices of the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Alice,
    Bob,
    Carol,
}

impl Device {
    fn secret(self) -> [u8; 32] {
        hex32(match self {
            Self::Alice => ALICE,
            Self::Bob => BOB,
            Self::Carol => CAROL,
        })
    }

    /// The signing key, as it comes from key storage.
    pub fn signing(self) -> SigningKey {
        SigningKey::from_bytes(&self.secret())
    }

    fn dalek(self) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&self.secret())
    }

    pub fn id(self) -> DeviceId {
        DeviceId::from_signing_key(&self.dalek().verifying_key().to_bytes())
    }

    /// The HPKE key, as it comes from key storage.
    pub fn hpke(self) -> HpkeKey {
        HpkeKey::from_bytes(
            &[match self {
                Self::Alice => 0xa0,
                Self::Bob => 0xb0,
                Self::Carol => 0xc0,
            }; 32],
        )
    }

    /// The HPKE public key of its certificate.
    pub fn hpke_public(self) -> [u8; 32] {
        self.hpke().public_key()
    }

    /// Its valid certificate, as the caller has checked it.
    pub fn cert(self) -> Cert {
        let master = SigningKey::from_bytes(&[0xff; 32]);
        let body = CertBody {
            account: AccountId::from_bytes([0xaa; 16]),
            device: self.id(),
            kind: DeviceKind::Client,
            signing: self.signing().public_key(),
            hpke: self.hpke_public(),
            issuer: None,
            issued: 1_800_000_000,
            ssh: None,
            name: format!("{self:?}"),
            unknown: Unknown::default(),
        };
        issue(&body, &master, &self.signing()).unwrap()
    }

    /// Its signature over the encoded `[label, parts...]`, made with ed25519-dalek.
    pub fn sign(self, label: &str, parts: &[&[u8]]) -> [u8; 64] {
        self.dalek().sign(&signed_data(label, parts)).to_bytes()
    }
}

/// The header of the spec's vector: from Alice's device to Bob's.
pub fn header() -> Header {
    let header = Envelope::from_cbor(&hex(ENVELOPE)).unwrap().header;
    assert_eq!(header.recipients.as_deref(), Some(&[Device::Bob.id()][..]));
    header
}

/// The content of the spec's vector.
pub fn content() -> Content {
    let plaintext = HpkePlaintext::from_cbor(&hex(PLAINTEXT)).unwrap();
    assert_eq!(plaintext.sender, Device::Alice.id());
    plaintext.content
}

/// The encoded `hpke-plaintext` of `sender`, `content` and `signature`.
pub fn plaintext(sender: Device, content: &Content, signature: [u8; 64]) -> Vec<u8> {
    HpkePlaintext {
        sender: sender.id(),
        content: content.clone(),
        signature,
    }
    .to_cbor()
}

/// The signature of `signer` over `["skald hpke", header, content]`, as `sender` would make it.
pub fn signature(signer: Device, header: &Header, content: &Content) -> [u8; 64] {
    signer.sign(LABEL, &[&header.to_cbor(), &content.to_cbor()])
}

/// The encoded `hpke-payload` of `plaintext`, sealed with the single-shot seal to `recipient`'s
/// key, with `info` and `aad`.
pub fn sealed_with(recipient: &[u8; 32], info: &[u8], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let (enc, ct) = HpkeKey::seal(recipient, info, aad, plaintext).unwrap();
    HpkePayload { enc, ct }.to_cbor()
}

/// The payload of `plaintext` as the spec seals it to `recipient`, under `header`.
pub fn sealed(recipient: Device, header: &Header, plaintext: &[u8]) -> Vec<u8> {
    sealed_with(&recipient.hpke_public(), INFO, &header.to_cbor(), plaintext)
}

/// The payload of the spec's content, from Alice's device to `recipient` under `header`, made apart
/// from the sealing under test.
pub fn made(recipient: Device, header: &Header) -> Vec<u8> {
    let content = content();
    let signature = signature(Device::Alice, header, &content);
    sealed(
        recipient,
        header,
        &plaintext(Device::Alice, &content, signature),
    )
}

/// What `device` gets from `payload` under `header`: its sender and content, if it opens with the
/// device's key and verifies with the certificate of the device it names as `sender`.
pub fn opened(device: Device, header: &Header, payload: &[u8]) -> Option<(DeviceId, Content)> {
    let message = open_message(header, payload, device.id(), &device.hpke()).ok()?;
    let sender = message.sender();
    let cert = [Device::Alice, Device::Bob, Device::Carol]
        .into_iter()
        .find(|known| known.id() == sender)?
        .cert();
    let content = message.verify(&cert).ok()?;
    Some((sender, content))
}
