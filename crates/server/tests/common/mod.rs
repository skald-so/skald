//! Helpers for the integration tests: a `skald-server` process and a fresh PostgreSQL database.
//! Each test binary uses a subset of them.
#![allow(dead_code)]

pub mod spec;

use std::env;
use std::io::{BufRead, BufReader, Read};
use std::net::TcpStream;
use std::process::{self, Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use sqlx::{AssertSqlSafe, Connection, PgConnection};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderMap;
use tungstenite::stream::MaybeTlsStream;

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
        Self::start_with(db, &[])
    }

    /// Starts the server on `db` with the extra arguments `args`, as [`Server::start`] does.
    pub fn start_with(db: &TestDb, args: &[&str]) -> Self {
        let url = db.url();
        let mut all = vec!["--listen", "127.0.0.1:0", "--database-url", &url];
        all.extend_from_slice(args);
        let mut child = spawn(&all);
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
        self.get_with(path, &[])
    }

    /// `GET <path>` with the request headers `headers`.
    pub fn get_with(&self, path: &str, headers: &[(&str, &str)]) -> Response {
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .build()
            .new_agent();
        let mut request = agent.get(format!("{}{path}", self.base));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut response = request
            .call()
            .unwrap_or_else(|err| panic!("GET {path}: {err}"));
        let body = response.body_mut().read_to_vec().expect("read body");
        Response::new(response.status().as_u16(), response.headers(), body)
    }

    /// Sends a WebSocket opening request to `path` with `Sec-WebSocket-Protocol: <offer>`, or
    /// without it if `offer` is `None`. `Ok` holds the open WebSocket and the `101` response; `Err`,
    /// the response that refuses it.
    pub fn open_websocket(
        &self,
        path: &str,
        offer: Option<&str>,
    ) -> Result<(WebSocket, Response), Response> {
        let url = format!("{}{path}", self.base.replacen("http://", "ws://", 1));
        let mut request = url.into_client_request().expect("a WebSocket URL");
        if let Some(offer) = offer {
            request.headers_mut().insert(
                "sec-websocket-protocol",
                offer.parse().expect("a header value"),
            );
        }
        match tungstenite::connect(request) {
            Ok((socket, response)) => {
                let response =
                    Response::new(response.status().as_u16(), response.headers(), Vec::new());
                Ok((socket, response))
            }
            Err(tungstenite::Error::Http(response)) => {
                let body = response.body().clone().unwrap_or_default();
                Err(Response::new(
                    response.status().as_u16(),
                    response.headers(),
                    body,
                ))
            }
            Err(err) => panic!("WebSocket {path}: {err}"),
        }
    }
}

pub type WebSocket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    /// Every header but `content-type`: lowercase names and their values.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    fn new(status: u16, headers: &HeaderMap, body: Vec<u8>) -> Self {
        let mut response = Response {
            status,
            content_type: String::new(),
            headers: Vec::new(),
            body,
        };
        for (name, value) in headers {
            let value = value.to_str().expect("header values are ASCII").to_owned();
            if name == "content-type" {
                response.content_type = value;
            } else {
                response.headers.push((name.as_str().to_owned(), value));
            }
        }
        response
    }

    /// The value of the first header `name`, in lowercase.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }
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
