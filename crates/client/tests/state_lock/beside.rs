//! Other runs beside a `skald send` or a `skald recv` that holds the lock: they wait for it, then
//! do their part, and every message is shown once.

use crate::gaps::{bob_in, bob_sends};
use crate::held::{
    LONG, assert_waits, holding, release, send_holding, sent_ids, start, start_send,
};
use crate::kit::{RECV, received};
use crate::machine::Finished;
use crate::talk::{BOB_HOST, Talk, assert_shows_once};
use crate::world::{ADD_RULE, CLIENT_A, RULES, dir_of, path, run};

/// The texts the tests send; none is anywhere else.
const BOB_SAYS: [&str; 2] = [
    "lock test: the staging database is back",
    "lock test: staging smoke tests passed",
];
const ADA_SAYS: &str = "lock test: thanks, promoting staging now";

/// Which run holds the lock while the other waits for it.
#[derive(Clone, Copy)]
enum First {
    Recv,
    Send,
}

/// Bob's device sends two messages to the channel. On Ada's machine, the run `first` says waits
/// for the server with the lock held, while the other of `skald recv` and `skald send` waits for
/// the lock and reaches no server. Once the server answers, both succeed. With Ada's next
/// `skald recv`, the runs showed each of Bob's messages once in all, and Bob's device reads Ada's
/// message, with the counter 1.
fn recv_beside_send(first: First, ids: &[u64], logins: &[&str]) {
    let mut talk = Talk::new(ids, logins, false);
    let (from_bob, channel, _) = bob_in(&talk);
    for (counter, text) in (1..).zip(BOB_SAYS) {
        bob_sends(&mut talk, counter, text);
    }
    let from = talk.front.sent().len();
    let runs = match first {
        First::Recv => {
            let holder = holding(&talk, LONG, &[RECV]).0;
            let mut waiting = start_send(&talk, LONG, ADA_SAYS);
            assert_waits(&talk, &mut waiting);
            vec![holder, waiting]
        }
        First::Send => {
            let holder = send_holding(&talk, ADA_SAYS);
            let mut waiting = start(&talk, LONG, &[RECV]);
            assert_waits(&talk, &mut waiting);
            vec![holder, waiting]
        }
    };

    let finished = release(&talk, runs);

    let next = received(&talk.ada.machine);
    let shown: String = finished
        .iter()
        .chain([&next])
        .map(Finished::shown)
        .collect();
    for text in BOB_SAYS {
        assert_shows_once(&shown, text, (channel, &from_bob, BOB_HOST));
    }
    let sent = sent_ids(&talk, from);
    assert_eq!(sent.len(), 1, "not one message envelope: {sent:?}");
    talk.bob_reads_from_ada(&[(sent[0], 1, ADA_SAYS)]);
}

#[test]
fn send_waiting_beside_recv_loses_nothing_and_each_message_is_shown_once() {
    recv_beside_send(
        First::Recv,
        &[4_145_054_001, 4_145_054_002],
        &["ada-recv-holds-lock", "bob-talks-to-recv"],
    );
}

#[test]
fn recv_waiting_beside_send_loses_nothing_and_shows_each_message_once() {
    recv_beside_send(
        First::Send,
        &[4_145_055_001, 4_145_055_002],
        &["ada-send-holds-lock", "bob-talks-to-send"],
    );
}

/// A `skald send` on Ada's machine waits for the server with the lock held. `skald add-rule`, which
/// changes only the local state, waits for the lock meanwhile; once the send succeeds, it adds the
/// rule, which `skald rules` then lists.
#[test]
fn rule_added_beside_a_send_waits_for_the_lock() {
    let talk = Talk::new(
        &[4_145_056_001, 4_145_056_002],
        &["ada-adds-rule-meanwhile", "bob-idle-in-channel"],
        false,
    );
    let folder = dir_of(&talk.ada.machine, "work/clientA");
    let holder = send_holding(&talk, ADA_SAYS);
    let mut adding = start(&talk, LONG, &[ADD_RULE, path(&folder), CLIENT_A]);

    assert_waits(&talk, &mut adding);
    release(&talk, vec![holder, adding]);

    let listed = run(&talk.ada.machine, &[RULES]);
    assert!(listed.success, "{listed:?}");
    assert!(
        listed.stdout.contains(CLIENT_A),
        "the rule is not listed: {listed:?}"
    );
}
