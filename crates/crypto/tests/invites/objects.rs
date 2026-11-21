//! The tests' invites and acceptances: built from their spec fields, devices by the seed of their
//! signing key, invite keys by their secret, encoded by [`crate::cbor`] and signed with
//! ed25519-dalek over `[label, body]`, apart from the crates under test.

use std::fmt::Debug;

use ed25519_dalek::Signer as _;
use hkdf::Hkdf;
use sha2::Sha256;
use skald_proto::{
    AcceptanceBody, AccountId, Cbor, ChannelId, DeviceId, InviteBody, InviteId, Role, Signed,
    SignedBody, SpaceId, Unknown,
};

use crate::cbor::{Value, bytes, text};
use crate::fields::signed_cbor;
use crate::fixtures::{LAPTOP, OTHER_ACCOUNT, OTHER_MASTER, device, public, sign};

/// The label of an invite's signature.
pub const INVITE: &str = "skald invite";
/// The label of an acceptance's signature.
pub const ACCEPTANCE: &str = "skald acceptance";

/// The invite the tests' objects name, as the server assigned it.
pub const INVITE_ID: InviteId = InviteId::from_bytes([0x1c; 16]);
pub const SPACE: SpaceId = SpaceId::from_bytes([0x5a; 16]);
/// The space's default channel, and another channel of it.
pub const GENERAL: ChannelId = ChannelId::from_bytes([0xc1; 16]);
pub const DESIGN: ChannelId = ChannelId::from_bytes([0xc2; 16]);
/// When the tests' invites expire, in seconds since the Unix epoch.
pub const EXPIRES: u64 = 1_800_604_800;

/// The secret of the tests' invites, and another one.
pub const SECRET: [u8; 32] = [0x5e; 32];
pub const OTHER_SECRET: [u8; 32] = [0x5f; 32];

/// The newcomer: an account of its own, with its own master key.
pub const NEWCOMER: AccountId = OTHER_ACCOUNT;

/// The newcomer's master public key.
pub fn newcomer_master() -> [u8; 32] {
    public(OTHER_MASTER)
}

/// The invite key pair of `secret`: the Ed25519 private key
/// `HKDF-SHA256(salt = "", IKM = secret, info = "skald invite", L = 32)`.
pub fn invite_dalek(secret: &[u8; 32]) -> ed25519_dalek::SigningKey {
    let mut private = [0; 32];
    Hkdf::<Sha256>::new(Some(b""), secret)
        .expand(b"skald invite", &mut private)
        .expect("32 bytes of HKDF-SHA256");
    ed25519_dalek::SigningKey::from_bytes(&private)
}

/// The invite key of `secret`: the public key of [`invite_dalek`].
pub fn invite_public(secret: &[u8; 32]) -> [u8; 32] {
    invite_dalek(secret).verifying_key().to_bytes()
}

/// A key that signs an object.
#[derive(Debug, Clone, Copy)]
pub enum By {
    /// The signing key of the device of this seed.
    Device(u8),
    /// The invite key of this secret.
    Secret([u8; 32]),
}

impl By {
    /// The signature over the encoded `[label, body]`.
    pub fn sign(self, label: &str, body: &[u8]) -> [u8; 64] {
        match self {
            Self::Device(seed) => sign(seed, label, body),
            Self::Secret(secret) => {
                let message = Value::Array(vec![text(label), bytes(body)]).encode();
                invite_dalek(&secret).sign(&message).to_bytes()
            }
        }
    }

    /// The public key it verifies with.
    pub fn public(self) -> [u8; 32] {
        match self {
            Self::Device(seed) => public(seed),
            Self::Secret(secret) => invite_public(&secret),
        }
    }
}

/// The spec fields of an invite body, keys 1 to 7.
pub fn invite_fields(body: &InviteBody) -> Vec<(i64, Value)> {
    let listed = body
        .channels
        .iter()
        .map(|&(channel, history)| {
            Value::Array(vec![bytes(channel.as_bytes()), Value::Bool(history)])
        })
        .collect();
    vec![
        (1, bytes(body.invite.as_bytes())),
        (2, bytes(body.space.as_bytes())),
        (3, bytes(&body.key)),
        (4, Value::Uint(body.role as u64)),
        (5, Value::Array(listed)),
        (6, bytes(body.signer.as_bytes())),
        (7, Value::Uint(body.expires)),
    ]
}

/// The spec fields of an acceptance body, keys 1 to 4.
pub fn acceptance_fields(
    invite: InviteId,
    account: AccountId,
    master: &[u8; 32],
    handle: &str,
) -> Vec<(i64, Value)> {
    vec![
        (1, bytes(invite.as_bytes())),
        (2, bytes(account.as_bytes())),
        (3, bytes(master)),
        (4, text(handle)),
    ]
}

