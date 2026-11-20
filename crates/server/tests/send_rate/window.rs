//! The window: once a device has had the setting's number of messages stored within it, its next
//! messages are refused with `rate-limited` and stored nowhere, however long they wait within it;
//! once it has passed, the device's next message is stored, and the device is limited again. Unset,
//! the window is a minute.

use std::thread;
use std::time::{Duration, Instant};

use crate::rate::{
    RATE, WINDOW, assert_over_rate, assert_within, message_to, stores, stores_messages, wait_past,
    with_alice,
};
use crate::wire::envelopes_of;

/// The window the tests set, in seconds.
const SHORT: u64 = 10;

/// With the setting at 3 and a window of [`SHORT`] seconds, alice's laptop stores 3 messages to
/// bob; her next ones, to bob and to carol, are `rate-limited`, and so is one sent halfway through
/// the window. Once the window has passed since her third, her next message is stored, and so are
/// 2 more; the one after them is `rate-limited` again. Bob's and carol's mailboxes hold only the
/// stored ones.
#[test]
fn a_device_s_messages_over_the_rate_are_refused_until_its_window_passes() {
    let short = SHORT.to_string();
    let (fixture, mut alice) = with_alice(3_216_060_101, &[RATE, "3", WINDOW, &short]);
    let window = Duration::from_secs(SHORT);
    let db = fixture.db();
    let [_, bob, carol, ..] = fixture.laptops();

    let first = Instant::now();
    let mut stored = stores_messages(&fixture, &mut alice, 3, &[bob]);
    let last = Instant::now();
    assert_over_rate(
        db,
        &mut alice,
        &message_to(&fixture, &[bob]),
        "the 4th, to bob",
    );
    assert_over_rate(
        db,
        &mut alice,
        &message_to(&fixture, &[carol]),
        "the 5th, to carol",
    );
    assert_within(first, window, "the refusals");

    thread::sleep(window / 2 - first.elapsed().min(window / 2));
    let halfway = message_to(&fixture, &[bob]);
    assert_over_rate(db, &mut alice, &halfway, "halfway through the window");
    assert_within(first, window, "the refusal halfway");

    wait_past(last, window);
    let again = Instant::now();
    let next = message_to(&fixture, &[bob]);
    stored.extend(stores(&mut alice, &next));
    stored.extend(stores_messages(&fixture, &mut alice, 2, &[bob]));
    assert_over_rate(
        db,
        &mut alice,
        &message_to(&fixture, &[bob]),
        "the one after them",
    );
    assert_within(again, window, "the second window's refusal");

    assert_eq!(envelopes_of(db, bob), stored, "bob's mailbox");
    assert_eq!(envelopes_of(db, carol), Vec::<Vec<u8>>::new(), "carol's");
}

/// With the setting at 2 and no window set, alice's laptop stores 2 messages; her next is
/// `rate-limited`, and so is one 50 seconds after the first. A minute after her second, her next
/// message is stored.
#[test]
fn unset_the_window_is_a_minute() {
    let (fixture, mut alice) = with_alice(3_216_060_111, &[RATE, "2"]);
    let minute = Duration::from_secs(60);
    let db = fixture.db();
    let [_, bob, ..] = fixture.laptops();

    let first = Instant::now();
    let mut stored = stores_messages(&fixture, &mut alice, 2, &[bob]);
    let last = Instant::now();
    assert_over_rate(db, &mut alice, &message_to(&fixture, &[bob]), "the 3rd");

    thread::sleep(Duration::from_secs(50).saturating_sub(first.elapsed()));
    assert_over_rate(db, &mut alice, &message_to(&fixture, &[bob]), "50 s on");
    assert_within(first, minute, "the refusal 50 s on");

    wait_past(last, minute);
    let next = message_to(&fixture, &[bob]);
    stored.extend(stores(&mut alice, &next));
    assert_eq!(envelopes_of(db, bob), stored, "bob's mailbox");
}
