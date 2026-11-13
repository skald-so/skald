//! The accounts, devices and revisions of the tests: a space `acme` with the admins alice and bob
//! and the member carol; dave's account is outside it. The client under test is one of alice's
//! devices: its own account is alice's, with alice's master key pinned.
//!
//! Revisions are built from their spec fields, encoded by [`crate::cbor`] and signed with
//! ed25519-dalek over `["skald roster", body]`, apart from the crates under test.

use ed25519_dalek::Signer as _;
use skald_crypto::{SigningKey, accept_next, check_roster, issue};
use skald_proto::{
    AccountId, Cbor, Cert, CertBody, ChannelId, DeviceId, DeviceKind, Name, Roster, RosterBody,
    SpaceId, Unknown,
};

use crate::cbor::{Value, bytes, text};

// Master keys, by seed.
pub const ALICE_MASTER: u8 = 1;
pub const BOB_MASTER: u8 = 2;
pub const CAROL_MASTER: u8 = 3;
pub const DAVE_MASTER: u8 = 4;
/// A master key of no account in the space.
pub const ROGUE_MASTER: u8 = 5;
/// A key of the server's own, which no account's chain ends at.
pub const SERVER_KEY: u8 = 6;

// Devices, by the seed of their signing key.
pub const ALICE_LAPTOP: u8 = 10;
pub const ALICE_BROWSER: u8 = 11;
/// A `service` device certified on alice's account, an admin's.
pub const ALICE_SERVICE: u8 = 12;
/// A `client` device on alice's account whose certificate only the server signed.
pub const ALICE_BY_SERVER: u8 = 13;
/// A `client` device on alice's account chained to another master key.
pub const ALICE_ROGUE: u8 = 14;
pub const BOB_LAPTOP: u8 = 20;
/// A valid `service` device on bob's account: the space's service agent.
pub const BOB_CI: u8 = 21;
/// A `client` device on bob's account chained to another master key.
pub const BOB_ROGUE: u8 = 22;
/// A `service` device on bob's account certified by [`BOB_ROGUE`].
pub const BOB_ROGUE_CI: u8 = 23;
/// A `client` device on bob's account chained to carol's master key.
pub const BOB_BY_CAROL: u8 = 24;
/// A `client` device on bob's account chained to alice's master key, an admin's.
pub const BOB_BY_ALICE: u8 = 25;
pub const CAROL_LAPTOP: u8 = 30;
pub const DAVE_LAPTOP: u8 = 40;
/// A valid `service` device on dave's account, outside the space.
pub const DAVE_CI: u8 = 41;
/// A `client` device on dave's account chained to bob's master key, a member's.
pub const DAVE_BY_BOB: u8 = 42;
/// A `service` device on dave's account certified by [`DAVE_BY_BOB`].
pub const DAVE_BY_BOB_CI: u8 = 43;
/// A device with no certificate.
pub const UNCERTIFIED: u8 = 50;

pub const ALICE: AccountId = AccountId::from_bytes([0xa1; 16]);
pub const BOB: AccountId = AccountId::from_bytes([0xb0; 16]);
pub const CAROL: AccountId = AccountId::from_bytes([0xca; 16]);
pub const DAVE: AccountId = AccountId::from_bytes([0xda; 16]);

pub const SPACE: SpaceId = SpaceId::from_bytes([0x5a; 16]);
pub const OTHER_SPACE: SpaceId = SpaceId::from_bytes([0x5b; 16]);
pub const CHANNEL: ChannelId = ChannelId::from_bytes([0xc1; 16]);
pub const SECOND_CHANNEL: ChannelId = ChannelId::from_bytes([0xc2; 16]);
pub const BOB_INVITE: [u8; 16] = [0x1b; 16];
pub const CAROL_INVITE: [u8; 16] = [0x1c; 16];

pub const ADMIN: u64 = 1;
pub const MEMBER: u64 = 2;
pub const READ_ONLY: u64 = 1;
pub const FULL: u64 = 2;
pub const DAY: u64 = 86_400;
pub const THIRTY_DAYS: u64 = 30 * DAY;
pub const TIMESTAMP: u64 = 1_763_000_000;

pub fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

pub fn public(seed: u8) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
        .verifying_key()
        .to_bytes()
}

pub fn device(seed: u8) -> DeviceId {
    DeviceId::from_signing_key(&public(seed))
}

