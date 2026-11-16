//! A test database and the connections that the tests' own queries keep open between them. A
//! connection its client closes holds one of the machine's ephemeral ports for 30 s on macOS, and
//! a test that polls the database every 100 ms would open hundreds; a kept one ends when
//! PostgreSQL ends it, as `DROP DATABASE … WITH (FORCE)` does. The client's tests include this
//! file too.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use sqlx::{AssertSqlSafe, Connection, PgConnection};
use tokio::runtime::Runtime;

/// Opens a connection to the database at a URL.
pub type Connect =
    fn(String) -> Pin<Box<dyn Future<Output = Result<PgConnection, sqlx::Error>> + Send>>;

/// A new empty database on a PostgreSQL server, dropped on drop, with its kept connections.
pub struct Database {
    server: String,
    name: String,
    connections: Connections,
}

impl Database {
    /// Makes the database `name` anew on the server at `server`, a connection URL without a
    /// database name; `connect` opens each connection.
    pub fn create(server: String, name: String, connect: Connect) -> Self {
        let connections = Connections::new(format!("{server}/{name}"), connect);
        // Dropped on drop from here on: also when making it fails.
        let db = Database {
            server,
            name,
            connections,
        };
        db.admin(&[
            format!("DROP DATABASE IF EXISTS {}", db.name),
            format!("CREATE DATABASE {}", db.name),
        ])
        .unwrap_or_else(|err| panic!("create a test database: {err}"));
        db
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn url(&self) -> String {
        format!("{}/{}", self.server, self.name)
    }

    /// Runs `work` on a kept connection to this database.
    pub fn query<T>(
        &self,
        work: impl AsyncFnOnce(&mut PgConnection) -> Result<T, sqlx::Error>,
    ) -> Result<T, sqlx::Error> {
        self.connections.run(work)
    }

    /// Runs `statements` one by one in the server's maintenance database: some, like
    /// `CREATE DATABASE`, cannot share a transaction.
    pub fn admin(&self, statements: &[String]) -> Result<(), String> {
        maintenance(&self.server, self.connections.connect)
            .run(async |conn| {
                for sql in statements {
                    let run = sqlx::raw_sql(AssertSqlSafe(sql.as_str()));
                    if let Err(err) = run.execute(&mut *conn).await {
                        return Ok(Err(format!("{sql}: {err:?}")));
                    }
                }
                Ok(Ok(()))
            })
            .map_err(|err| format!("connect: {err:?}"))?
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let statement = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name);
        if let Err(err) = self.admin(&[statement]) {
            eprintln!("cannot drop the test database {}: {err}", self.name);
        }
    }
}

/// The kept connections to the `postgres` maintenance database of the server at `server`, the
/// only server of a test process.
pub fn maintenance(server: &str, connect: Connect) -> &'static Connections {
    static MAINTENANCE: OnceLock<Connections> = OnceLock::new();
    MAINTENANCE.get_or_init(|| Connections::new(format!("{server}/postgres"), connect))
}

/// The open connections to the database at a URL that no query uses now.
pub struct Connections {
    url: String,
    connect: Connect,
    idle: Mutex<Vec<Open>>,
}

/// A connection and the runtime it was made on, which its socket needs.
struct Open {
    /// Dropped before its runtime.
    conn: PgConnection,
    runtime: Runtime,
}

impl Connections {
    fn new(url: String, connect: Connect) -> Self {
        Connections {
            url,
            connect,
            idle: Mutex::new(Vec::new()),
        }
    }

    /// Runs `work` on an idle connection that still answers, or on a new one; then keeps the
    /// connection for the next query. Not in an async context.
    pub fn run<T>(
        &self,
        work: impl AsyncFnOnce(&mut PgConnection) -> Result<T, sqlx::Error>,
    ) -> Result<T, sqlx::Error> {
        // One that PostgreSQL closed, such as while the database was unreachable, goes.
        let kept = loop {
            let Some(mut open) = self.lock().pop() else {
                break None;
            };
            if open.runtime.block_on(open.conn.ping()).is_ok() {
                break Some(open);
            }
        };
        let mut open = match kept {
            Some(open) => open,
            None => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("start a tokio runtime");
                let conn = runtime.block_on((self.connect)(self.url.clone()))?;
                Open { conn, runtime }
            }
        };
        let done = open.runtime.block_on(work(&mut open.conn));
        self.lock().push(open);
        done
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Open>> {
        self.idle.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
