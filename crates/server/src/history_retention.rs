//! History retention: `docs/protocol/membership.md`, "Uploads". The server looks by itself for
//! entries stored longer ago than the retention and deletes them. A channel's cutoff is the latest
//! epoch of an entry the server has deleted, and never moves back; the server deletes the links and
//! wraps of epochs before it, and [`crate::history`] refuses them.

use std::time::Duration;

use skald_proto::{Cbor as _, HistoryEntry};
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;

use crate::error_reports;
use crate::history::epoch;

/// How often the server looks for entries past the retention.
const SWEEP: Duration = Duration::from_secs(5);
/// How many entries the server deletes at a time.
const PAGE: i64 = 100;
/// How long the server waits between two pages.
const PAUSE: Duration = Duration::from_millis(10);

/// Looks for entries past `retention` every [`SWEEP`], for good. A failure goes to the server's
/// output and its error reports, and the next look tries again.
pub(crate) async fn run(db: PgPool, retention: Duration) {
    let mut looks = tokio::time::interval(SWEEP);
    looks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        looks.tick().await;
        if let Err(err) = sweep(&db, retention.as_secs_f64()).await {
            error_reports::background_failed("delete history past the retention", &err);
        }
    }
}

/// Deletes every entry stored longer ago than `retention` seconds, a page of one channel's at a
/// time, with the links and wraps their epochs leave behind; looks again after each page.
async fn sweep(db: &PgPool, retention: f64) -> Result<(), sqlx::Error> {
    loop {
        let due: Option<(String,)> = sqlx::query_as(
            "SELECT channel_id::text FROM history_entries
             WHERE stored_at <= now() - make_interval(secs => $1)
             ORDER BY stored_at LIMIT 1",
        )
        .bind(retention)
        .fetch_optional(db)
        .await?;
        let Some((channel,)) = due else {
            return Ok(());
        };
        delete_page(db, &channel, retention).await?;
        // Leaves the database to other work between pages.
        tokio::time::sleep(PAUSE).await;
    }
}

/// Deletes up to [`PAGE`] entries of `channel` stored longer ago than `retention` seconds; then
/// moves the channel's cutoff up to the latest epoch among them, if it is below, and deletes the
/// links and wraps of epochs before that epoch.
async fn delete_page(db: &PgPool, channel: &str, retention: f64) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    // The group's row lock, which an upload takes before it stores, makes the upload wait for the
    // new cutoff or the deletion for the upload's link or wrap.
    sqlx::query("SELECT 1 FROM channel_groups WHERE channel_id = $1::uuid FOR NO KEY UPDATE")
        .bind(channel)
        .execute(&mut *tx)
        .await?;
    let deleted: Vec<(Vec<u8>,)> = sqlx::query_as(
        "DELETE FROM history_entries
         WHERE (sender_id, msg_id) IN (
             SELECT sender_id, msg_id FROM history_entries
             WHERE channel_id = $1::uuid AND stored_at <= now() - make_interval(secs => $2)
             LIMIT $3
         )
         RETURNING entry",
    )
    .bind(channel)
    .bind(retention)
    .bind(PAGE)
    .fetch_all(&mut *tx)
    .await?;
    // The server decoded each entry when it took it.
    let latest = (deleted.iter())
        .filter_map(|(bytes,)| HistoryEntry::from_cbor(bytes).ok())
        .map(|entry| entry.epoch)
        .max();
    if let Some(latest) = latest {
        sqlx::query(
            "UPDATE channel_groups SET history_cutoff = greatest(history_cutoff, $2)
             WHERE channel_id = $1::uuid",
        )
        .bind(channel)
        .bind(epoch(latest))
        .execute(&mut *tx)
        .await?;
        for earlier in [
            "DELETE FROM history_links WHERE channel_id = $1::uuid AND epoch < $2",
            "DELETE FROM history_wraps WHERE channel_id = $1::uuid AND epoch < $2",
        ] {
            sqlx::query(earlier)
                .bind(channel)
                .bind(epoch(latest))
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await
}