/// The signature of `seed`'s key over the encoded `[label, body]`.
pub fn signature(seed: u8, label: &str, body: &[u8]) -> [u8; 64] {
    let signed = Value::Array(vec![text(label), bytes(body)]).encode();
    ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
        .sign(&signed)
        .to_bytes()
}

/// Who signs a certificate.
#[derive(Clone, Copy)]
enum By {
    /// This key, as a master key: the certificate has no `issuer`.
    Key(u8),
    /// This device, as its `issuer`.
    Device(u8),
}

const DEVICES: [(u8, AccountId, DeviceKind, By); 16] = [
    (
        ALICE_LAPTOP,
        ALICE,
        DeviceKind::Client,
        By::Key(ALICE_MASTER),
    ),
    (
        ALICE_BROWSER,
        ALICE,
        DeviceKind::Browser,
        By::Device(ALICE_LAPTOP),
    ),
    (
        ALICE_SERVICE,
        ALICE,
        DeviceKind::Service,
        By::Device(ALICE_LAPTOP),
    ),
    (
        ALICE_BY_SERVER,
        ALICE,
        DeviceKind::Client,
        By::Key(SERVER_KEY),
    ),
    (
        ALICE_ROGUE,
        ALICE,
        DeviceKind::Client,
        By::Key(ROGUE_MASTER),
    ),
    (BOB_LAPTOP, BOB, DeviceKind::Client, By::Key(BOB_MASTER)),
    (BOB_CI, BOB, DeviceKind::Service, By::Device(BOB_LAPTOP)),
    (BOB_ROGUE, BOB, DeviceKind::Client, By::Key(ROGUE_MASTER)),
    (
        BOB_ROGUE_CI,
        BOB,
        DeviceKind::Service,
        By::Device(BOB_ROGUE),
    ),
    (BOB_BY_CAROL, BOB, DeviceKind::Client, By::Key(CAROL_MASTER)),
    (BOB_BY_ALICE, BOB, DeviceKind::Client, By::Key(ALICE_MASTER)),
    (
        CAROL_LAPTOP,
        CAROL,
        DeviceKind::Client,
        By::Key(CAROL_MASTER),
    ),
    (DAVE_LAPTOP, DAVE, DeviceKind::Client, By::Key(DAVE_MASTER)),
    (DAVE_CI, DAVE, DeviceKind::Service, By::Device(DAVE_LAPTOP)),
    (DAVE_BY_BOB, DAVE, DeviceKind::Client, By::Key(BOB_MASTER)),
    (
        DAVE_BY_BOB_CI,
        DAVE,
        DeviceKind::Service,
        By::Device(DAVE_BY_BOB),
    ),
];

