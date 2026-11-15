//! Helpers for the integration tests: a `skald-server` process and a fresh PostgreSQL database.
//! Each test binary uses a subset of them.
#![allow(dead_code)]

pub mod accounts;
pub mod agent;
pub mod browser;
pub mod cbor;
pub mod check;
pub mod connections;
pub mod device_flow;
pub mod devices;
pub mod google;
pub mod sign_in;
pub mod spec;
pub mod times;

use std::env;
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::process::{self, Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use sqlx::{AssertSqlSafe, Connection, PgConnection};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderMap;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;

use agent::{agent, call};
use connections::Database;

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

/// Runs `skald-server` with `args` and the environment variables `envs` until it exits; gives
/// whether it succeeded and everything it printed, stdout then stderr.
pub fn run(args: &[&str], envs: &[(&str, &str)]) -> (bool, String) {
    let output = Command::new(SERVER)
        .args(args)
        .envs(envs.iter().copied())
        .output()
        .expect("run skald-server");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), printed)
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

/// Starts a server whose address must be known before it starts: `launch` starts it listening on
/// `127.0.0.1:<port>`, as [`Server::try_launch`] does. The port is one the OS gave and took back,
/// which another socket can take before the server listens on it; `launch` then runs again with
/// another port, for up to `within`.
pub fn on_free_port<T>(within: Duration, mut launch: impl FnMut(u16) -> Result<T, String>) -> T {
    let deadline = Instant::now() + within;
    loop {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().expect("local address").port();
        drop(listener);
        match launch(port) {
            Ok(launched) => return launched,
            Err(taken) if Instant::now() < deadline => {
                eprintln!("skald-server's port was taken, trying another: {taken:?}");
            }
            Err(taken) => panic!("no free port for skald-server within {within:?}: {taken:?}"),
        }
    }
}

/// A `skald-server` listening on a local port; killed on drop.
pub struct Server {
    child: Child,
    base: String,
    /// Everything the server printed so far, stdout and stderr, read by `readers`; also so that
    /// the server never writes to a closed pipe.
    output: Arc<Mutex<String>>,
    readers: Vec<JoinHandle<()>>,
}

impl Server {
    /// Starts the server on `db` and waits until it listens: its migrations are applied by then.
    pub fn start(db: &TestDb) -> Self {
        Self::start_with(db, &[])
    }

    /// Starts the server on `db` with the extra arguments `args`, as [`Server::start`] does.
    pub fn start_with(db: &TestDb, args: &[&str]) -> Self {
        Self::launch(db, "127.0.0.1:0", args, &[])
    }

    /// Starts the server on `db`, listening on `listen`, with the extra arguments `args` and the
    /// environment variables `envs`, as [`Server::start`] does.
    pub fn launch(db: &TestDb, listen: &str, args: &[&str], envs: &[(&str, &str)]) -> Self {
        Self::try_launch(db, listen, args, envs)
            .unwrap_or_else(|line| panic!("unexpected skald-server output: {line:?}"))
    }

