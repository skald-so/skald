//! Roster revisions as an admin's device builds them: bodies from the `skald-proto` types, signed
//! over `["skald roster", body]` and encoded as `[body, signature]` here, so that a test can sign
//! any bytes, also a body that breaks the spec.

use sha2::{Digest, Sha256};
use skald_crypto::SigningKey;
use skald_proto::{
    Cbor, Channel, ChannelId, ChannelMember, Member, Name, ROSTER_LABEL, Role, Roster, RosterBody,
    ServiceAgent, SpaceId, Unknown,
};

use crate::api::{ACME, Person, Team, assert_refused};
use crate::common::devices::{Device, assert_taken};

/// The `timestamp` of the tests' revisions.
pub const TIMESTAMP: u64 = 1_794_000_000;

/// `text` as a name; it must be a valid one.
pub fn name(text: &str) -> Name {
    text.parse().expect("a valid name")
}

/// `person` as a member of `role`, with its handle and master key.
pub fn member(person: &Person, role: Role) -> Member {
    Member {
        account: person.account,
        handle: person.handle.clone(),
        role,
        master: person.master.public(),
        invite: None,
        excluded: None,
        unknown: Unknown::default(),
    }
}

/// Revision 1 of the space `space` named `space_name`, which `creator` signs with its laptop: the
/// creator its only member and admin.
pub fn first(space: SpaceId, space_name: &str, creator: &Person) -> RosterBody {
    RosterBody::first(
        space,
        name(space_name),
        creator.account,
        creator.handle.clone(),
        creator.master.public(),
        creator.laptop.id,
        TIMESTAMP,
    )
}

/// The body of the revision after `previous`, a revision's bytes, unchanged but for `revision`,
/// `prev` and `signer`.
pub fn next(previous: &[u8], signer: &Device) -> RosterBody {
    let roster = Roster::from_cbor(previous).expect("a revision that decodes");
    let mut body = roster.body().clone();
    body.revision += 1;
    body.prev = Some(Sha256::digest(previous).into());
    body.signer = signer.id;
    body
}

/// A channel with `members` and no services.
pub fn channel(id: ChannelId, members: &[&Person]) -> Channel {
    Channel {
        id,
        members: members
            .iter()
            .map(|person| ChannelMember {
                account: person.account,
                history: false,
            })
            .collect(),
        services: Vec::new(),
        unknown: Unknown::default(),
    }
}

/// A service agent `ci/deploy-bot` on `device`.
pub fn service_agent(device: &Device) -> ServiceAgent {
    ServiceAgent {
        device: device.id,
        owner: name("ci"),
        agent: name("deploy-bot"),
        unknown: Unknown::default(),
    }
}

/// `body` encoded and signed by `key`.
pub fn signed(body: &RosterBody, key: &SigningKey) -> Vec<u8> {
    signed_bytes(&body.to_cbor(), key)
}

/// The body bytes `body` signed by `key`.
pub fn signed_bytes(body: &[u8], key: &SigningKey) -> Vec<u8> {
    encoded(body, &key.sign(ROSTER_LABEL, &[body]))
}

/// The roster `[body, signature]`, encoded.
pub fn encoded(body: &[u8], signature: &[u8; 64]) -> Vec<u8> {
    let mut roster = vec![0x82];
    roster.extend(byte_string(body));
    roster.extend(byte_string(signature));
    roster
}

/// `bytes` as a CBOR byte string, its length in the shortest form.
fn byte_string(bytes: &[u8]) -> Vec<u8> {
    let len = bytes.len();
    let mut out = match len {
        0..=23 => vec![0x40 | len as u8],
        24..=0xff => vec![0x58, len as u8],
        0x100..=0xffff => vec![0x59, (len >> 8) as u8, len as u8],
        _ => panic!("a body of {len} bytes"),
    };
    out.extend_from_slice(bytes);
    out
}

/// `body`, an encoded roster body, with a field of a later version added: key 11, the text
/// `later`. Its keys stay in order, as the deterministic encoding has them.
pub fn with_later_field(body: &[u8]) -> Vec<u8> {
    let fields = body[0];
    assert!((0xa1..0xb7).contains(&fields), "a map of up to 22 fields");
    let mut changed = vec![fields + 1];
    changed.extend_from_slice(&body[1..]);
    changed.extend_from_slice(&[0x0b, 0x65]);
    changed.extend_from_slice(b"later");
    changed
}

impl Person {
    /// `body` signed by this person's laptop.
    pub fn sign(&self, body: &RosterBody) -> Vec<u8> {
        signed(body, &self.laptop.signing.key)
    }
}

impl Team {
    /// Stores revision 1 of `acme` from alice's session, and revision 2, which adds bob as an admin
    /// and carol as a member, both signed by alice's laptop; gives them.
    pub fn with_members(&self) -> [Vec<u8>; 2] {
        let first = self.alice.sign(&first(self.space, ACME, &self.alice));
        self.stored_by(&self.alice, &first);
        let mut body = next(&first, &self.alice.laptop);
        body.members.push(member(&self.bob, Role::Admin));
        body.members.push(member(&self.carol, Role::Member));
        let second = self.alice.sign(&body);
        self.stored_by(&self.alice, &second);
        [first, second]
    }

    /// Stores `revision` from `person`'s session, which the server must take.
    pub fn stored_by(&self, person: &Person, revision: &[u8]) {
        assert_taken(&person.session.store(self.space, revision));
    }

    /// Checks that the server refuses `revision` from `person`'s session and keeps `stored` as the
    /// space's revisions; `what` names the case.
    pub fn refused(&self, person: &Person, revision: &[u8], stored: &[Vec<u8>], what: &str) {
        assert_refused(&person.session.store(self.space, revision), what);
        assert_eq!(
            self.alice.session.revisions(self.space),
            stored,
            "{what}: stored revisions"
        );
    }
}
