//! Private review findings from existing account-level counter summaries.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sqlx::PgPool;
use tokio::sync::OnceCell;

use crate::sweeps::{self, PAGE, PAUSE};

const FILE_LIMIT: u64 = 32 * 1024;
const RULE_LIMIT: usize = 16;
const DAY: u32 = 86_400;
const RETENTION_LIMIT: u32 = 90 * DAY;
/// Overlap two sweeps so a qualifying window that ends between sweeps is still seen.
const LOOKBACK: Duration = sweeps::SWEEP.saturating_mul(2);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rules {
    version: String,
    retention_seconds: u32,
    rules: Vec<Rule>,
}

#[derive(Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Rule {
    signal: Signal,
    window_seconds: u32,
    minimum_count: i64,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Signal {
    CountedMessages,
}

impl Rules {
    /// Reads and validates the whole file before using any rule.
    fn read(path: &Path, counter_retention: Option<Duration>) -> Option<Self> {
        if !path.metadata().ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        File::open(path)
            .ok()?
            .take(FILE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > FILE_LIMIT {
            return None;
        }
        let rules: Self = serde_json::from_slice(&bytes).ok()?;
        rules.valid(counter_retention).then_some(rules)
    }

    fn valid(&self, counter_retention: Option<Duration>) -> bool {
        (1..=64).contains(&self.version.len())
            && self
                .version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            && (1..=RETENTION_LIMIT).contains(&self.retention_seconds)
            && self.rules.len() <= RULE_LIMIT
            && self.rules.iter().enumerate().all(|(index, rule)| {
                let window = Duration::from_secs(rule.window_seconds.into());
                (1..=DAY.min(self.retention_seconds)).contains(&rule.window_seconds)
                    && u64::from(self.retention_seconds) >= LOOKBACK.as_secs()
                    && rule.minimum_count > 0
                    && counter_retention.is_none_or(|retention| window + LOOKBACK <= retention)
                    && !self.rules[..index].contains(rule)
            })
    }
}

pub(crate) async fn run(db: PgPool, path: PathBuf, counter_retention: Option<Duration>) {
    let rules = tokio::task::spawn_blocking(move || Rules::read(&path, counter_retention)).await;
    let Ok(Some(rules)) = rules else {
        eprintln!("skald-server: cannot load abuse rules");
        return;
    };
    if rules.rules.is_empty() {
        return;
    }
    let started = OnceCell::new();
    sweeps::every("evaluate abuse rules", || async {
        let started = started
            .get_or_try_init(|| {
                sqlx::query_scalar::<_, String>("SELECT now()::text").fetch_one(&db)
            })
            .await?;
        evaluate(&db, &rules, started).await
    })
    .await;
}

async fn evaluate(db: &PgPool, rules: &Rules, started: &str) -> Result<(), sqlx::Error> {
    for rule in &rules.rules {
        match rule.signal {
            Signal::CountedMessages => counted_messages(db, rules, rule, started).await?,
        }
    }
    Ok(())
}

/// Keeps one open finding per account and versioned rule while it is retained. Its snapshot is
/// the first qualifying window observed. Every sample consists only of counts and times.
async fn counted_messages(
    db: &PgPool,
    rules: &Rules,
    rule: &Rule,
    started: &str,
) -> Result<(), sqlx::Error> {
    // Consider the current window and each interval ending at a recorded message since the
    // later of startup and the lookback. PostgreSQL timestamps have microsecond resolution.
    // Lock each still-active account so an account deletion either waits for this insert or the
    // insert sees the deletion. Use the same order on all evaluating instances.
    sqlx::query(
        "WITH bounds AS (
             SELECT make_interval(secs => $2::integer) AS span,
                 greatest($5::timestamptz, now() - make_interval(secs => $6)) AS since
         ), recent AS (
             SELECT d.account_id, m.counted_at
             FROM counted_messages m JOIN devices d ON d.id = m.sender_id
             CROSS JOIN bounds b
             WHERE m.counted_at > b.since - b.span AND m.counted_at <= now()
         ), windows AS (
             SELECT r.account_id, count(*) AS count,
                 min(r.counted_at) AS first_at, max(r.counted_at) AS last_at, now() AS ends_at
             FROM recent r CROSS JOIN bounds b WHERE r.counted_at > now() - b.span
             GROUP BY r.account_id
             UNION ALL
             SELECT account_id, count(*) OVER observed, first_value(counted_at) OVER observed,
                 counted_at, counted_at
             FROM recent
             WINDOW observed AS (PARTITION BY account_id ORDER BY counted_at RANGE BETWEEN
                 (make_interval(secs => $2::integer) - interval '1 microsecond') PRECEDING
                 AND CURRENT ROW)
         ), summary AS (
             SELECT DISTINCT ON (account_id) account_id, count, first_at, last_at
             FROM windows w CROSS JOIN bounds b
             WHERE w.count >= $3 AND w.ends_at >= b.since
             ORDER BY account_id, ends_at DESC
         )
         INSERT INTO abuse_findings (account_id, rule_version, signal, window_seconds,
             minimum_count, retention_seconds, evidence)
         SELECT a.id, $1, 'counted_messages', $2, $3, $4,
             jsonb_build_object('count', s.count, 'first_counted_at', s.first_at,
                 'last_counted_at', s.last_at)
         FROM summary s JOIN accounts a ON a.id = s.account_id
         WHERE a.deleted_at IS NULL AND NOT EXISTS (
             SELECT 1 FROM abuse_findings f WHERE f.account_id = a.id AND f.rule_version = $1
                 AND f.signal = 'counted_messages' AND f.window_seconds = $2 AND f.minimum_count = $3
         )
         ORDER BY a.id FOR SHARE OF a
         ON CONFLICT (account_id, rule_version, signal, window_seconds, minimum_count) DO NOTHING",
    )
    .bind(&rules.version)
    .bind(i32::try_from(rule.window_seconds).expect("a validated window"))
    .bind(rule.minimum_count)
    .bind(i32::try_from(rules.retention_seconds).expect("a validated retention"))
    .bind(started)
    .bind(LOOKBACK.as_secs_f64())
    .execute(db)
    .await?;
    Ok(())
}

/// Expires findings even if this instance has no rules. Each finding carries its own retention.
pub(crate) async fn expire(db: PgPool) {
    sweeps::every("expire abuse findings", || expire_due(&db)).await;
}

async fn expire_due(db: &PgPool) -> Result<(), sqlx::Error> {
    loop {
        let removed = sqlx::query(
            "DELETE FROM abuse_findings WHERE id IN (
                 SELECT id FROM abuse_findings WHERE expires_at <= (now() AT TIME ZONE 'UTC')
                 ORDER BY expires_at LIMIT $1 FOR UPDATE SKIP LOCKED
             )",
        )
        .bind(PAGE)
        .execute(db)
        .await?;
        if removed.rows_affected() == 0 {
            return Ok(());
        }
        tokio::time::sleep(PAUSE).await;
    }
}
