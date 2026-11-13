//! The client's local state: an SQLite database in the state directory. It holds no secret: those go
//! to [`crate::secrets`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use skald_proto::DeviceId;

/// The database's file in the state directory.
pub const FILE: &str = "state.sqlite3";

/// The schema's migrations, in order: the database's `user_version` counts those applied. Later
/// versions of the client only append to it.
const MIGRATIONS: &[&str] = &[
    // The machine's session: the server it signed in to and the session's account, once known. At
    // most one row.
    "CREATE TABLE session (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        server TEXT NOT NULL,
        account TEXT,
        github_login TEXT
    ) STRICT",
    // The machine's own device: its ID and `name`, set at its first sign-in, whether the server
    // took its registration, and how it is linked: `new` until it knows, `sent` with the master
    // public key it pinned and the certificate it issued with that key until the server takes
    // them, then `certified`; `waiting` for linking by another device. At most one row.
    "CREATE TABLE device (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        device TEXT NOT NULL,
        name TEXT NOT NULL,
        registered INTEGER NOT NULL DEFAULT 0 CHECK (registered IN (0, 1)),
        link TEXT NOT NULL DEFAULT 'new'
            CHECK (link IN ('new', 'sent', 'certified', 'waiting')),
        master BLOB CHECK (length(master) = 32),
        cert BLOB,
        CHECK ((link IN ('sent', 'certified')) = (master IS NOT NULL AND cert IS NOT NULL)),
        CHECK (link IN ('sent', 'certified') OR (master IS NULL AND cert IS NULL))
    ) STRICT",
    // The account the device belongs to: the account of the session it was set up with. A device
    // of a state older than this column takes its session's account, if the session has one.
    "ALTER TABLE device ADD COLUMN account TEXT;
    UPDATE device SET account = (SELECT account FROM session);",
    // The session keeps its account's username instead of the GitHub login. A session of a state
    // older than this column has no username until its next sign-in or `skald set-username`.
    "ALTER TABLE session DROP COLUMN github_login;
    ALTER TABLE session ADD COLUMN username TEXT;",
];

/// The state directory: `dir` if given, otherwise the platform's directory for the user's
/// application state.
pub fn dir(given: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(dir) = given {
        return Ok(dir);
    }
    let var = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    #[cfg(windows)]
    let base = var("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = var("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(windows, target_os = "macos")))]
    let base = var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".local/state")));
    base.map(|base| base.join("skald"))
        .ok_or_else(|| "no state directory: set SKALD_STATE_DIR".to_owned())
}

/// Creates `dir` if it is missing, readable only by its owner.
pub fn create_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// The session the state holds.
#[derive(Debug, PartialEq)]
pub struct Session {
    /// The server's base URL.
    pub server: String,
    /// The account's ID, once the server gave it.
    pub account: Option<String>,
    /// The account's username, as the latest sign-in or `skald set-username` recorded it.
    pub username: Option<String>,
}

/// The machine's own device, as the state holds it.
#[derive(Debug, PartialEq)]
pub struct Device {
    pub id: DeviceId,
    /// Its certificate's `name`.
    pub name: String,
    /// The account it belongs to; `None` only in a state older than this field whose session had
    /// no account.
    pub account: Option<String>,
    /// Whether the server took its registration.
    pub registered: bool,
    pub link: Link,
}

/// How the device is linked to its account.
#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    /// Not yet: it has not learned whether it is the account's first device.
    New,
    /// As the account's first device: it created the master key, pinned its public key `master`
    /// and issued its certificate `cert`, in CBOR, with it; the server has not taken them yet.
    Sent { master: [u8; 32], cert: Vec<u8> },
    /// The server took the master key and the certificate: the device holds its certificate.
    Certified { master: [u8; 32], cert: Vec<u8> },
    /// It waits for linking by another device of the account.
    Waiting,
}

/// Why the state cannot be used.
#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    /// A newer client wrote the state: it has more migrations than this one knows.
    Newer(u32),
    /// The state holds what this client does not read, for the reason it holds.
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Sqlite(err) => write!(f, "the local state failed: {err}"),
            Error::Newer(version) => write!(
                f,
                "a newer skald wrote the local state (schema version {version}); update skald"
            ),
            Error::Malformed(what) => write!(f, "the local state is malformed: {what}"),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Error::Sqlite(err)
    }
}

