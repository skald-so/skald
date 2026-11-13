//! Helpers for the integration tests: `skald` runs against a `skald-server` on a fresh PostgreSQL
//! database, or against a stub that gives canned answers.

use std::env;
use std::future::Future;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::process::{self, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use sqlx::{AssertSqlSafe, Connection, PgConnection};
use tokio::runtime::Runtime;

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
            sign_in: None,
        })
    }

    /// Starts the server with the settings `settings` gives for the address it listens on.
    pub fn start_with(settings: impl FnOnce(SocketAddr) -> skald_server::Settings) -> Self {
        let runtime = Runtime::new().expect("start a tokio runtime");
        let db = TestDb::create(&runtime);
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
        let runtime = Runtime::new().expect("start a tokio runtime");
        runtime
            .block_on(async {
                let mut conn = connect(&self.db.url()).await?;
                sqlx::raw_sql("SET xmlbinary = hex")
                    .execute(&mut conn)
                    .await?;
                let tables: Vec<(String,)> = sqlx::query_as(
                    "SELECT query_to_xml(format('SELECT * FROM %I.%I', table_schema, table_name),
                                         true, false, '')::text
                     FROM information_schema.tables
                     WHERE table_type = 'BASE TABLE'
                       AND table_schema NOT IN ('pg_catalog', 'information_schema')
                       AND table_name <> '_sqlx_migrations'",
                )
                .fetch_all(&mut conn)
                .await?;
                conn.close().await?;
                Ok::<_, sqlx::Error>(tables.into_iter().map(|(table,)| table).collect())
            })
            .expect("read the database's rows")
    }

    /// Runs `sql`, one or more statements, on the server's database, as an operator would.
    pub fn execute(&self, sql: &str) {
        let runtime = Runtime::new().expect("start a tokio runtime");
        runtime
            .block_on(async {
                let mut conn = connect(&self.db.url()).await?;
                sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut conn).await?;
                conn.close().await
            })
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
const SETUP_RETRY_WAIT: Duration = Duration::from_millis(100);

/// The result of `step`, `what`, tried again while PostgreSQL resets its connection, up to
/// [`SETUP_ATTEMPTS`] times; any other error is returned at once. Under the load of the client's
/// tests PostgreSQL on Windows now and then resets a new connection and goes on serving the others.
pub async fn retried_on_reset<T, F: Future<Output = Result<T, sqlx::Error>>>(
    what: &str,
    mut step: impl FnMut() -> F,
) -> Result<T, sqlx::Error> {
    let mut wait = SETUP_RETRY_WAIT;
    let mut attempt = 1;
    loop {
        match step().await {
            Err(sqlx::Error::Io(err))
                if err.kind() == io::ErrorKind::ConnectionReset && attempt < SETUP_ATTEMPTS =>
            {
                eprintln!("{what}: PostgreSQL reset the connection: {err}; again in {wait:?}");
                tokio::time::sleep(wait).await;
                wait *= 2;
                attempt += 1;
            }
            result => return result,
        }
    }
}

/// A new connection to the database at `url`, tried again while PostgreSQL resets it. Only
/// opening it is: a statement may have run when its connection resets.
async fn connect(url: &str) -> Result<PgConnection, sqlx::Error> {
    retried_on_reset("connect to PostgreSQL", || PgConnection::connect(url)).await
}

/// The names of the test databases this process has on the PostgreSQL server.
pub fn test_databases() -> Vec<String> {
    let runtime = Runtime::new().expect("start a tokio runtime");
    let prefix = format!("{}{}_", TestDb::PREFIX, process::id());
    runtime
        .block_on(async {
            let mut conn = connect(&format!("{}/postgres", server())).await?;
            let names: Vec<(String,)> = sqlx::query_as("SELECT datname FROM pg_database")
                .fetch_all(&mut conn)
                .await?;
            conn.close().await?;
            Ok::<_, sqlx::Error>(names)
        })
        .expect("list the databases")
        .into_iter()
        .map(|(name,)| name)
        .filter(|name| name.starts_with(&prefix))
        .collect()
}

/// The PostgreSQL server at `SKALD_TEST_DATABASE_URL`: a connection URL without a database name,
/// e.g. `postgres://postgres:password@127.0.0.1:5432`; `script/test` sets it.
fn server() -> String {
    env::var("SKALD_TEST_DATABASE_URL")
        .expect("SKALD_TEST_DATABASE_URL is not set: run the tests with script/test")
}

/// A new empty database on the PostgreSQL server at `SKALD_TEST_DATABASE_URL`, dropped on drop.
struct TestDb {
    server: String,
    name: String,
}

impl TestDb {
    /// The start of the names of the test databases: `<PREFIX><pid>_<n>`.
    const PREFIX: &str = "skald_client_test_";

    fn create(runtime: &Runtime) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "{}{}_{}",
            Self::PREFIX,
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        // Dropped on drop from here on: also when making it fails.
        let db = TestDb {
            server: server(),
            name,
        };
        db.admin(
            runtime,
            &[
                format!("DROP DATABASE IF EXISTS {}", db.name),
                format!("CREATE DATABASE {}", db.name),
            ],
        )
        .expect("create a test database");
        db
    }

    fn url(&self) -> String {
        format!("{}/{}", self.server, self.name)
    }

    /// Runs `statements` one by one in the `postgres` maintenance database of the server: some,
    /// like `CREATE DATABASE`, cannot share a transaction.
    fn admin(&self, runtime: &Runtime, statements: &[String]) -> Result<(), sqlx::Error> {
        runtime.block_on(async {
            let mut conn = connect(&format!("{}/postgres", self.server)).await?;
            for sql in statements {
                sqlx::raw_sql(AssertSqlSafe(sql.as_str()))
                    .execute(&mut conn)
                    .await?;
            }
            conn.close().await
        })
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let runtime = Runtime::new().expect("start a tokio runtime");
        let statement = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name);
        if let Err(err) = self.admin(&runtime, &[statement]) {
            eprintln!("cannot drop the test database {}: {err}", self.name);
        }
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

/// A whole HTTP response with `status`, `content_type` and `body`, closing the connection.
pub fn response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\
         skald-protocol-latest: 3\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}
