//! Helpers for the integration tests: a `skald-server` process and a fresh PostgreSQL database.
//! Each test binary uses a subset of them.
#![allow(dead_code)]

use std::env;
use std::io::{BufRead, BufReader, Read};
use std::process::{self, Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use sqlx::{AssertSqlSafe, Connection, PgConnection};

pub const SERVER: &str = env!("CARGO_BIN_EXE_skald-server");

/// Starts `skald-server` with `args`, stdout discarded and stderr piped.
pub fn spawn(args: &[&str]) -> Child {
    Command::new(SERVER)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start skald-server")
}

/// Runs `skald-server` with `args`, expecting it to fail to start; returns its stderr.
pub fn start_failure(args: &[&str]) -> String {
    let mut child = spawn(args);
    let status = wait(&mut child, Duration::from_secs(30));
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr is piped")
        .read_to_string(&mut stderr)
        .expect("read skald-server stderr");
    assert!(
        !status.success(),
        "exit status: {status}, stderr: {stderr:?}"
    );
    stderr
}

/// Waits for `child` to exit, killing it after `timeout`.
pub fn wait(child: &mut Child, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("wait for skald-server") {
            return status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("skald-server did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// A `skald-server` listening on a free local port; killed on drop.
pub struct Server {
    child: Child,
    /// Kept open so that the server never writes to a closed pipe.
    stderr: BufReader<ChildStderr>,
    base: String,
}

impl Server {
    /// Starts the server on `db` and waits until it listens: its migrations are applied by then.
    pub fn start(db: &TestDb) -> Self {
        let mut child = spawn(&["--listen", "127.0.0.1:0", "--database-url", &db.url()]);
        let stderr = BufReader::new(child.stderr.take().expect("stderr is piped"));
        let mut server = Server {
            child,
            stderr,
            base: String::new(),
        };
        let mut line = String::new();
        server
            .stderr
            .read_line(&mut line)
            .expect("read skald-server stderr");
        server.base = line
            .trim_end()
            .strip_prefix("skald-server: listening on ")
            .unwrap_or_else(|| panic!("unexpected skald-server output: {line:?}"))
            .to_owned();
        server
    }

    pub fn get(&self, path: &str) -> Response {
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .build()
            .new_agent();
        let mut response = agent
            .get(format!("{}{path}", self.base))
            .call()
            .unwrap_or_else(|err| panic!("GET {path}: {err}"));
        Response {
            status: response.status().as_u16(),
            content_type: response
                .headers()
                .get("content-type")
                .map(|value| value.to_str().expect("content type is ASCII").to_owned())
                .unwrap_or_default(),
            body: response.body_mut().read_to_vec().expect("read body"),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// A migration as recorded in the database.
#[derive(Debug, PartialEq, sqlx::FromRow)]
pub struct AppliedMigration {
    pub version: i64,
    pub checksum: Vec<u8>,
    pub success: bool,
    pub installed_on: String,
}

/// A new empty database on the PostgreSQL server at `SKALD_TEST_DATABASE_URL`; dropped on drop.
///
/// `SKALD_TEST_DATABASE_URL` is a connection URL without a database name, e.g.
/// `postgres://postgres:password@127.0.0.1:5432`; `script/test` sets it.
pub struct TestDb {
    server: String,
    name: String,
}

impl TestDb {
    pub fn create() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let server = env::var("SKALD_TEST_DATABASE_URL")
            .expect("SKALD_TEST_DATABASE_URL is not set: run the tests with script/test");
        let name = format!(
            "skald_test_{}_{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let db = TestDb { server, name };
        db.admin(&[
            format!("DROP DATABASE IF EXISTS {}", db.name),
            format!("CREATE DATABASE {}", db.name),
        ])
        .expect("create a test database");
        db
    }

    pub fn url(&self) -> String {
        format!("{}/{}", self.server, self.name)
    }

    /// Runs `sql` in this database.
    pub fn execute(&self, sql: &str) {
        execute(&self.url(), &[sql.to_owned()]).unwrap_or_else(|err| panic!("{sql}: {err}"));
    }

    /// Makes the database refuse new connections and closes the open ones, or accept them again.
    pub fn set_reachable(&self, reachable: bool) {
        let name = &self.name;
        let mut statements = vec![format!(
            "ALTER DATABASE {name} ALLOW_CONNECTIONS {reachable}"
        )];
        if !reachable {
            // Waits up to 5 s for each connection to close.
            statements.push(format!(
                "SELECT pg_terminate_backend(pid, 5000) FROM pg_stat_activity
                 WHERE datname = '{name}'"
            ));
        }
        self.admin(&statements)
            .expect("change whether the test database is reachable");
    }

    /// The migrations recorded in `_sqlx_migrations`, by version.
    pub fn applied_migrations(&self) -> Vec<AppliedMigration> {
        block_on(async {
            let mut conn = PgConnection::connect(&self.url()).await?;
            let applied = sqlx::query_as(
                "SELECT version, checksum, success, installed_on::text AS installed_on
                 FROM _sqlx_migrations ORDER BY version",
            )
            .fetch_all(&mut conn)
            .await?;
            conn.close().await?;
            Ok::<_, sqlx::Error>(applied)
        })
        .expect("read _sqlx_migrations")
    }

    /// Runs `statements` one by one in the `postgres` maintenance database of the server.
    fn admin(&self, statements: &[String]) -> Result<(), sqlx::Error> {
        execute(&format!("{}/postgres", self.server), statements)
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = self.admin(&[format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        )]);
    }
}

/// Runs `statements` at `url` one by one: some, like `CREATE DATABASE`, cannot share a transaction.
fn execute(url: &str, statements: &[String]) -> Result<(), sqlx::Error> {
    block_on(async {
        let mut conn = PgConnection::connect(url).await?;
        for sql in statements {
            sqlx::raw_sql(AssertSqlSafe(sql.as_str()))
                .execute(&mut conn)
                .await?;
        }
        conn.close().await
    })
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("start a tokio runtime")
        .block_on(future)
}
