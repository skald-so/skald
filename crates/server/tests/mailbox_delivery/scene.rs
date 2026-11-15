//! The scene of most tests: the fixture of storing sent envelopes, alice's connection, which sends,
//! and bob's laptop, which receives.

use skald_proto::DeviceId;

use crate::api::{Person, Team};
use crate::common::TestDb;
use crate::fixture::Fixture;
use crate::link::{Entry, Link, Sent, store_to};

pub struct Scene {
    pub fixture: Fixture,
    /// alice's connection, which states no cursor until a test does.
    pub alice: Link,
    /// bob's certified laptop.
    pub bob: DeviceId,
}

impl Scene {
    /// The scene; GitHub user IDs from `github` on.
    pub fn new(github: u64) -> Self {
        Self::with(github, &[])
    }

    /// As [`Scene::new`], on a server with the extra arguments `args`.
    pub fn with(github: u64, args: &[&str]) -> Self {
        let fixture = Fixture::with(github, args);
        let alice = Link::of(&fixture.team.flow, &fixture.team.alice);
        let bob = fixture.team.bob.laptop.id;
        Scene {
            fixture,
            alice,
            bob,
        }
    }

    pub fn team(&self) -> &Team {
        &self.fixture.team
    }

    pub fn db(&self) -> &TestDb {
        self.fixture.db()
    }

    /// A new connection of `person`'s certified laptop.
    pub fn link(&self, person: &Person) -> Link {
        Link::of(&self.fixture.team.flow, person)
    }

    /// Closes alice's connection and opens another, which states no cursor until a test does.
    pub fn reconnect_alice(&mut self) {
        self.alice = self.link(&self.fixture.team.alice);
    }

    /// Stores `count` envelopes on alice's connection to bob, with `report`.
    #[track_caller]
    pub fn send_bob(&mut self, count: usize, report: bool) -> Vec<Sent> {
        store_to(&mut self.alice, &self.fixture, self.bob, count, report)
    }

    /// A new connection of `person`'s laptop from `cursor`, whose catch-up sends `expected`.
    #[track_caller]
    pub fn synced(&self, person: &Person, cursor: u64, expected: &[Entry]) -> Link {
        Link::synced(&self.fixture.team.flow, person, cursor, expected)
    }

    /// A new connection of bob's laptop from `cursor`, whose catch-up sends `expected`.
    #[track_caller]
    pub fn bob_from(&self, cursor: u64, expected: &[Entry]) -> Link {
        self.synced(&self.fixture.team.bob, cursor, expected)
    }
}