    /// As [`Server::launch`]; `Err` holds what the server printed if it could not listen on
    /// `listen` because the address is in use.
    pub fn try_launch(
        db: &TestDb,
        listen: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> Result<Self, String> {
        Self::try_launch_at(&db.url(), listen, args, envs)
    }

    /// As [`Server::launch`], on the database at `url`, such as a test database reached as another
    /// role.
    pub fn launch_at(url: &str, listen: &str, args: &[&str], envs: &[(&str, &str)]) -> Self {
        Self::try_launch_at(url, listen, args, envs)
            .unwrap_or_else(|line| panic!("unexpected skald-server output: {line:?}"))
    }

    /// As [`Server::try_launch`], on the database at `url`.
    pub fn try_launch_at(
        url: &str,
        listen: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> Result<Self, String> {
        let mut all = vec!["--listen", listen, "--database-url", url];
        all.extend_from_slice(args);
        let mut child = Command::new(SERVER)
            .args(&all)
            .envs(envs.iter().copied())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start skald-server");
        let mut stderr = BufReader::new(child.stderr.take().expect("stderr is piped"));
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut line = String::new();
        stderr
            .read_line(&mut line)
            .expect("read skald-server stderr");
        let Some(base) = line.trim_end().strip_prefix("skald-server: listening on ") else {
            let _ = child.kill();
            let _ = child.wait();
            let in_use = format!("skald-server: cannot listen on {listen}: Address already in use");
            if line.starts_with(&in_use) {
                return Err(line);
            }
            panic!("unexpected skald-server output: {line:?}");
        };
        let base = base.to_owned();
        let output = Arc::new(Mutex::new(line));
        let readers = vec![collect(stderr, &output), collect(stdout, &output)];
        Ok(Server {
            child,
            base,
            output,
            readers,
        })
    }

    /// The address it listens on, e.g. `http://127.0.0.1:41234`.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Stops the server and returns everything it printed, stdout and stderr.
    pub fn stop(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for reader in self.readers.drain(..) {
            reader.join().expect("read skald-server output");
        }
        self.output.lock().expect("server output").clone()
    }

    pub fn get(&self, path: &str) -> Response {
        self.get_with(path, &[])
    }

    /// Checks that its health check answers ok.
    #[track_caller]
    pub fn assert_healthy(&self) {
        let health = self.get("/health");
        assert_eq!(health.status, 200);
        assert_eq!(health.body, br#"{"status":"ok"}"#);
    }

    /// `GET <path>` with the request headers `headers`.
    pub fn get_with(&self, path: &str, headers: &[(&str, &str)]) -> Response {
        get_url(&format!("{}{path}", self.base), headers)
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

/// `GET <url>` with the request headers `headers`; a redirect is returned, not followed.
pub fn get_url(url: &str, headers: &[(&str, &str)]) -> Response {
    let response = call(|| {
        let mut request = agent().get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.call()
    });
    read(response.unwrap_or_else(|err| panic!("GET {url}: {err}")))
}

/// `DELETE <url>` with the request headers `headers`; a redirect is returned, not followed.
pub fn delete_url(url: &str, headers: &[(&str, &str)]) -> Response {
    let response = call(|| {
        let mut request = agent().delete(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.call()
    });
    read(response.unwrap_or_else(|err| panic!("DELETE {url}: {err}")))
}

/// `POST <url>` of the form `fields`, `application/x-www-form-urlencoded`, with the request headers
/// `headers`; a redirect is returned, not followed.
pub fn post_form(url: &str, headers: &[(&str, &str)], fields: &[(&str, &str)]) -> Response {
    let response = call(|| {
        let mut request = agent().post(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send_form(fields.iter().copied())
    });
    read(response.unwrap_or_else(|err| panic!("POST {url}: {err}")))
}

/// `<method> <url>`, `method` `post` or `put`, of `body` with the request headers `headers`; a
/// redirect is returned, not followed.
pub fn send(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Response {
    let response = call(|| {
        let mut request = match method {
            "post" => agent().post(url),
            "put" => agent().put(url),
            _ => panic!("a method with a body: {method}"),
        };
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send(body)
    });
    read(response.unwrap_or_else(|err| panic!("{method} {url}: {err}")))
}

fn read(mut response: ureq::http::Response<ureq::Body>) -> Response {
    let body = response.body_mut().read_to_vec().expect("read body");
    Response::new(response.status().as_u16(), response.headers(), body)
}

/// The forms in which a database could hold `secret` in the clear: as it is, in lowercase, and the
/// bytes it encodes, if it is Base64, in the hex of PostgreSQL's text form of `bytea`.
pub fn stored_forms(secret: &str) -> Vec<String> {
    use base64::Engine as _;
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

    let mut forms = vec![
        secret.to_owned(),
        secret.to_lowercase(),
        hex(secret.as_bytes()),
    ];
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(secret)
            && bytes.len() >= 8
        {
            forms.push(hex(&bytes));
        }
    }
    forms
}

/// `bytes` in lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks that none of `rows`, a database's, holds any of `secrets` in a form of [`stored_forms`].
#[track_caller]
pub fn assert_holds_none(rows: &[String], secrets: &[String]) {
    for secret in secrets {
        for form in stored_forms(secret) {
            let found: Vec<_> = rows.iter().filter(|row| row.contains(&form)).collect();
            assert!(found.is_empty(), "{secret:?} as {form:?} in {found:?}");
        }
    }
}

/// Appends what `pipe` gives to `output` until it closes.
pub fn collect(pipe: impl Read + Send + 'static, output: &Arc<Mutex<String>>) -> JoinHandle<()> {
    let output = Arc::clone(output);
    thread::spawn(move || {
        let mut pipe = BufReader::new(pipe);
        let mut line = String::new();
        while matches!(pipe.read_line(&mut line), Ok(read) if read > 0) {
            output.lock().expect("server output").push_str(&line);
            line.clear();
        }
    })
}

pub type WebSocket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

/// How the server ended a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closed {
    /// With a close frame of this code and reason.
    Frame(CloseCode, String),
    /// Without a close frame.
    Dropped,
}

impl Closed {
    /// How a `Close` message with `frame` ended the connection.
    pub fn of(frame: Option<CloseFrame>) -> Self {
        frame.map_or(Closed::Dropped, |frame| {
            Closed::Frame(frame.code, frame.reason.as_str().to_owned())
        })
    }
}

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

    /// The values of every header `name`, in lowercase, such as each `set-cookie`.
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
            .collect()
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
    db: Database,
}

/// Its URL and the queries on its kept connections.
impl std::ops::Deref for TestDb {
    type Target = Database;

    fn deref(&self) -> &Database {
        &self.db
    }
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
        let db = Database::create(server, name, |url| {
            Box::pin(async move { PgConnection::connect(&url).await })
        });
        TestDb { db }
    }

    /// The URL of this database for the role `role` with `password`.
    pub fn url_as(&self, role: &str, password: &str) -> String {
        let mut url = url::Url::parse(&self.url()).expect("a database URL");
        url.set_username(role).expect("a URL with a user");
        url.set_password(Some(password))
            .expect("a URL with a password");
        url.into()
    }

    /// Runs `sql` in this database.
    pub fn execute(&self, sql: &str) {
        self.query(async |conn| sqlx::raw_sql(AssertSqlSafe(sql)).execute(conn).await)
            .unwrap_or_else(|err| panic!("{sql}: {err}"));
    }

    /// Makes the database refuse new connections and closes the open ones, or accept them again.
    pub fn set_reachable(&self, reachable: bool) {
        let name = self.db.name();
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
        self.db
            .admin(&statements)
            .expect("change whether the test database is reachable");
    }

    /// The migrations recorded in `_sqlx_migrations`, by version.
    pub fn applied_migrations(&self) -> Vec<AppliedMigration> {
        self.query(async |conn| {
            sqlx::query_as(
                "SELECT version, checksum, success, installed_on::text AS installed_on
                 FROM _sqlx_migrations ORDER BY version",
            )
            .fetch_all(conn)
            .await
        })
        .expect("read _sqlx_migrations")
    }

    /// Applies the embedded migrations up to `version`, the last one it applies.
    pub fn migrate_to(&self, version: i64) {
        self.query(async |conn| {
            skald_server::MIGRATOR
                .run_to(version, conn)
                .await
                .map_err(|err| sqlx::Error::Migrate(Box::new(err)))
        })
        .expect("apply the migrations");
    }

    /// Every column of every table, as `<schema>.<table>.<column> <type>`, sorted: the schema the
    /// migrations made.
    pub fn columns(&self) -> Vec<String> {
        self.query(async |conn| {
            let columns: Vec<(String,)> = sqlx::query_as(
                "SELECT table_schema || '.' || table_name || '.' || column_name || ' ' || data_type
                 FROM information_schema.columns
                 WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
                 ORDER BY 1",
            )
            .fetch_all(conn)
            .await?;
            Ok(columns.into_iter().map(|(column,)| column).collect())
        })
        .expect("read the database's columns")
    }

    /// Every row of every table but `_sqlx_migrations`, each as PostgreSQL's text form of the row:
    /// what the database holds, whatever its schema.
    pub fn rows(&self) -> Vec<String> {
        self.query(async |conn| {
            let tables: Vec<(String, String)> = sqlx::query_as(
                "SELECT table_schema::text, table_name::text FROM information_schema.tables
                 WHERE table_type = 'BASE TABLE'
                   AND table_schema NOT IN ('pg_catalog', 'information_schema')
                   AND table_name <> '_sqlx_migrations'",
            )
            .fetch_all(&mut *conn)
            .await?;
            let mut rows = Vec::new();
            for (schema, table) in tables {
                let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
                let sql = format!("SELECT t::text FROM {}.{} t", quote(&schema), quote(&table));
                let found: Vec<(String,)> = sqlx::query_as(AssertSqlSafe(sql))
                    .fetch_all(&mut *conn)
                    .await?;
                rows.extend(found.into_iter().map(|(row,)| format!("{table}: {row}")));
            }
            Ok(rows)
        })
        .expect("read the database's rows")
    }
}