pub struct State {
    db: Connection,
}

impl State {
    /// Opens the state in `dir`, which exists, creating it if it is missing, and migrates its
    /// schema to the latest.
    pub fn open(dir: &Path) -> Result<Self, Error> {
        let mut db = Connection::open(dir.join(FILE))?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        migrate(&mut db)?;
        Ok(State { db })
    }

    /// The session, if the machine has one.
    pub fn session(&self) -> Result<Option<Session>, Error> {
        let session = self
            .db
            .query_row("SELECT server, account, username FROM session", [], |row| {
                Ok(Session {
                    server: row.get(0)?,
                    account: row.get(1)?,
                    username: row.get(2)?,
                })
            })
            .optional()?;
        Ok(session)
    }

    /// Replaces the session with one on `server` whose account is not known yet.
    pub fn start_session(&self, server: &str) -> Result<(), Error> {
        self.db.execute(
            "INSERT OR REPLACE INTO session (id, server) VALUES (1, ?1)",
            params![server],
        )?;
        Ok(())
    }

    /// Records the session's account and its username.
    pub fn set_account(&self, account: &str, username: &str) -> Result<(), Error> {
        self.db.execute(
            "UPDATE session SET account = ?1, username = ?2",
            params![account, username],
        )?;
        Ok(())
    }

    /// Records the username of the session's account.
    pub fn set_username(&self, username: &str) -> Result<(), Error> {
        self.db
            .execute("UPDATE session SET username = ?1", params![username])?;
        Ok(())
    }

    /// Forgets the session.
    pub fn end_session(&self) -> Result<(), Error> {
        self.db.execute("DELETE FROM session", [])?;
        Ok(())
    }

    /// The machine's own device, once it has one.
    pub fn device(&self) -> Result<Option<Device>, Error> {
        let row = self
            .db
            .query_row(
                "SELECT device, name, account, registered, link, master, cert FROM device",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<Vec<u8>>>(5)?,
                        row.get::<_, Option<Vec<u8>>>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, name, account, registered, link, master, cert)) = row else {
            return Ok(None);
        };
        let id = id
            .parse()
            .map_err(|err| Error::Malformed(format!("the device ID: {err}")))?;
        let pinned = || -> Result<([u8; 32], Vec<u8>), Error> {
            let master = master
                .and_then(|master| master.try_into().ok())
                .ok_or_else(|| Error::Malformed("the pinned master key".to_owned()))?;
            let cert = cert.ok_or_else(|| Error::Malformed("the certificate".to_owned()))?;
            Ok((master, cert))
        };
        let link = match link.as_str() {
            "new" => Link::New,
            "sent" => pinned().map(|(master, cert)| Link::Sent { master, cert })?,
            "certified" => pinned().map(|(master, cert)| Link::Certified { master, cert })?,
            "waiting" => Link::Waiting,
            other => return Err(Error::Malformed(format!("the device's link {other:?}"))),
        };
        Ok(Some(Device {
            id,
            name,
            account,
            registered,
            link,
        }))
    }

    /// Records the machine's own device of `account`, new and not registered yet.
    pub fn add_device(&self, id: DeviceId, name: &str, account: &str) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO device (id, device, name, account) VALUES (1, ?1, ?2, ?3)",
            params![id.to_string(), name, account],
        )?;
        Ok(())
    }

    /// Records the account of a device that has none recorded.
    pub fn set_device_account(&self, account: &str) -> Result<(), Error> {
        self.db.execute(
            "UPDATE device SET account = ?1 WHERE account IS NULL",
            params![account],
        )?;
        Ok(())
    }

    /// Records that the server took the device's registration.
    pub fn set_registered(&self) -> Result<(), Error> {
        self.db.execute("UPDATE device SET registered = 1", [])?;
        Ok(())
    }

    /// Records how the device is linked.
    pub fn set_link(&self, link: &Link) -> Result<(), Error> {
        let (name, pinned) = match link {
            Link::New => ("new", None),
            Link::Sent { master, cert } => ("sent", Some((master, cert))),
            Link::Certified { master, cert } => ("certified", Some((master, cert))),
            Link::Waiting => ("waiting", None),
        };
        let (master, cert) = pinned.unzip();
        self.db.execute(
            "UPDATE device SET link = ?1, master = ?2, cert = ?3",
            params![name, master.map(|master| master.as_slice()), cert],
        )?;
        Ok(())
    }
}

