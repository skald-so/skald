//! Erasing a deleted account's data: 30 days after an account's deletion ([`crate::deletion`]), the
//! server looks by itself for the account and erases everything of it, whether or not every channel
//! has removed its devices, but for what the spaces it shared need: for each space whose revisions
//! list it, those revisions, the history entries its devices wrote, its devices' certificates and
//! revocations, and that it was deleted. Those go with the last such space. A space it created
//! keeps no creator, so it counts against no account's owned spaces.

use sqlx::{PgConnection, PgPool};

use crate::sweeps::{self, PAGE, PAUSE};

/// How old a deletion is when the server erases the account, as PostgreSQL takes an interval.
const ERASED_AFTER: &str = "30 days";

/// Looks for accounts to erase, as [`sweeps::every`] says.
pub(crate) async fn run(db: PgPool) {
    sweeps::every("erase deleted accounts", || {
        let db = db.clone();
        async move { sweep(&db).await }
    })
    .await;
}

/// Erases every account whose deletion is [`ERASED_AFTER`] old, then deletes what is left of every
/// erased account that no space's revisions list any more, [`PAGE`] at a time.
async fn sweep(db: &PgPool) -> Result<(), sqlx::Error> {
    loop {
        let due: Vec<(String,)> = sqlx::query_as(
            "SELECT id::text FROM accounts
             WHERE deleted_at IS NOT NULL AND NOT erased
                 AND deleted_at <= now() - $1::interval
             ORDER BY deleted_at LIMIT $2",
        )
        .bind(ERASED_AFTER)
        .bind(PAGE)
        .fetch_all(db)
        .await?;
        if due.is_empty() {
            break;
        }
        for (account,) in &due {
            let mut tx = db.begin().await?;
            erase(&mut tx, account).await?;
            tx.commit().await?;
        }
        // Leaves the database to other work between pages.
        tokio::time::sleep(PAUSE).await;
    }
    loop {
        // The account's row goes with its devices and revocations.
        let deleted = sqlx::query(
            "DELETE FROM accounts WHERE id IN (
                 SELECT a.id FROM accounts a
                 WHERE a.erased
                     AND NOT EXISTS (SELECT 1 FROM roster_members m WHERE m.account_id = a.id)
                 LIMIT $1)",
        )
        .bind(PAGE)
        .execute(db)
        .await?;
        if deleted.rows_affected() == 0 {
            return Ok(());
        }
        tokio::time::sleep(PAUSE).await;
    }
}

/// Erases `account`, a deleted one, on `conn`, in its transaction, unless it is erased already: it
/// keeps the account's row with its ID and deletion, and its certified devices' rows with what
/// their certificates hold.
async fn erase(conn: &mut PgConnection, account: &str) -> Result<(), sqlx::Error> {
    let due: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM accounts WHERE id = $1::uuid AND NOT erased FOR UPDATE")
            .bind(account)
            .fetch_optional(&mut *conn)
            .await?;
    if due.is_none() {
        return Ok(());
    }
    let devices: Vec<(String,)> =
        sqlx::query_as("SELECT id::text FROM devices WHERE account_id = $1::uuid")
            .bind(account)
            .fetch_all(&mut *conn)
            .await?;
    let devices: Vec<String> = devices.into_iter().map(|(device,)| device).collect();
    // Each statement takes the account as $1 and its devices as $2. The invites its devices signed
    // and its acceptances expired by now, 30 days at most after the server stored them.
    for statement in [
        // Its devices' messages still waiting, and the statuses of messages to them.
        "DELETE FROM entries
         WHERE recipient_id = ANY($2::text[]::uuid[]) OR envelope_id IN (
             SELECT id FROM envelopes WHERE sender_id = ANY($2::text[]::uuid[]))",
        "DELETE FROM envelopes WHERE sender_id = ANY($2::text[]::uuid[])",
        "DELETE FROM mailboxes WHERE device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM sends
         WHERE sender_id = ANY($2::text[]::uuid[]) OR recipient_id = ANY($2::text[]::uuid[])",
        "DELETE FROM counted_messages WHERE sender_id = ANY($2::text[]::uuid[])",
        "DELETE FROM key_packages WHERE device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM presence_values WHERE device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM long_poll_revisions WHERE device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM long_poll_link_runs WHERE device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM device_challenges WHERE device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM link_runs
         WHERE account_id = $1::uuid OR device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM sessions
         WHERE account_id = $1::uuid OR device_id = ANY($2::text[]::uuid[])
             OR key_device = ANY($2::text[]::uuid[])",
        "DELETE FROM service_tokens WHERE device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM device_codes WHERE account_id = $1::uuid",
        "DELETE FROM wrong_user_codes WHERE account_id = $1::uuid",
        "DELETE FROM wrong_pairings WHERE account_id = $1::uuid",
        "DELETE FROM acceptances WHERE account_id = $1::uuid",
        // Tasks its devices uploaded or that only one of them could claim, and its claims.
        "DELETE FROM tasks
         WHERE uploader_id = ANY($2::text[]::uuid[]) OR addressee = ANY($2::text[]::uuid[])",
        "DELETE FROM task_uploads WHERE device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM task_waits WHERE device_id = ANY($2::text[]::uuid[])",
        "DELETE FROM claims WHERE device_id = ANY($2::text[]::uuid[])",
        "UPDATE tasks SET claim_device = NULL WHERE claim_device = ANY($2::text[]::uuid[])",
        // The wraps to its recovery key and browsers.
        "DELETE FROM history_wraps
         WHERE recipient_id = $1::uuid OR recipient_id = ANY($2::text[]::uuid[])",
        "DELETE FROM channel_additions WHERE account_id = $1::uuid",
        "UPDATE channel_epochs
         SET members = ARRAY(SELECT m FROM unnest(members) m WHERE m <> ALL($2::text[]::uuid[]))
         WHERE members && $2::text[]::uuid[]",
        "UPDATE channel_groups SET creator_id = NULL WHERE creator_id = ANY($2::text[]::uuid[])",
        "UPDATE queues SET creator_id = NULL WHERE creator_id = $1::uuid",
        "UPDATE spaces SET creator_id = NULL WHERE creator_id = $1::uuid",
        "DELETE FROM devices WHERE account_id = $1::uuid AND cert IS NULL",
        "UPDATE devices SET anchors = NULL WHERE account_id = $1::uuid AND anchors IS NOT NULL",
        "UPDATE accounts SET master = NULL, record = NULL, backup = NULL, erased = true
         WHERE id = $1::uuid",
    ] {
        sqlx::query(statement)
            .bind(account)
            .bind(&devices)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
