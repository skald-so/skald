//! A deletion held while it runs: the test holds a lock on the account's sign-in methods, which the
//! deletion takes once it holds the account's row lock, so a request sent meanwhile runs while the
//! deletion's transaction is open.

use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use skald_proto::AccountId;
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
        let url = db.url();
        let account = account.to_string();
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
                let held = sqlx::query(
                    "SELECT 1 FROM sign_in_methods WHERE account_id = $1::uuid FOR SHARE",
                )
                .bind(&account)
                .execute(&mut *tx)
                .await
                .expect("lock the sign-in methods")
                .rows_affected();
                locked.send(held).expect("tell of the lock");
                // Released, or dropped.
                let _ = wait.recv();
                tx.commit().await.expect("commit");
            });
        });
        let held = ready.recv().expect("the lock");
        assert!(held > 0, "the account has a sign-in method");
        Held {
            release: Some(release),
            locker: Some(locker),
        }
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
