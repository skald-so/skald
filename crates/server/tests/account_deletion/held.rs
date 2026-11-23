//! Locks the test holds so that requests run in an order: one on an account's sign-in methods,
//! which a deletion takes once it holds the account's row lock, so a request sent meanwhile runs
//! while the deletion's transaction is open; and a device's lock for opening its link runs.

use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use skald_proto::{AccountId, DeviceId};
use sqlx::{Connection as _, PgConnection};

use crate::common::TestDb;

/// How long a test waits for a request to wait on a lock.
const WAIT: Duration = Duration::from_secs(15);

/// A lock the test holds in a transaction of its own until it releases it.
pub struct Held {
    release: Option<Sender<()>>,
    locker: Option<JoinHandle<()>>,
}

impl Held {
    /// Holds a share lock on the sign-in methods of `account` in `db`.
    pub fn sign_in_methods(db: &TestDb, account: AccountId) -> Self {
        let sql = "SELECT 1 FROM sign_in_methods WHERE account_id = $1::uuid FOR SHARE";
        let held = Self::locking(db, sql, account.to_string());
        assert!(held.1 > 0, "the account has a sign-in method");
        held.0
    }

    /// Holds the lock of `device` that the server takes to open a link run of it, in `db`.
    pub fn link_runs(db: &TestDb, device: DeviceId) -> Self {
        let sql = "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))";
        Self::locking(db, sql, format!("link run {device}")).0
    }

    /// Runs `sql`, which takes locks, `$1` bound to `bound`, in a transaction of its own until
    /// released; gives the lock and the rows `sql` gave.
    fn locking(db: &TestDb, sql: &'static str, bound: String) -> (Self, u64) {
        let url = db.url();
        let (locked, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel::<()>();
        let locker = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("start a tokio runtime");
            runtime.block_on(async {
                let mut conn = PgConnection::connect(&url).await.expect("connect");
                let mut tx = conn.begin().await.expect("begin");
                let held = sqlx::query(sql)
                    .bind(&bound)
                    .execute(&mut *tx)
                    .await
                    .expect("take the lock")
                    .rows_affected();
                locked.send(held).expect("tell of the lock");
                // Released, or dropped.
                let _ = wait.recv();
                tx.commit().await.expect("commit");
            });
        });
        let held = ready.recv().expect("the lock");
        let lock = Held {
            release: Some(release),
            locker: Some(locker),
        };
        (lock, held)
    }

    /// Releases the lock.
    pub fn release(mut self) {
        self.end();
    }

    fn end(&mut self) {
        drop(self.release.take());
        if let Some(locker) = self.locker.take() {
            locker.join().expect("the lock's thread");
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        if !thread::panicking() {
            self.end();
        }
    }
}

/// Waits until `count` requests to `db` wait on a lock, which they must within [`WAIT`]; `what`
/// names the last.
#[track_caller]
pub fn until_waiting(db: &TestDb, count: i64, what: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        let waiting: (i64,) = db
            .query(async |conn| {
                sqlx::query_as(
                    "SELECT count(*) FROM pg_stat_activity
                     WHERE datname = current_database() AND wait_event_type = 'Lock'",
                )
                .fetch_one(conn)
                .await
            })
            .expect("read the waiting requests");
        if waiting.0 >= count {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: not waiting within {WAIT:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}
