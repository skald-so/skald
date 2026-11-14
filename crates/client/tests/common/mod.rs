//! Helpers for the integration tests: `skald` runs against a `skald-server` on a fresh PostgreSQL
//! database, or against a stub that gives canned answers.

use std::env;
use std::io::{BufRead, BufReader, Read, Write};
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
/// The server and the database are removed on drop.
pub struct Server {
    pub url: String,
    runtime: Option<Runtime>,
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
        let (listener, pool) = runtime.block_on(async {
            let pool = skald_server::connect(&url)
                .await
                .expect("connect to the test database");
            skald_server::MIGRATOR
                .run(&pool)
                .await
                .expect("apply migrations");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind a local port");
            (listener, pool)
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
                let mut conn = PgConnection::connect(&self.db.url()).await?;
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
                let mut conn = PgConnection::connect(&self.db.url()).await?;
                sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut conn).await?;
                conn.close().await
            })
            .unwrap_or_else(|err| panic!("run {sql:?} on the database: {err}"));
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Stops the server and closes its connections, so the database can be dropped.
        let runtime = self.runtime.take().expect("the runtime runs until drop");
        runtime.shutdown_background();
        let runtime = Runtime::new().expect("start a tokio runtime");
        let _ = self.db.admin(
            &runtime,
            &[format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                self.db.name
            )],
        );
    }
}

/// A new empty database on the PostgreSQL server at `SKALD_TEST_DATABASE_URL`.
///
/// `SKALD_TEST_DATABASE_URL` is a connection URL without a database name, e.g.
/// `postgres://postgres:password@127.0.0.1:5432`; `script/test` sets it.
struct TestDb {
    server: String,
    name: String,
}

impl TestDb {
    fn create(runtime: &Runtime) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let server = env::var("SKALD_TEST_DATABASE_URL")
            .expect("SKALD_TEST_DATABASE_URL is not set: run the tests with script/test");
        let name = format!(
            "skald_client_test_{}_{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let db = TestDb { server, name };
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
            let mut conn = PgConnection::connect(&format!("{}/postgres", self.server)).await?;
            for sql in statements {
                sqlx::raw_sql(AssertSqlSafe(sql.as_str()))
                    .execute(&mut conn)
                    .await?;
            }
            conn.close().await
        })
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