/// A fixture object: its body's spec fields, fields of later versions included, the key that signs
/// it and the label its signature covers.
pub trait Object: Debug {
    /// The body's spec fields.
    fn spec_fields(&self) -> Vec<(i64, Value)>;
    /// The body's fields of later versions.
    fn extra(&self) -> &[(i64, Value)];
    /// The key that signs it, and the label its signature covers.
    fn signing(&self) -> (By, &'static str);

    /// The fields of the body, as the spec numbers them, and those of later versions.
    fn fields(&self) -> Vec<(i64, Value)> {
        let mut fields = self.spec_fields();
        fields.extend_from_slice(self.extra());
        fields
    }

    /// The body, a map.
    fn value(&self) -> Value {
        Value::Map(self.fields())
    }

    /// The encoded body.
    fn body(&self) -> Vec<u8> {
        self.value().encode()
    }

    /// The signature over `[label, body]`.
    fn signature(&self) -> [u8; 64] {
        let (by, label) = self.signing();
        by.sign(label, &self.body())
    }

    /// The encoded object: `[body, signature]`.
    fn bytes(&self) -> Vec<u8> {
        signed_cbor(&self.body(), &self.signature())
    }
}

/// Checks that `signed` holds the body and signature of `fixture` and encodes to its bytes.
pub fn assert_is<B: SignedBody>(signed: &Signed<B>, fixture: &impl Object) {
    assert_eq!(
        signed.body_bytes(),
        fixture.body(),
        "{fixture:?}: the body, byte for byte"
    );
    assert_eq!(
        signed.signature(),
        &fixture.signature(),
        "{fixture:?}: the signature"
    );
    assert_eq!(
        signed.to_cbor(),
        fixture.bytes(),
        "{fixture:?}: the object, byte for byte"
    );
}

/// An invite's spec fields, its signer device by seed, and how it is signed.
#[derive(Debug, Clone)]
pub struct Inv {
    pub invite: InviteId,
    pub space: SpaceId,
    /// The invite key.
    pub key: [u8; 32],
    pub role: Role,
    /// Each channel and whether the newcomer reads its earlier history, in order.
    pub channels: Vec<(ChannelId, bool)>,
    /// The device the body names as `signer`.
    pub signer: u8,
    pub expires: u64,
    /// Fields of later versions.
    pub extra: Vec<(i64, Value)>,
    /// The key that signs it, and the label its signature covers.
    pub by: By,
    pub label: &'static str,
}

impl Inv {
    /// The laptop's invite of a member to the default channel, reading its history, then to
    /// `design`, not reading it, its key that of [`SECRET`].
    pub fn of_member() -> Self {
        Self {
            invite: INVITE_ID,
            space: SPACE,
            key: invite_public(&SECRET),
            role: Role::Member,
            channels: vec![(GENERAL, true), (DESIGN, false)],
            signer: LAPTOP,
            expires: EXPIRES,
            extra: Vec::new(),
            by: By::Device(LAPTOP),
            label: INVITE,
        }
    }

    /// As [`Inv::of_member`], an invite of an admin to no channel.
    pub fn of_admin_to_no_channel() -> Self {
        Self {
            role: Role::Admin,
            channels: Vec::new(),
            ..Self::of_member()
        }
    }

    /// The device of `signer`.
    pub fn signer_id(&self) -> DeviceId {
        device(self.signer)
    }

    /// The body as a client builds it; only for an invite without fields of later versions.
    pub fn proto_body(&self) -> InviteBody {
        assert!(
            self.extra.is_empty(),
            "a body with fields of later versions"
        );
        InviteBody {
            invite: self.invite,
            space: self.space,
            key: self.key,
            role: self.role,
            channels: self.channels.clone(),
            signer: self.signer_id(),
            expires: self.expires,
            unknown: Unknown::default(),
        }
    }
}

impl Object for Inv {
    fn spec_fields(&self) -> Vec<(i64, Value)> {
        let spec = Self {
            extra: Vec::new(),
            ..self.clone()
        };
        invite_fields(&spec.proto_body())
    }

    fn extra(&self) -> &[(i64, Value)] {
        &self.extra
    }

    fn signing(&self) -> (By, &'static str) {
        (self.by, self.label)
    }
}

/// An acceptance's spec fields and how it is signed.
#[derive(Debug, Clone)]
pub struct Acc {
    pub invite: InviteId,
    /// The newcomer's account.
    pub account: AccountId,
    /// The newcomer's master public key.
    pub master: [u8; 32],
    /// The handle it asks for.
    pub handle: &'static str,
    /// Fields of later versions.
    pub extra: Vec<(i64, Value)>,
    /// The key that signs it, and the label its signature covers.
    pub by: By,
    pub label: &'static str,
}

impl Acc {
    /// The newcomer's acceptance of [`Inv::of_member`] as `bob`, signed with the invite key of
    /// [`SECRET`].
    pub fn of_newcomer() -> Self {
        Self {
            invite: INVITE_ID,
            account: NEWCOMER,
            master: newcomer_master(),
            handle: "bob",
            extra: Vec::new(),
            by: By::Secret(SECRET),
            label: ACCEPTANCE,
        }
    }

    /// As [`Acc::of_newcomer`], with a handle of digits and a hyphen.
    pub fn with_other_handle() -> Self {
        Self {
            handle: "bob-2",
            ..Self::of_newcomer()
        }
    }

    /// The body as a client builds it; only for an acceptance without fields of later versions.
    pub fn proto_body(&self) -> AcceptanceBody {
        assert!(
            self.extra.is_empty(),
            "a body with fields of later versions"
        );
        AcceptanceBody {
            invite: self.invite,
            account: self.account,
            master: self.master,
            handle: self.handle.parse().expect("a valid handle"),
            unknown: Unknown::default(),
        }
    }
}

impl Object for Acc {
    fn spec_fields(&self) -> Vec<(i64, Value)> {
        acceptance_fields(self.invite, self.account, &self.master, self.handle)
    }

    fn extra(&self) -> &[(i64, Value)] {
        &self.extra
    }

    fn signing(&self) -> (By, &'static str) {
        (self.by, self.label)
    }
}
