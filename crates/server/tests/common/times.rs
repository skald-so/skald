//! Time passing as the server's database shows it: the mailbox retention the tests set, and the
//! statement that moves the times the server keeps back. The client's tests share this file.

use std::time::Duration;

/// The server setting of the mailbox retention, in seconds; unset, there is none.
pub const RETENTION: &str = "--mailbox-retention";
/// The environment variable of the same setting.
pub const RETENTION_ENV: &str = "SKALD_MAILBOX_RETENTION";

pub const HOUR: u64 = 3_600;
pub const DAY: u64 = 24 * HOUR;
/// The retention the tests set.
pub const WEEK: u64 = 7 * DAY;
pub const YEAR: u64 = 365 * DAY;

/// How soon the server finds an expired entry by itself, with no connection of its device.
pub const EXPIRY: Duration = Duration::from_secs(30);

/// The tables of envelopes, sends and mailbox entries, whose times expiry reads.
pub const MAILBOX: [&str; 3] = ["envelopes", "sends", "entries"];

/// One statement that moves each `timestamptz` and `date` column of `tables` back by `interval`, a
/// PostgreSQL interval such as `10 hours` or `1 month`, by the calendar in UTC, as if that much time
/// had passed; it fails if they have none. In one transaction, so that the server never sees some
/// times moved and others not, such as an entry it expires before the `expired` it adds is moved
/// back too.
pub fn moved_back(tables: &[&str], interval: &str) -> String {
    let names: Vec<String> = tables.iter().map(|table| format!("'{table}'")).collect();
    let names = names.join(", ");
    format!(
        "DO $$
         DECLARE timed record;
         BEGIN
           FOR timed IN
             SELECT table_name, column_name, data_type FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name IN ({names})
               AND data_type IN ('timestamp with time zone', 'date')
           LOOP
             EXECUTE format(
               CASE timed.data_type
                 WHEN 'date' THEN 'UPDATE %I SET %I = (%I - interval %L)::date'
                 ELSE 'UPDATE %I SET %I = ((%I AT TIME ZONE ''UTC'') - interval %L)
                       AT TIME ZONE ''UTC'''
               END,
               timed.table_name, timed.column_name, timed.column_name, '{interval}');
           END LOOP;
           IF NOT FOUND THEN
             RAISE EXCEPTION 'no times in %', ARRAY[{names}];
           END IF;
         END $$"
    )
}