/// Every device's certificate, as the server returns them.
pub fn certs() -> Vec<Cert> {
    DEVICES
        .iter()
        .map(|&(seed, account, kind, by)| {
            let (issuer, signer) = match by {
                By::Key(master) => (None, master),
                By::Device(issuer) => (Some(device(issuer)), issuer),
            };
            let body = CertBody {
                account,
                device: device(seed),
                kind,
                signing: public(seed),
                hpke: [seed; 32],
                issuer,
                issued: TIMESTAMP,
                ssh: None,
                name: format!("Device {seed}"),
                unknown: Unknown::default(),
            };
            issue(&body, &key(signer), &key(seed)).unwrap()
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Member {
    pub account: AccountId,
    pub handle: String,
    pub role: u64,
    pub master: [u8; 32],
    pub invite: Option<[u8; 16]>,
    /// Empty: the field is left out.
    pub excluded: Vec<DeviceId>,
    /// Fields of later versions.
    pub extra: Vec<(i64, Value)>,
}

#[derive(Debug, Clone)]
pub struct ServiceAgent {
    pub device: DeviceId,
    pub owner: String,
    pub agent: String,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub id: ChannelId,
    pub members: Vec<(AccountId, bool)>,
    pub services: Vec<DeviceId>,
}

/// A revision's body, field by field.
#[derive(Debug, Clone)]
pub struct Rev {
    pub space: SpaceId,
    pub name: String,
    pub revision: u64,
    pub prev: Option<[u8; 32]>,
    /// The seed of the `signer` device.
    pub signer: u8,
    pub timestamp: u64,
    pub members: Vec<Member>,
    pub service_agents: Vec<ServiceAgent>,
    pub channels: Vec<Channel>,
    pub ceilings: [u64; 2],
    pub history: u64,
    /// Fields of later versions, in the body.
    pub extra: Vec<(i64, Value)>,
    /// Fields of later versions, in the policy.
    pub policy_extra: Vec<(i64, Value)>,
}

pub fn member(account: AccountId, handle: &str, role: u64, master: u8) -> Member {
    Member {
        account,
        handle: handle.to_owned(),
        role,
        master: public(master),
        invite: None,
        excluded: Vec::new(),
        extra: Vec::new(),
    }
}

pub fn alice() -> Member {
    member(ALICE, "alice", ADMIN, ALICE_MASTER)
}

pub fn bob() -> Member {
    Member {
        invite: Some(BOB_INVITE),
        ..member(BOB, "bob", ADMIN, BOB_MASTER)
    }
}

pub fn carol() -> Member {
    Member {
        invite: Some(CAROL_INVITE),
        ..member(CAROL, "carol", MEMBER, CAROL_MASTER)
    }
}

pub fn service_agent(seed: u8, owner: &str, agent: &str) -> ServiceAgent {
    ServiceAgent {
        device: device(seed),
        owner: owner.to_owned(),
        agent: agent.to_owned(),
    }
}

/// The client's last accepted revision, n = 2.
pub fn current() -> Rev {
    Rev {
        space: SPACE,
        name: "acme".to_owned(),
        revision: 2,
        prev: Some([0x77; 32]),
        signer: ALICE_LAPTOP,
        timestamp: TIMESTAMP,
        members: vec![alice(), bob(), carol()],
        service_agents: vec![service_agent(BOB_CI, "ci", "deploy-bot")],
        channels: vec![Channel {
            id: CHANNEL,
            members: vec![(ALICE, true), (CAROL, false)],
            services: vec![device(BOB_CI)],
        }],
        ceilings: [READ_ONLY, FULL],
        history: THIRTY_DAYS,
        extra: Vec::new(),
        policy_extra: Vec::new(),
    }
}

/// A valid revision n+1 after [`current`] by alice's laptop: it adds a channel.
pub fn next() -> Rev {
    let mut next = successor(&current());
    next.channels.push(Channel {
        id: SECOND_CHANNEL,
        members: vec![(BOB, true)],
        services: Vec::new(),
    });
    next
}

/// Revision 1 of `acme` as alice's laptop builds it for a new space: alice its only member and
/// admin, no service agents or channels, and a new space's policy.
pub fn revision_one() -> Rev {
    Rev {
        revision: 1,
        prev: None,
        members: vec![alice()],
        service_agents: Vec::new(),
        channels: Vec::new(),
        ceilings: [READ_ONLY, READ_ONLY],
        ..current()
    }
}

/// [`revision_one`], built by the crate under test.
pub fn first_body() -> RosterBody {
    let name: Name = "acme".parse().unwrap();
    let handle: Name = "alice".parse().unwrap();
    RosterBody::first(
        SPACE,
        name,
        ALICE,
        handle,
        public(ALICE_MASTER),
        device(ALICE_LAPTOP),
        TIMESTAMP,
    )
}

/// Asserts that `roster` holds the encoded body of `rev` as it is, signed by alice's laptop over
/// `["skald roster", body]`.
pub fn assert_holds(roster: &Roster, rev: &Rev) {
    assert_eq!(roster.body_bytes(), rev.body().as_slice());
    assert_eq!(
        roster.signature(),
        &signature(ALICE_LAPTOP, "skald roster", &rev.body())
    );
    assert_eq!(roster.to_cbor(), rev.bytes_signed_with(ALICE_LAPTOP));
}

/// `rev` as the next revision after it: the same content, `revision` one more and `prev` its hash.
pub fn successor(rev: &Rev) -> Rev {
    Rev {
        revision: rev.revision + 1,
        prev: Some(rev.roster().hash()),
        timestamp: rev.timestamp + 60,
        ..rev.clone()
    }
}

impl Rev {
    pub fn value(&self) -> Value {
        let mut fields = vec![
            (1, bytes(self.space.as_bytes())),
            (2, text(&self.name)),
            (3, Value::Uint(self.revision)),
            (5, bytes(device(self.signer).as_bytes())),
            (6, Value::Uint(self.timestamp)),
            (
                7,
                Value::Array(self.members.iter().map(Member::value).collect()),
            ),
            (
                8,
                Value::Array(
                    self.service_agents
                        .iter()
                        .map(ServiceAgent::value)
                        .collect(),
                ),
            ),
            (
                9,
                Value::Array(self.channels.iter().map(Channel::value).collect()),
            ),
            (10, self.policy()),
        ];
        fields.extend(self.prev.map(|prev| (4, bytes(&prev))));
        fields.extend(self.extra.iter().cloned());
        Value::Map(fields)
    }

    pub fn policy(&self) -> Value {
        let mut fields = vec![
            (
                1,
                Value::Array(
                    self.ceilings
                        .iter()
                        .map(|&mode| Value::Uint(mode))
                        .collect(),
                ),
            ),
            (2, Value::Uint(self.history)),
        ];
        fields.extend(self.policy_extra.iter().cloned());
        Value::Map(fields)
    }

    /// The encoded body.
    pub fn body(&self) -> Vec<u8> {
        self.value().encode()
    }

    /// The encoded roster: the body signed by `seed`'s key, whatever its `signer` says.
    pub fn bytes_signed_with(&self, seed: u8) -> Vec<u8> {
        let body = self.body();
        let signature = signature(seed, "skald roster", &body);
        Value::Array(vec![bytes(&body), bytes(&signature)]).encode()
    }

    /// The encoded roster, signed by its `signer`.
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes_signed_with(self.signer)
    }

    pub fn signed_with(&self, seed: u8) -> Roster {
        Roster::from_cbor(&self.bytes_signed_with(seed)).unwrap()
    }

    /// The roster, signed by its `signer`.
    pub fn roster(&self) -> Roster {
        self.signed_with(self.signer)
    }
}

impl Member {
    pub fn value(&self) -> Value {
        let mut fields = vec![
            (1, bytes(self.account.as_bytes())),
            (2, text(&self.handle)),
            (3, Value::Uint(self.role)),
            (4, bytes(&self.master)),
        ];
        fields.extend(self.invite.map(|invite| (5, bytes(&invite))));
        if !self.excluded.is_empty() {
            let excluded = self.excluded.iter().map(|id| bytes(id.as_bytes()));
            fields.push((6, Value::Array(excluded.collect())));
        }
        fields.extend(self.extra.iter().cloned());
        Value::Map(fields)
    }
}

impl ServiceAgent {
    pub fn value(&self) -> Value {
        Value::Map(vec![
            (1, bytes(self.device.as_bytes())),
            (2, text(&self.owner)),
            (3, text(&self.agent)),
        ])
    }
}

impl Channel {
    pub fn value(&self) -> Value {
        let members = self.members.iter().map(|(account, history)| {
            Value::Array(vec![bytes(account.as_bytes()), Value::Bool(*history)])
        });
        let services = self.services.iter().map(|id| bytes(id.as_bytes()));
        Value::Map(vec![
            (1, bytes(self.id.as_bytes())),
            (2, Value::Array(members.collect())),
            (3, Value::Array(services.collect())),
        ])
    }
}

/// Whether a client of alice's account, alice's master key pinned, accepts `next` after
/// `previous`: checks 1 to 4 of "Roster".
pub fn accepts_after(previous: &Rev, next: &Roster) -> Result<(), String> {
    accept_next(
        &previous.roster(),
        next,
        &certs(),
        ALICE,
        &public(ALICE_MASTER),
    )
    .map_err(|error| format!("{error:?}"))
}

/// [`accepts_after`] [`current`].
pub fn accepts(next: &Roster) -> Result<(), String> {
    accepts_after(&current(), next)
}

/// Whether the client accepts the encoded `next` after [`current`]: it decodes, and the checks
/// pass. A revision the spec's CDDL forbids may fail either way.
pub fn accepts_bytes(next: &[u8]) -> Result<(), String> {
    let roster = Roster::from_cbor(next).map_err(|error| format!("{error:?}"))?;
    accepts(&roster)
}

/// Whether the encoded `roster` decodes and passes checks 3 and 4 of "Roster" on its own.
pub fn passes_content_checks(roster: &[u8]) -> Result<(), String> {
    let roster = Roster::from_cbor(roster).map_err(|error| format!("{error:?}"))?;
    check_roster(&roster, &certs()).map_err(|error| format!("{error:?}"))
}
