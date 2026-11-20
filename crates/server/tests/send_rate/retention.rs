//! A mailbox retention shorter than the window: the server forgets a device's idempotency keys once
//! the retention has passed, but still counts its messages within the window.

use std::thread;
use std::time::{Duration, Instant};

use crate::common::times::RETENTION;
use crate::rate::{
    RATE, WINDOW, assert_over_rate, assert_within, message_to, stores_messages, with_alice,
};
use crate::wire::query;

/// The window the test sets, in seconds.
const SHORT: u64 = 40;
/// How long the test waits at most for the server's sweep to forget the keys.
const SWEPT: Duration = Duration::from_secs(20);

/// With the setting at 2, a window of [`SHORT`] seconds and a mailbox retention of a second, alice's
/// laptop stores 2 messages. Once the server has forgotten their idempotency keys, her next message,
/// within the window, is still `rate-limited`.
#[test]
fn a_retention_shorter_than_the_window_still_counts_the_window() {
    let short = SHORT.to_string();
    let args = [RATE, "2", WINDOW, &short, RETENTION, "1"];
    let (fixture, mut alice) = with_alice(3_216_060_701, &args);
    let window = Duration::from_secs(SHORT);
    let db = fixture.db();
    let [_, bob, ..] = fixture.laptops();

    let first = Instant::now();
    stores_messages(&fixture, &mut alice, 2, &[bob]);
    let keys = || -> i64 {
        query(db, async |conn| {
            sqlx::query_scalar("SELECT count(*) FROM sends")
                .fetch_one(conn)
                .await
        })
    };
    while keys() > 0 {
        assert!(first.elapsed() < SWEPT, "the keys are not forgotten");
        thread::sleep(Duration::from_millis(200));
    }
    // The sweep forgets counted messages right after the keys.
    thread::sleep(Duration::from_secs(1));

    assert_over_rate(
        db,
        &mut alice,
        &message_to(&fixture, &[bob]),
        "past the retention",
    );
    assert_within(first, window, "the refusal");
}
