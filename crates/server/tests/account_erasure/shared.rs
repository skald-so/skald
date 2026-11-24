//! The scenario most tests share: alice, who created `acme`, writes in its channel `general`, has
//! wraps to her recovery key, an anchors object, a personal space and a revoked second device, then
//! deletes her account once bob is an admin too.

use std::collections::BTreeMap;

use skald_proto::{Role, SpaceId};

use crate::api::Person;
use crate::common::TestDb;
use crate::common::devices::{Device, devices_of};
use crate::deletion::{deletes, personal_space, with_role};
use crate::erasure::{ERASED_AFTER, Kept, Traces, move_deletion_back};
use crate::fixture::Fixture;
use crate::history::{ENTRIES, Item, entry, formed, given, stores, wrap};
use crate::listener::second_device;
use crate::objects::assert_stored;
use crate::rosters::next;
use crate::scene::{Peer, Scene, removing};
use crate::sessions::Api;
use crate::wire::mailbox;

/// alice stores an anchors object of her laptop, creates her personal space and revokes a second
/// device of hers, `Phone`, by her master key, in `f`; a revision alice's laptop signs makes bob
/// an admin of `acme`. Gives the phone and its revocation.
pub fn alices_records(f: &mut Fixture) -> (Device, Vec<u8>) {
    let t = &f.team;
    let alice = &t.alice;
    assert_stored(
        &alice.session,
        &alice.anchors_of(&alice.laptop, 1),
        "alice's anchors",
    );
    personal_space(alice);
    let phone = second_device(f.db(), alice, "Phone");
    let revocation = alice.by_master(phone.id, &[]);
    alice.revoke(&revocation);
    let mut body = next(f.revisions.last().expect("a revision"), &alice.laptop);
    with_role(&mut body, &t.bob, Role::Admin);
    let revision = alice.sign(&body);
    t.stored_by(alice, &revision);
    f.revisions.push(revision);
    (phone, revocation)
}

/// The scenario: alice's laptop forms `general` with bob's and carol's as in the tests of a
/// channel's history, and uploads her entry of epoch 1 and a wrap of epoch 1 to her recovery key.
/// If `removed`, bob's laptop then commits the Remove of hers, which carol's processes. bob's
/// laptop uploads a wrap of its epoch to alice's recovery key, and bob's and carol's laptops ack
/// what their mailboxes hold, as their clients do once they have it. alice keeps the records of
/// [`alices_records`]. Unless her laptop was removed, a row may still hold her devices' IDs, as the
/// channel's group state does.
pub struct Shared {
    pub s: Scene,
    pub phone: Device,
    pub revocation: Vec<u8>,
    /// alice's entry of epoch 1.
    pub entry: Item,
    /// The wraps to alice's recovery key: hers and bob's.
    pub wraps: [Item; 2],
    /// alice's data, as the server gave it before her deletion.
    pub alice: Traces,
    /// What `acme` keeps of her, as bob's session got it before her deletion.
    pub kept: Kept,
    /// `acme`'s revisions and its devices' certificates, as bob's session got them before.
    pub revisions: Vec<Vec<u8>>,
    pub certs: BTreeMap<String, Vec<u8>>,
}

impl Shared {
    /// The scenario, with GitHub user IDs from `github` on; alice's laptop removed from `general`
    /// if `removed`. alice has not deleted her account yet.
    #[track_caller]
    pub fn new(github: u64, removed: bool) -> Self {
        let (mut s, _) = formed(github);
        let general = s.fixture.general;
        let alice = &s.fixture.team.alice;
        let entry = entry(general, 1);
        stores(&mut s.alice, &s.fixture, &entry, "alice's entry");
        let own = wrap(general, 1, alice, alice);
        stores(&mut s.alice, &s.fixture, &own, "alice's wrap");
        if removed {
            let commit = s.bob.commits(&s.fixture, removing(&[s.alice.id]));
            s.carol.process(&s.fixture, &commit);
        }
        let epoch = (s.bob.mls.epoch(general)).expect("bob's epoch");
        let bobs = wrap(general, epoch, &s.fixture.team.alice, &s.fixture.team.bob);
        stores(
            &mut s.bob,
            &s.fixture,
            &bobs,
            "bob's wrap to alice's recovery key",
        );
        for peer in [&mut s.bob, &mut s.carol] {
            acks_all(peer, s.fixture.db());
        }
        let (phone, revocation) = alices_records(&mut s.fixture);
        let t = &s.fixture.team;
        let devices = [t.alice.laptop.id, t.alice.uncertified.id, phone.id];
        let mut alice = Traces::of(&t.alice, github, &devices).with_wraps(&[&own, &bobs]);
        if !removed {
            alice = alice.devices_in_groups();
        }
        let kept = Kept::of_space(&t.bob.session, t.space, &devices).with_entries(&[&entry]);
        let revisions = t.bob.session.revisions(t.space);
        let certs = certified(&t.bob, t.space);
        Shared {
            s,
            phone,
            revocation,
            entry,
            wraps: [own, bobs],
            alice,
            kept,
            revisions,
            certs,
        }
    }

    pub fn f(&self) -> &Fixture {
        &self.s.fixture
    }

    pub fn db(&self) -> &TestDb {
        self.s.fixture.db()
    }

    /// alice deletes her account, and its deletion is moved back 30 days.
    #[track_caller]
    pub fn deletes_long_ago(&self) {
        let t = &self.f().team;
        deletes(&Api::of(&t.flow), &t.alice);
        move_deletion_back(self.db(), t.alice.account, ERASED_AFTER);
    }

    /// The history entries `person`'s session gets of `general`.
    pub fn entries_for(&self, person: &Person) -> Vec<Vec<u8>> {
        given(&person.session, self.f(), ENTRIES)
    }
}

/// `peer` acks every entry its mailbox holds in `db`.
pub fn acks_all(peer: &mut Peer, db: &TestDb) {
    if let Some((last, _)) = mailbox(db, peer.id).last() {
        peer.socket.ack(u64::try_from(*last).expect("a seq from 1"));
    }
}

/// The certified devices of `space` that `person`'s session gets: each one's certificate by its ID.
#[track_caller]
pub fn certified(person: &Person, space: SpaceId) -> BTreeMap<String, Vec<u8>> {
    let listed = devices_of(&person.session.devices_response(space));
    (listed.into_iter())
        .filter_map(|(device, cert)| cert.map(|cert| (device, cert)))
        .collect()
}