/// Applies the migrations `db` lacks, each in a transaction with its new `user_version`.
fn migrate(db: &mut Connection) -> Result<(), Error> {
    let latest = u32::try_from(MIGRATIONS.len()).expect("migrations fit in u32");
    loop {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: u32 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > latest {
            return Err(Error::Newer(version));
        }
        let Some(&migration) = MIGRATIONS.get(version as usize) else {
            return Ok(());
        };
        tx.execute_batch(migration)?;
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    #[test]
    fn new_state_has_the_latest_schema_and_no_session() {
        let dir = TestDir::new("state-new");
        let state = State::open(&dir).expect("open");
        let version: u32 = state
            .db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        assert_eq!(state.session().unwrap(), None);
    }

    #[test]
    fn session_is_kept_across_opens() {
        let dir = TestDir::new("state-kept");
        let state = State::open(&dir).unwrap();
        state.start_session("http://one.test").unwrap();
        assert_eq!(
            state.session().unwrap(),
            Some(Session {
                server: "http://one.test".to_owned(),
                account: None,
                username: None,
            })
        );
        state.set_account("0190", "ada").unwrap();
        drop(state);

        let state = State::open(&dir).unwrap();
        let ada = |username: &str| {
            Some(Session {
                server: "http://one.test".to_owned(),
                account: Some("0190".to_owned()),
                username: Some(username.to_owned()),
            })
        };
        assert_eq!(state.session().unwrap(), ada("ada"));
        state.set_username("ada-of-skald").unwrap();
        assert_eq!(
            State::open(&dir).unwrap().session().unwrap(),
            ada("ada-of-skald")
        );
        // A new session replaces it, with no account yet.
        state.start_session("http://two.test").unwrap();
        let session = state.session().unwrap().unwrap();
        assert_eq!(
            (session.server.as_str(), session.account, session.username),
            ("http://two.test", None, None)
        );
        state.end_session().unwrap();
        assert_eq!(state.session().unwrap(), None);
    }

    #[test]
    fn device_is_kept_across_opens_with_how_it_is_linked() {
        let dir = TestDir::new("state-device");
        let state = State::open(&dir).unwrap();
        assert_eq!(state.device().unwrap(), None);
        let id = DeviceId::from_signing_key(&[7; 32]);
        state.add_device(id, "ada-laptop", "0190").unwrap();
        assert!(state.add_device(id, "again", "0190").is_err(), "one device");
        let new = Device {
            id,
            name: "ada-laptop".to_owned(),
            account: Some("0190".to_owned()),
            registered: false,
            link: Link::New,
        };
        assert_eq!(state.device().unwrap().as_ref(), Some(&new));
        state.set_registered().unwrap();

        let (master, cert) = ([9; 32], vec![1, 2, 3]);
        for link in [
            Link::Sent {
                master,
                cert: cert.clone(),
            },
            Link::Certified { master, cert },
            Link::Waiting,
            Link::New,
        ] {
            state.set_link(&link).unwrap();
            let state = State::open(&dir).unwrap();
            let device = state.device().unwrap().expect("a device");
            assert!(device.registered);
            assert_eq!((device.id, device.name.as_str()), (id, "ada-laptop"));
            assert_eq!(device.account.as_deref(), Some("0190"));
            assert_eq!(device.link, link);
        }
        // The account is set only where none is recorded, and ending the session keeps it.
        state.set_device_account("0191").unwrap();
        state.start_session("http://one.test").unwrap();
        state.end_session().unwrap();
        assert_eq!(
            state.device().unwrap().unwrap().account.as_deref(),
            Some("0190")
        );
    }

    #[test]
    fn device_of_an_older_state_takes_its_sessions_account() {
        let dir = TestDir::new("state-device-account");
        let db = Connection::open(dir.join(FILE)).unwrap();
        for migration in &MIGRATIONS[..2] {
            db.execute_batch(migration).unwrap();
        }
        db.execute_batch(
            "PRAGMA user_version = 2;
             INSERT INTO session (id, server, account, github_login)
                 VALUES (1, 'http://one.test', '0190', 'ada');
             INSERT INTO device (id, device, name) VALUES (1, 'x', 'ada-laptop');",
        )
        .unwrap();
        let id = DeviceId::from_signing_key(&[7; 32]).to_string();
        db.execute("UPDATE device SET device = ?1", [&id]).unwrap();
        drop(db);

        let state = State::open(&dir).unwrap();
        let device = state.device().unwrap().expect("a device");
        assert_eq!(device.account.as_deref(), Some("0190"));
        // Without an account in its session, it has none until it signs in.
        state
            .db
            .execute("UPDATE device SET account = NULL", [])
            .unwrap();
        state.set_device_account("0191").unwrap();
        assert_eq!(
            state.device().unwrap().unwrap().account.as_deref(),
            Some("0191")
        );
    }

    #[test]
    fn session_of_a_state_with_the_github_login_keeps_its_account_and_drops_the_login() {
        let dir = TestDir::new("state-session-username");
        let db = Connection::open(dir.join(FILE)).unwrap();
        for migration in &MIGRATIONS[..3] {
            db.execute_batch(migration).unwrap();
        }
        db.execute_batch(
            "PRAGMA user_version = 3;
             INSERT INTO session (id, server, account, github_login)
                 VALUES (1, 'http://one.test', '0190', 'Ada_Lovelace');",
        )
        .unwrap();
        drop(db);

        let state = State::open(&dir).unwrap();
        assert_eq!(
            state.session().unwrap(),
            Some(Session {
                server: "http://one.test".to_owned(),
                account: Some("0190".to_owned()),
                username: None,
            })
        );
        let columns: Vec<String> = state
            .db
            .prepare("SELECT name FROM pragma_table_info('session')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(columns, ["id", "server", "account", "username"]);
    }

    #[test]
    fn device_of_a_state_that_breaks_its_schema_is_refused() {
        let dir = TestDir::new("state-device-malformed");
        let state = State::open(&dir).unwrap();
        state
            .db
            .execute(
                "INSERT INTO device (id, device, name) VALUES (1, 'not an ID', 'ada')",
                [],
            )
            .unwrap();
        let error = state.device().expect_err("malformed");
        assert!(matches!(error, Error::Malformed(_)), "{error:?}");
        // A sent master key needs its certificate, and a short one is no key.
        let id = DeviceId::from_signing_key(&[7; 32]).to_string();
        state
            .db
            .execute("UPDATE device SET device = ?1", [&id])
            .unwrap();
        for update in [
            "UPDATE device SET link = 'sent'",
            "UPDATE device SET link = 'sent', master = zeroblob(32)",
            "UPDATE device SET link = 'sent', master = zeroblob(31), cert = x'01'",
            "UPDATE device SET master = zeroblob(32), cert = x'01'",
            "UPDATE device SET link = 'linked'",
        ] {
            assert!(state.db.execute(update, []).is_err(), "{update}");
        }
    }

    #[test]
    fn state_of_a_newer_client_is_refused() {
        let dir = TestDir::new("state-newer");
        let newer = MIGRATIONS.len() as u32 + 1;
        Connection::open(dir.join(FILE))
            .unwrap()
            .pragma_update(None, "user_version", newer)
            .unwrap();
        let error = State::open(&dir).err().expect("refused");
        assert!(
            matches!(error, Error::Newer(version) if version == newer),
            "{error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn created_directory_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let parent = TestDir::new("state-mode");
        let dir = parent.join("a/skald");
        create_dir(&dir).expect("create");
        let mode = fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
        // An existing directory stays as it is.
        create_dir(&dir).expect("exists");
    }

    #[test]
    fn errors_say_what_failed() {
        assert_eq!(
            Error::Newer(7).to_string(),
            "a newer skald wrote the local state (schema version 7); update skald"
        );
        assert_eq!(
            Error::Malformed("the device ID".to_owned()).to_string(),
            "the local state is malformed: the device ID"
        );
        let error = Error::Sqlite(rusqlite::Error::InvalidQuery);
        assert!(
            error.to_string().starts_with("the local state failed: "),
            "{error}"
        );
    }

    #[test]
    fn given_dir_is_used() {
        let given = PathBuf::from("/some/where");
        assert_eq!(dir(Some(given.clone())).unwrap(), given);
    }
}
