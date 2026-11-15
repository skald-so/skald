//! Expiry on several instances: an `expired` one instance adds reaches the sender on another at
//! once, each expired entry gives one `expired`, and an ack racing an expiry gives exactly one of
//! `delivered` and `expired`. The fixture's own server has no retention, so it expires nothing.

use std::collections::BTreeMap;

use skald_proto::{DeliveryStatus, DeviceId, Header, MessageId, ServerStatus};

use crate::common::Server;
use crate::fixture::Fixture;
use crate::instances::{
    EXPIRY, HOUR, WAKE, WEEK, assert_status, expiring, instance, move_back, sender, settled, store,
    watch,
};
use crate::link::{Entry, WAIT, messages, seqs};
use crate::watch::Watch;
use crate::wire::Socket;

/// How many messages the tests of several expiring instances send.
const MANY: usize = 20;

/// The fixture with instances that expire entries, alice's connection to the fixture's server,
/// which reads her mailbox, and another, which sends.
struct Scene {
    fixture: Fixture,
    expiring: Vec<Server>,
    alice: Watch,
    sends: Socket,
}

impl Scene {
    /// The scene with `count` expiring instances; GitHub user IDs from `github` on.
    fn new(github: u64, count: usize) -> Self {
        let fixture = Fixture::new(github);
        let expiring = (0..count)
            .map(|_| instance(&fixture, &expiring()))
            .collect();
        let t = &fixture.team;
        let first = &t.flow.stand.server;
        let alice = watch(first, &fixture, &t.alice);
        let sends = sender(first, &fixture, &t.alice);
        Scene {
            fixture,
            expiring,
            alice,
            sends,
        }
    }

    fn bob(&self) -> DeviceId {
        self.fixture.team.bob.laptop.id
    }

    /// Stores `count` envelopes from alice to bob, with `report` true.
    fn send_bob(&mut self, count: usize) -> Vec<(Header, Vec<u8>)> {
        let bob = self.bob();
        (0..count)
            .map(|_| store(&mut self.sends, &self.fixture, bob))
            .collect()
    }

    /// Moves time past the retention.
    fn pass_retention(&self) {
        move_back(self.fixture.db(), WEEK + HOUR);
    }

    /// Checks that alice's mailbox comes to hold one status for each of `sent` and nothing more,
    /// that her connection gets them, and that bob's mailbox is empty; gives each status.
    #[track_caller]
    fn one_status_each(&mut self, sent: &[Header]) -> Vec<DeliveryStatus> {
        let alice = self.fixture.team.alice.laptop.id;
        let added = settled(self.fixture.db(), alice, sent.len(), EXPIRY);
        assert_eq!(added.len(), sent.len(), "one status per entry: {added:?}");
        let entries = self.alice.entries_within(sent.len(), WAIT);
        let mut by_message: BTreeMap<MessageId, Vec<&Entry>> = BTreeMap::new();
        for entry in &entries {
            let Entry::Status(_, status) = entry else {
                panic!("a status, not {entry:?}");
            };
            by_message.entry(status.msg_id).or_default().push(entry);
        }
        let statuses = sent
            .iter()
            .map(|header| {
                let found = by_message.get(&header.msg_id).cloned().unwrap_or_default();
                let [entry @ Entry::Status(_, status)] = found[..] else {
                    panic!("one status for {}: {found:?}", header.msg_id);
                };
                assert_status(entry, status.status, header, self.bob());
                status.clone()
            })
            .collect();
        self.alice.assert_quiet();
        assert_eq!(seqs(self.fixture.db(), self.bob()), Vec::<i64>::new());
        statuses
    }
}

#[test]
fn expired_added_by_one_instance_reaches_the_sender_on_another_at_once() {
    let mut s = Scene::new(3_121_290_061, 1);
    let (header, _) = s.send_bob(1).remove(0);

    s.pass_retention();

    let alice = s.fixture.team.alice.laptop.id;
    assert_eq!(settled(s.fixture.db(), alice, 1, EXPIRY), [1]);
    let expired = s.alice.entry_within(WAKE);
    assert_eq!(
        assert_status(&expired, ServerStatus::Expired, &header, s.bob()),
        1
    );
    s.alice.assert_quiet();
    assert_eq!(seqs(s.fixture.db(), s.bob()), Vec::<i64>::new());
}

#[test]
fn expiry_on_two_instances_adds_one_expired_per_entry() {
    let mut s = Scene::new(3_121_290_071, 2);
    let headers: Vec<Header> = s.send_bob(MANY).into_iter().map(|(h, _)| h).collect();

    s.pass_retention();

    for status in s.one_status_each(&headers) {
        assert_eq!(status.status, ServerStatus::Expired, "{status:?}");
    }
}

#[test]
fn an_ack_racing_an_expiry_gives_exactly_one_of_delivered_and_expired() {
    let mut s = Scene::new(3_121_290_081, 2);
    let sent = s.send_bob(MANY);
    let envelopes: Vec<Vec<u8>> = sent.iter().map(|(_, envelope)| envelope.clone()).collect();
    let t = &s.fixture.team;
    let mut bob = Watch::from(&s.expiring[0], &t.flow, &t.bob, 0, &messages(1, &envelopes));

    // bob has written every entry and acks them as they expire.
    s.pass_retention();
    bob.ack(u64::try_from(MANY).expect("a seq"));

    let headers: Vec<Header> = sent.into_iter().map(|(header, _)| header).collect();
    s.one_status_each(&headers);
    bob.assert_quiet();
}
