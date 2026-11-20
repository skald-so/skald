//! What the tests of the send rate limit share: the settings and the problem, messages of one
//! `msg_id` to several devices, the server's answers to them, and waiting for a window to pass; the
//! fixture, the answers and a second device of the tests of the cloud limits and of delivering a
//! mailbox.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{DeviceId, Header, MessageId};

use crate::common::TestDb;
use crate::fixture::Fixture;
pub use crate::limits::{answer_to, with_alice};
use crate::limits::{assert_refused_with, assert_stored};
pub use crate::listener::second_device;
use crate::wire::{Answer, Socket, envelope};

/// The settings, as arguments and as environment variables.
pub const RATE: &str = "--send-rate-limit";
pub const RATE_ENV: &str = "SKALD_SEND_RATE_LIMIT";
pub const WINDOW: &str = "--send-rate-window";
pub const WINDOW_ENV: &str = "SKALD_SEND_RATE_WINDOW";

/// The problem of `send-refused` over the rate.
pub const RATE_LIMITED: &str = "rate-limited";

/// How long past a window a test waits before it counts the window as passed.
pub const SLACK: Duration = Duration::from_secs(1);

/// The headers of one message, one new `msg_id`, to each of `recipients` in the fixture's channel.
pub fn message_to(fixture: &Fixture, recipients: &[DeviceId]) -> Vec<Header> {
    let mut headers: Vec<Header> = recipients.iter().map(|&to| fixture.to(to)).collect();
    let msg_id = MessageId::generate();
    for header in &mut headers {
        header.msg_id = msg_id;
    }
    headers
}

/// Sends each envelope of the message of `headers` on `socket` and checks that the server stores
/// it; gives the envelopes.
#[track_caller]
pub fn stores(socket: &mut Socket, headers: &[Header]) -> Vec<Vec<u8>> {
    let sent: Vec<Vec<u8>> = headers.iter().map(envelope).collect();
    for (header, sent) in headers.iter().zip(&sent) {
        assert_stored(socket, header, sent);
    }
    sent
}

/// Stores `count` messages on `socket`, each to `recipients`; gives their envelopes in order.
#[track_caller]
pub fn stores_messages(
    fixture: &Fixture,
    socket: &mut Socket,
    count: usize,
    recipients: &[DeviceId],
) -> Vec<Vec<u8>> {
    (0..count)
        .flat_map(|_| stores(socket, &message_to(fixture, recipients)))
        .collect()
}

/// Checks that `answer` refuses its envelope with `rate-limited`.
#[track_caller]
pub fn assert_rate_limited(answer: &Answer, what: &str) {
    match answer {
        Answer::Refused(_, problem) => assert_eq!(problem, RATE_LIMITED, "{what}: the problem"),
        stored => panic!("{what}: send-refused with {RATE_LIMITED}, not {stored:?}"),
    }
}

/// Sends each envelope of the message of `headers` on `socket` and checks that the server refuses
/// it with `rate-limited` and stores nothing of it in `db`.
#[track_caller]
pub fn assert_over_rate(db: &TestDb, socket: &mut Socket, headers: &[Header], what: &str) {
    for header in headers {
        let sent = envelope(header);
        assert_refused_with(db, socket, header, &sent, RATE_LIMITED, what);
    }
}

/// Sleeps until `window` and [`SLACK`] have passed since `since`.
pub fn wait_past(since: Instant, window: Duration) {
    let until = since + window + SLACK;
    let now = Instant::now();
    if until > now {
        thread::sleep(until - now);
    }
}

/// Checks that less than `window` has passed since `since`, so that what a test sent since then
/// falls in one window; `what` names the step.
#[track_caller]
pub fn assert_within(since: Instant, window: Duration, what: &str) {
    let took = since.elapsed();
    assert!(
        took < window,
        "{what}: the test took {took:?}, past the window of {window:?}; it cannot tell"
    );
}
