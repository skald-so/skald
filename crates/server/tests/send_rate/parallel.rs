//! One device's sends at once, over several connections to two instances of the server on one
//! database: exactly the setting's number of messages is stored, however the sends interleave. Each
//! connection sends to a device of its own, so that no mailbox makes the sends wait for each other.

use std::sync::Barrier;
use std::thread;

use skald_proto::Header;

use crate::rate::{RATE, RATE_LIMITED, answer_to, message_to, second_device, with_alice};
use crate::wire::{Answer, Socket, envelope, envelopes_of};

/// The setting.
const LIMIT: usize = 10;
/// The connections of alice's laptop to each instance.
const CONNECTIONS: usize = 4;
/// The messages sent on each connection.
const EACH: usize = 6;

/// With the setting at [`LIMIT`] on two instances, alice's laptop sends [`EACH`] messages on each of
/// [`CONNECTIONS`] connections to each instance, all connections at once, each to a device of bob's
/// of its own: [`LIMIT`] are stored, the rest are `rate-limited`, and bob's devices' mailboxes hold
/// [`LIMIT`] together.
#[test]
fn one_device_s_sends_at_once_through_two_instances_store_exactly_the_setting() {
    let limit = LIMIT.to_string();
    let (fixture, first) = with_alice(3_216_060_601, &[RATE, &limit]);
    let t = &fixture.team;
    let other = t.flow.stand.instance(&[RATE, &limit], &[]);
    let bobs: Vec<_> = (0..2 * CONNECTIONS)
        .map(|at| second_device(fixture.db(), &t.bob, &format!("bob-{at}")).id)
        .collect();
    let mut sockets = vec![first];
    for at in 1..2 * CONNECTIONS {
        let server = if at < CONNECTIONS {
            &t.flow.stand.server
        } else {
            &other
        };
        sockets.push(Socket::to(server, &t.flow, &t.alice, &t.alice.laptop));
    }
    let sends: Vec<Vec<Header>> = (bobs.iter())
        .map(|&bob| {
            (0..EACH)
                .map(|_| message_to(&fixture, &[bob]).remove(0))
                .collect()
        })
        .collect();

    let together = Barrier::new(sockets.len());
    let answers: Vec<Answer> = thread::scope(|scope| {
        let threads: Vec<_> = (sockets.into_iter().zip(sends))
            .map(|(mut socket, headers)| {
                let together = &together;
                scope.spawn(move || {
                    together.wait();
                    (headers.iter())
                        .map(|header| answer_to(&mut socket, header, &envelope(header)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        (threads.into_iter())
            .flat_map(|sending| sending.join().expect("a sending connection"))
            .collect()
    });

    let stored = (answers.iter())
        .filter(|answer| matches!(answer, Answer::Stored(_)))
        .count();
    assert_eq!(stored, LIMIT, "stored at once: {answers:?}");
    let refused =
        |answer: &Answer| matches!(answer, Answer::Refused(_, problem) if problem == RATE_LIMITED);
    assert_eq!(
        answers.iter().filter(|answer| refused(answer)).count(),
        answers.len() - LIMIT,
        "{RATE_LIMITED} past the rate: {answers:?}"
    );
    let kept: usize = (bobs.iter())
        .map(|&bob| envelopes_of(fixture.db(), bob).len())
        .sum();
    assert_eq!(kept, LIMIT, "bob's devices' mailboxes");
}
