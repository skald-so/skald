//! How the server keeps a space's monthly count, in the tables the tests of the monthly limit move
//! back: a stored message counts in the database's current month in UTC, and the sweep forgets a
//! counted message once the mailbox retention has passed since its count, as it does its sends'
//! idempotency keys.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{Header, MessageId};

use crate::common::TestDb;
use crate::limits::{MONTHLY, assert_stored, clear_of_the_month_end, with_alice};
use crate::wire::{envelope, query};

/// The months the space `space` has a count for, each with its count, as text.
fn counts_of(db: &TestDb, space: String) -> Vec<(String, i64)> {
    query(db, async |conn| {
        sqlx::query_as("SELECT month::text, messages FROM message_counts WHERE space_id = $1::uuid")
            .bind(space)
            .fetch_all(conn)
            .await
    })
}

/// Whether the server holds `msg_id` as a counted message.
fn counted(db: &TestDb, msg_id: MessageId) -> bool {
    query(db, async |conn| {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM counted_messages WHERE msg_id = $1::uuid)")
            .bind(msg_id.to_string())
            .fetch_one(conn)
            .await
    })
}

/// Two messages, one of them to two devices, count 2 in the month that starts on the first day of
/// the database's current month in UTC, and in no other. In January a count by year, and on a
/// month's first day a count by day, would start on that day too.
#[test]
fn a_message_counts_in_the_current_month_in_utc() {
    let (fixture, mut alice) = with_alice(3_121_310_301, &[MONTHLY, "5"]);
    let db = fixture.db();
    clear_of_the_month_end(db);
    let [_, bob, carol, ..] = fixture.laptops();
    let msg_id = MessageId::generate();
    for recipient in [bob, carol] {
        let header = Header {
            msg_id,
            ..fixture.to(recipient)
        };
        assert_stored(&mut alice, &header, &envelope(&header));
    }
    let other = fixture.to(bob);
    assert_stored(&mut alice, &other, &envelope(&other));

    let month: String = query(db, async |conn| {
        sqlx::query_scalar("SELECT date_trunc('month', now() AT TIME ZONE 'UTC')::date::text")
            .fetch_one(conn)
            .await
    });
    assert_eq!(counts_of(db, fixture.team.space.to_string()), [(month, 2)]);
}

/// With a retention of 60 seconds, a message counted 61 seconds ago is forgotten by the next sweep,
/// and one counted just now is kept.
#[test]
fn the_sweep_forgets_a_counted_message_after_the_retention() {
    let args = [MONTHLY, "5", "--mailbox-retention", "60"];
    let (fixture, mut alice) = with_alice(3_121_310_311, &args);
    let db = fixture.db();
    let [_, bob, ..] = fixture.laptops();
    let (old, new) = (fixture.to(bob), fixture.to(bob));
    for header in [&old, &new] {
        assert_stored(&mut alice, header, &envelope(header));
    }
    db.execute(&format!(
        "UPDATE counted_messages SET counted_at = counted_at - interval '61 seconds'
         WHERE msg_id = '{}'::uuid",
        old.msg_id
    ));

    // The server sweeps every 5 seconds.
    let start = Instant::now();
    while counted(db, old.msg_id) && start.elapsed() < Duration::from_secs(30) {
        thread::sleep(Duration::from_millis(200));
    }
    assert!(!counted(db, old.msg_id), "kept past the retention");
    assert!(counted(db, new.msg_id), "forgotten within the retention");
}
