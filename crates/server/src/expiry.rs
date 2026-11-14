//! Expiry of mailbox entries: `docs/protocol/delivery.md`, "Mailbox". A `message` expires at its
//! storing time plus its envelope's `ttl`, capped by the server's mailbox retention, and a `status`
//! at its adding time plus the retention; a `message` of a version not deliverable on a connection
//! expires when the connection reaches it. The server looks for expired entries by itself, deletes
//! them and adds `expired` for each message sent with `report` true; it forgets an idempotency key
//! once the retention has passed since its send.

use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use skald_proto::{DeviceId, ServerStatus};
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;

use crate::delivery::{Wakers, remove, removing};

/// How often the server looks for expired entries.
const SWEEP: Duration = Duration::from_secs(5);
/// How many expired entries the server removes at a time.
const PAGE: i64 = 100;
/// How long the server waits between two pages of expired entries.
const PAUSE: Duration = Duration::from_millis(10);

/// The mailbox retention `retention` in seconds, as the queries take it; `None` without one.
pub(crate) fn seconds(retention: Option<Duration>) -> Option<f64> {
    retention.map(|retention| retention.as_secs_f64())
}

/// Whether an envelope of the protocol version `v` is delivered on a connection of `version`: `v`
/// is that version or adjacent to it (`docs/protocol/README.md`, "Protocol version").
pub(crate) fn deliverable(v: NonZeroU64, version: NonZeroU64) -> bool {
    v.get().abs_diff(version.get()) <= 1
}

/// Looks for expired entries every [`SWEEP`], for good, on a server with the mailbox retention
/// `retention`; wakes the sending devices' connections in `wakers`. A failure goes to the server's
/// output, and the next look tries again.
pub(crate) async fn run(db: PgPool, wakers: Arc<Wakers>, retention: Option<Duration>) {
    let mut looks = tokio::time::interval(SWEEP);
    looks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        looks.tick().await;
        if let Err(err) = sweep(&db, &wakers, seconds(retention)).await {
            eprintln!("skald-server: cannot expire mailbox entries: {err}");
        }
    }
}

/// Removes every entry expired by time, with the retention of `retention` seconds, then the
/// idempotency keys stored longer ago than that.
async fn sweep(db: &PgPool, wakers: &Wakers, retention: Option<f64>) -> Result<(), sqlx::Error> {
    // Entries that an ack or another sweep holds are left to it.
    let sql = removing!(
        "DELETE FROM entries e
         WHERE (e.device_id, e.seq) IN (
             SELECT device_id, seq FROM entries
             WHERE ttl_ends_at <= now() OR added_at <= now() - make_interval(secs => $1)
             LIMIT $2 FOR UPDATE SKIP LOCKED
         )
         RETURNING e.device_id, e.seq, e.envelope_id,
             least(e.ttl_ends_at, e.added_at + make_interval(secs => $1)) AS at"
    );
    loop {
        let query = sqlx::query_as(sql).bind(retention).bind(PAGE);
        if remove(db, wakers, query, ServerStatus::Expired).await? == 0 {
            break;
        }
        // Leaves the database to other work between pages.
        tokio::time::sleep(PAUSE).await;
    }
    sqlx::query("DELETE FROM sends WHERE answered_at <= now() - make_interval(secs => $1)")
        .bind(retention)
        .execute(db)
        .await?;
    Ok(())
}

/// Expires the entry `seq` of `device`'s mailbox now, unless it is gone already; wakes the sending
/// device's connections in `wakers`.
pub(crate) async fn expire(
    db: &PgPool,
    wakers: &Wakers,
    device: DeviceId,
    seq: u64,
) -> Result<(), sqlx::Error> {
    let query = sqlx::query_as(removing!(
        "DELETE FROM entries WHERE device_id = $1::uuid AND seq = $2
         RETURNING device_id, seq, envelope_id, now() AS at"
    ))
    .bind(device.to_string())
    .bind(i64::try_from(seq).expect("a stored seq"));
    remove(db, wakers, query, ServerStatus::Expired).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(version: u64) -> NonZeroU64 {
        NonZeroU64::new(version).unwrap()
    }

    #[test]
    fn an_envelope_is_delivered_on_its_version_and_the_adjacent_ones_only() {
        assert!(deliverable(v(2), v(2)));
        assert!(deliverable(v(2), v(1)));
        assert!(deliverable(v(2), v(3)));
        assert!(!deliverable(v(1), v(3)));
        assert!(!deliverable(v(4), v(2)));
    }
}
