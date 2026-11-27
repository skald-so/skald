//! Helpers for the integration tests: `skald` runs against a `skald-server` on a fresh PostgreSQL
//! database, or against a stub that gives canned answers.

use std::env;
use std::future::Future;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::pin::Pin;
use std::process::{self, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use sqlx::{AssertSqlSafe, Connection, PgConnection};
use tokio::runtime::Runtime;

#[path = "../../../server/tests/common/connections.rs"]
mod connections;
use connections::{Database, maintenance};

pub const CLIENT: &str = env!("CARGO_BIN_EXE_skald");

/// What a `skald` run printed, and how it exited.
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `skald` with `args`, speaking the protocol versions `versions` if given.
pub fn skald(args: &[&str], versions: Option<&str>) -> Output {
    output(command(args, versions).output().expect("run skald"))
}

/// Runs `skald` with `args` and a timeout of `seconds` for each request; fails if `skald` runs for
/// longer than `limit`.
pub fn skald_with_timeout(args: &[&str], seconds: u64, limit: Duration) -> Output {
    let mut child = command(args, None)
        .env("SKALD_TIMEOUT", seconds.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run skald");
    let started = Instant::now();
    while child.try_wait().expect("wait for skald").is_none() {
        if started.elapsed() > limit {
            let _ = child.kill();
            panic!("skald ran for more than {limit:?}");
        }
        thread::sleep(Duration::from_millis(50));
    }
    output(child.wait_with_output().expect("read skald's output"))
}

fn command(args: &[&str], versions: Option<&str>) -> Command {
    let mut command = Command::new(CLIENT);
    command
        .args(args)
        .env_remove("SKALD_PROTOCOL_VERSIONS")
        .env_remove("SKALD_TIMEOUT");
    if let Some(versions) = versions {
        command.env("SKALD_PROTOCOL_VERSIONS", versions);
    }
    command
}

fn output(output: process::Output) -> Output {
    Output {
        success: output.status.success(),
        stdout: String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr is UTF-8"),
    }
}

/// A `skald-server` on a new PostgreSQL database, serving on a free local port in this process.
/// The server and the database are removed on drop, the database also when the server fails to
/// start.
pub struct Server {
    pub url: String,
    runtime: Option<Runtime>,
    /// Dropped once the server has stopped.
    db: TestDb,
}

impl Server {
    /// Starts the server accepting the protocol versions `versions`, such as `2-3`, or its default
    /// versions if `None`.
    pub fn start(versions: Option<&str>) -> Self {
        let versions: skald_server::Versions = match versions {
            Some(versions) => versions.parse().expect("a protocol version range"),
            None => Default::default(),
        };
        Self::start_with(|_| skald_server::Settings {
            versions,
            ..Default::default()
        })
    }

    /// Starts the server with the settings `settings` gives for the address it listens on.
    pub fn start_with(settings: impl FnOnce(SocketAddr) -> skald_server::Settings) -> Self {
        let runtime = Runtime::new().expect("start a tokio runtime");
        let db = TestDb::create();
        let url = db.url();
        let pool = runtime
            .block_on(retried_on_reset("connect to the test database", || {
                skald_server::connect(&url)
            }))
            .expect("connect to the test database");
        let listener = runtime.block_on(async {
            skald_server::MIGRATOR
                .run(&pool)
                .await
                .expect("apply migrations");
            tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind a local port")
        });
        let addr = listener.local_addr().expect("the server's address");
        let settings = settings(addr);
        runtime.spawn(async move {
            axum::serve(listener, skald_server::app(pool, settings))
                .await
                .expect("serve the API");
        });
        Server {
            url: format!("http://{addr}"),
            runtime: Some(runtime),
            db,
        }
    }
}

impl Server {
    /// What every table but `_sqlx_migrations` holds, one XML document per table, whatever its
    /// schema; byte strings in hex, which, unlike base64 there, has no line breaks.
    pub fn rows(&self) -> Vec<String> {
        self.db
            .query(async |conn| {
                sqlx::raw_sql("SET xmlbinary = hex")
                    .execute(&mut *conn)
                    .await?;
                let tables: Vec<(String,)> = sqlx::query_as(
                    "SELECT query_to_xml(format('SELECT * FROM %I.%I', table_schema, table_name),
                                         true, false, '')::text
                     FROM information_schema.tables
                     WHERE table_type = 'BASE TABLE'
                       AND table_schema NOT IN ('pg_catalog', 'information_schema')
                       AND table_name <> '_sqlx_migrations'",
                )
                .fetch_all(conn)
                .await?;
                Ok(tables.into_iter().map(|(table,)| table).collect())
            })
            .expect("read the database's rows")
    }

    /// Runs `sql`, one or more statements, on the server's database, as an operator would.
    pub fn execute(&self, sql: &str) {
        self.db
            .query(async |conn| sqlx::raw_sql(AssertSqlSafe(sql)).execute(conn).await)
            .unwrap_or_else(|err| panic!("run {sql:?} on the database: {err}"));
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Stops the server and closes its connections; then the database goes, as `db` drops.
        let runtime = self.runtime.take().expect("the runtime runs until drop");
        runtime.shutdown_background();
    }
}

/// How many times a connection is tried while PostgreSQL resets it, and the wait before the first
/// retry, doubled before each next one: 1.5 s in all.
pub const SETUP_ATTEMPTS: u32 = 5;
pub const SETUP_RETRY_WAIT: Duration = Duration::from_millis(100);

/// WSAENOBUFS: Windows had no buffer space or port for a new socket.
pub const NO_BUFFER_SPACE: i32 = 10055;

/// The result of `step`, `what`, tried again while PostgreSQL resets its connection or opens none
/// in time, or while Windows has no buffer space for it, up to [`SETUP_ATTEMPTS`] times; any other
/// error is returned at once. Under the load of the client's tests PostgreSQL on Windows now and
/// then resets or delays a new connection and goes on serving the others, and Windows now and then
/// fails a connect with WSAENOBUFS for a cause still unknown. Each retry is a line on stderr, past
/// the test harness's capture so a passing test's log keeps it too, as [`retried_into`] writes it
/// with the samples CI's sampler of the ports in use writes.
pub async fn retried_on_reset<T, F: Future<Output = Result<T, sqlx::Error>>>(
    what: &str,
    step: impl FnMut() -> F,
) -> Result<T, sqlx::Error> {
    retried_into(what, port_samples(), &mut io::stderr(), step).await
}

/// The file where CI's sampler of the ports in use writes its state, if it runs.
pub fn port_samples() -> Option<std::path::PathBuf> {
    env::var_os("SKALD_TEST_PORT_SAMPLES").map(Into::into)
}

/// What [`retried_on_reset`] gives, each retry a line written to `sink`: its error and wait, and for
/// WSAENOBUFS the last of the ports in use that `samples` holds.
pub async fn retried_into<T, F: Future<Output = Result<T, sqlx::Error>>>(
    what: &str,
    samples: Option<std::path::PathBuf>,
    sink: &mut (impl Write + Send),
    mut step: impl FnMut() -> F,
) -> Result<T, sqlx::Error> {
    let mut wait = SETUP_RETRY_WAIT;
    let mut attempt = 1;
    loop {
        match step().await {
            Err(sqlx::Error::Io(err))
                if (matches!(
                    err.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::TimedOut
                ) || err.raw_os_error() == Some(NO_BUFFER_SPACE))
                    && attempt < SETUP_ATTEMPTS =>
            {
                let mut line =
                    format!("{what}: no connection to PostgreSQL: {err}; again in {wait:?}");
                if err.raw_os_error() == Some(NO_BUFFER_SPACE) {
                    line += &format!("; ports: {}", last_sample(samples.clone()));
                }
                let _ = writeln!(sink, "{line}");
                tokio::time::sleep(wait).await;
                wait *= 2;
                attempt += 1;
            }
            result => return result,
        }
    }
}

/// The last line of `samples`, the file where CI's sampler of the ports in use writes its state;
/// `not sampled` without one.
pub fn last_sample(samples: Option<std::path::PathBuf>) -> String {
    samples
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| {
            text.lines()
                .rfind(|line| !line.trim().is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "not sampled".to_owned())
}

/// A connection to `target`, such as `127.0.0.1:41234`, for the stand-in `who`, tried again while
/// Windows has no buffer space for it, as the tests' database connects are
/// ([`retried_on_reset`]); each retry a line on stderr.
pub fn reach(who: &str, target: &str) -> io::Result<TcpStream> {
    reached(who, target, port_samples(), &mut io::stderr(), || {
        TcpStream::connect(target)
    })
}

/// What [`reach`] gives with `connect` for the connect: up to [`SETUP_ATTEMPTS`] tries while it fails
/// with WSAENOBUFS, waiting [`SETUP_RETRY_WAIT`] before the first retry, doubled before each next
/// one; each retry a line written to `sink` with its error, its wait and the last of the ports in
/// use that `samples` holds. Any other error is given at once.
pub fn reached<T>(
    who: &str,
    target: &str,
    samples: Option<std::path::PathBuf>,
    sink: &mut impl Write,
    mut connect: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut wait = SETUP_RETRY_WAIT;
    for _ in 1..SETUP_ATTEMPTS {
        match connect() {
            Err(err) if err.raw_os_error() == Some(NO_BUFFER_SPACE) => {
                let ports = last_sample(samples.clone());
                let _ = writeln!(
                    sink,
                    "{who}: cannot reach {target}: {err}; again in {wait:?}; ports: {ports}"
                );
                thread::sleep(wait);
                wait *= 2;
            }
            result => return result,
        }
    }
    connect()
}

/// A new connection to the database at `url`, tried again while PostgreSQL resets it. Only
/// opening it is: a statement may have run when its connection resets.
pub async fn connect(url: &str) -> Result<PgConnection, sqlx::Error> {
    retried_on_reset("connect to PostgreSQL", || PgConnection::connect(url)).await
}

/// Closes `conn`, its statements done. An error of the close says nothing of them: PostgreSQL on
/// Windows resets a connection as its session ends, so the close fails when the reset comes first.
pub async fn close(conn: PgConnection) {
    let _ = conn.close().await;
}

/// The names of the test databases this process has on the PostgreSQL server.
pub fn test_databases() -> Vec<String> {
    let prefix = format!("{}{}_", TestDb::PREFIX, process::id());
    maintenance(&server(), connect_to)
        .run(async |conn| {
            sqlx::query_as::<_, (String,)>("SELECT datname FROM pg_database")
                .fetch_all(conn)
                .await
        })
        .expect("list the databases")
        .into_iter()
        .map(|(name,)| name)
        .filter(|name| name.starts_with(&prefix))
        .collect()
}

/// Opens a connection to the database at `url`, as [`connect`] does.
fn connect_to(
    url: String,
) -> Pin<Box<dyn Future<Output = Result<PgConnection, sqlx::Error>> + Send>> {
    Box::pin(async move { connect(&url).await })
}

/// The PostgreSQL server at `SKALD_TEST_DATABASE_URL`: a connection URL without a database name,
/// e.g. `postgres://postgres:password@127.0.0.1:5432`; `script/test` sets it.
pub fn server() -> String {
    env::var("SKALD_TEST_DATABASE_URL")
        .expect("SKALD_TEST_DATABASE_URL is not set: run the tests with script/test")
}

/// A new empty database on the PostgreSQL server at `SKALD_TEST_DATABASE_URL`, dropped on drop.
pub struct TestDb {
    db: Database,
}

impl TestDb {
    /// The start of the names of the test databases: `<PREFIX><pid>_<n>`.
    const PREFIX: &str = "skald_client_test_";

    pub fn create() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "{}{}_{}",
            Self::PREFIX,
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        TestDb {
            db: Database::create(server(), name, connect_to),
        }
    }
}

/// Its URL and the queries on its kept connections.
impl std::ops::Deref for TestDb {
    type Target = Database;

    fn deref(&self) -> &Database {
        &self.db
    }
}

/// A stub server: answers each request with `answer(path)`, a whole HTTP response, and records
/// the requests' heads. An empty answer keeps the connection open and never answers.
pub struct Stub {
    pub url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Stub {
    pub fn start(answer: fn(&str) -> String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let addr: SocketAddr = listener.local_addr().expect("the stub's address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        thread::spawn(move || {
            let mut silent = Vec::new();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().expect("clone a stream"));
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                // The body too: a connection closed with unread bytes resets, and the answer with it.
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                let _ = reader.read_exact(&mut vec![0; length]);
                let path = head.split(' ').nth(1).unwrap_or_default().to_owned();
                recorded.lock().unwrap().push(head);
                let answer = answer(&path);
                if answer.is_empty() {
                    silent.push(stream);
                    continue;
                }
                // One write: a WebSocket client reads a refusal's body only from the head's read.
                let _ = stream.write_all(answer.as_bytes());
            }
        });
        Stub {
            url: format!("http://{addr}"),
            requests,
        }
    }

    /// The heads of the requests so far, in lowercase.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|head| head.to_lowercase())
            .collect()
    }
}

/// The URL of `stub` at `0.0.0.0`, no address of this machine; yet on Linux and macOS a connection
/// to it reaches the stub, which would get any request sent there.
pub fn elsewhere(stub: &Stub) -> String {
    let port = stub.url.rsplit(':').next().expect("the stub's port");
    format!("http://0.0.0.0:{port}")
}

/// The error of a server URL `url` in plain HTTP to another machine.
pub fn cleartext(url: &str) -> String {
    format!(
        "skald: {url:?} is plain HTTP to another machine: use https://; plain http:// goes only to \
         this machine: localhost, 127.0.0.0/8 or [::1]\n"
    )
}

/// A whole HTTP response with `status`, `content_type` and `body`, closing the connection.
pub fn response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\
         skald-protocol-latest: 3\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}
