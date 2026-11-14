//! The client's local state: an SQLite database in the state directory. It holds no secret: those go
//! to [`crate::secrets`].

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use skald_proto::{ChannelId, DeviceId, MessageId, SpaceId};

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
    // The spaces the device accepted a revision of: each one's ID and name, its anchor (revision 1
    // of a space it created) and the last revision it accepted, as their CBOR bytes, and the
    // revision it signed that the server has not taken yet, if any.
    "CREATE TABLE space (
        space TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        anchor BLOB NOT NULL,
        accepted BLOB NOT NULL,
        pending BLOB
    ) STRICT",
    // The hash of each revision the device accepted of a space, by its number, but that of the
    // space's last accepted revision, which the space holds: it is recorded once the device
    // accepts the next one. A state older than this table accepted only revision 1 of each space.
    "CREATE TABLE accepted_revision (
        space TEXT NOT NULL,
        revision INTEGER NOT NULL CHECK (revision >= 1),
        hash BLOB NOT NULL CHECK (length(hash) = 32),
        PRIMARY KEY (space, revision)
    ) STRICT",
    // The device's personal space: the one it pinned when it signed its revision 1. At most one.
    "ALTER TABLE space ADD COLUMN personal INTEGER NOT NULL DEFAULT 0 CHECK (personal IN (0, 1));
    CREATE UNIQUE INDEX space_personal ON space (personal) WHERE personal = 1;",
    // The rules that bind a folder and everything below it to a space: the folder's canonical path
    // and the space's name, which may be of a space the device is not in.
    "CREATE TABLE folder_rule (
        folder TEXT PRIMARY KEY,
        space TEXT NOT NULL
    ) STRICT",
    // The `.skald.toml` files the device's user approved: the canonical path of each one's folder
    // and the SHA-256 of its bytes as approved.
    "CREATE TABLE approved_config (
        folder TEXT PRIMARY KEY,
        hash BLOB NOT NULL CHECK (length(hash) = 32)
    ) STRICT",
    // The outbox: each envelope the device sealed that the server has not answered yet, in the
    // order it goes out, by its message ID and recipient device, with its space and its bytes; and
    // the last `counter` the device gave content to each recipient device in each channel.
    "CREATE TABLE outbox (
        position INTEGER PRIMARY KEY,
        msg_id TEXT NOT NULL,
        recipient TEXT NOT NULL,
        space TEXT NOT NULL,
        envelope BLOB NOT NULL,
        UNIQUE (msg_id, recipient)
    ) STRICT;
    CREATE TABLE send_counter (
        channel TEXT NOT NULL,
        recipient TEXT NOT NULL,
        last INTEGER NOT NULL CHECK (last >= 1),
        PRIMARY KEY (channel, recipient)
    ) STRICT;",
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

/// Creates `dir` if it is missing: on Unix readable only by its owner; on Windows it has the access
/// rights it inherits from its parent.
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

/// A space the device accepted a revision of, as the state holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    /// Its anchor, in CBOR: revision 1 of a space the device created.
    pub anchor: Vec<u8>,
    /// The last revision the device accepted, in CBOR.
    pub accepted: Vec<u8>,
    /// The revision the device signed that the server has not taken yet, in CBOR.
    pub pending: Option<Vec<u8>>,
    /// Whether it is the device's personal space.
    pub personal: bool,
}

impl Space {
    /// Whether its pending revision is its revision 1, which the server has not taken yet.
    pub fn first_waits(&self) -> bool {
        self.pending.as_ref() == Some(&self.anchor)
    }
}

/// What the state holds of the revisions the device accepted of a space.
#[derive(Debug, Clone, PartialEq)]
pub struct Accepted {
    /// The last accepted revision, in CBOR.
    pub last: Vec<u8>,
    /// The hash of each earlier accepted revision, by number.
    pub hashes: BTreeMap<u64, [u8; 32]>,
}

/// An envelope in the outbox.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub msg_id: MessageId,
    pub recipient: DeviceId,
    pub space: SpaceId,
    /// Its bytes, as they are sent every time.
    pub envelope: Vec<u8>,
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

    /// The spaces the device accepted a revision of, by name.
    pub fn spaces(&self) -> Result<Vec<Space>, Error> {
        let mut query = self.db.prepare(
            "SELECT space, name, anchor, accepted, pending, personal FROM space ORDER BY name",
        )?;
        let rows = query.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?;
        rows.map(|row| {
            let (id, name, anchor, accepted, pending, personal) = row?;
            let id = id
                .parse()
                .map_err(|err| Error::Malformed(format!("the space ID {id:?}: {err}")))?;
            Ok(Space {
                id,
                name,
                anchor,
                accepted,
                pending,
                personal,
            })
        })
        .collect()
    }

    /// Records the space `id` named `name` that the device created, with its revision 1 `first`,
    /// in CBOR, as its anchor and last accepted revision, and as pending until the server takes it.
    pub fn add_space(&self, id: SpaceId, name: &str, first: &[u8]) -> Result<(), Error> {
        self.insert_space(id, name, first, false)
    }

    /// Records the space `id` as [`State::add_space`] does, pinned as the device's personal space;
    /// fails if the device pinned one.
    pub fn add_personal_space(&self, id: SpaceId, name: &str, first: &[u8]) -> Result<(), Error> {
        self.insert_space(id, name, first, true)
    }

    fn insert_space(
        &self,
        id: SpaceId,
        name: &str,
        first: &[u8],
        personal: bool,
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO space (space, name, anchor, accepted, pending, personal)
             VALUES (?1, ?2, ?3, ?3, ?3, ?4)",
            params![id.to_string(), name, first, personal],
        )?;
        Ok(())
    }

    /// Records `next`, in CBOR, as the pending revision of the space `id`, in place of `replaced`
    /// or of none, while `base`, in CBOR, is its last accepted revision. Records nothing and gives
    /// `false` if another run changed either meanwhile.
    pub fn set_pending(
        &self,
        id: SpaceId,
        base: &[u8],
        replaced: Option<&[u8]>,
        next: &[u8],
    ) -> Result<bool, Error> {
        let changed = self.db.execute(
            "UPDATE space SET pending = ?4 WHERE space = ?1 AND accepted = ?2 AND pending IS ?3",
            params![id.to_string(), base, replaced, next],
        )?;
        Ok(changed > 0)
    }

    /// Forgets `pending`, in CBOR, the pending revision of the space `id`, which the server took or
    /// refused; leaves another one.
    pub fn clear_pending(&self, id: SpaceId, pending: &[u8]) -> Result<(), Error> {
        self.db.execute(
            "UPDATE space SET pending = NULL WHERE space = ?1 AND pending = ?2",
            params![id.to_string(), pending],
        )?;
        Ok(())
    }

    /// Forgets the space `id`.
    pub fn remove_space(&self, id: SpaceId) -> Result<(), Error> {
        let tx = self.db.unchecked_transaction()?;
        let id = id.to_string();
        tx.execute(
            "DELETE FROM accepted_revision WHERE space = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM space WHERE space = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    /// What the state holds of the space `id`, if it holds it: its last accepted revision and the
    /// hashes of the earlier ones, read in one transaction, so another run's accepting is in both
    /// or in neither.
    pub fn accepted(&self, id: SpaceId) -> Result<Option<Accepted>, Error> {
        let tx = self.db.unchecked_transaction()?;
        let id = id.to_string();
        let Some(last) = tx
            .query_row(
                "SELECT accepted FROM space WHERE space = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let mut query =
            tx.prepare("SELECT revision, hash FROM accepted_revision WHERE space = ?1")?;
        let rows = query.query_map(params![id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let hashes = rows
            .map(|row| {
                let (revision, hash) = row?;
                let malformed = || Error::Malformed(format!("the hash of revision {revision}"));
                let revision = u64::try_from(revision).map_err(|_| malformed())?;
                let hash = hash.try_into().map_err(|_| malformed())?;
                Ok((revision, hash))
            })
            .collect::<Result<_, Error>>()?;
        drop(query);
        tx.commit()?;
        Ok(Some(Accepted { last, hashes }))
    }

    /// Records that the device accepted revisions of the space `id` after `from`, its last accepted
    /// revision until now, in CBOR: `hashes`, the number and hash of `from` and of each revision but
    /// the last, and `last`, in CBOR, its new last accepted revision. Records nothing and gives
    /// `false` if `from` is no longer the space's last accepted revision.
    pub fn accept(
        &self,
        id: SpaceId,
        from: &[u8],
        hashes: &[(u64, [u8; 32])],
        last: &[u8],
    ) -> Result<bool, Error> {
        let tx = self.db.unchecked_transaction()?;
        let id = id.to_string();
        let changed = tx.execute(
            "UPDATE space SET accepted = ?3 WHERE space = ?1 AND accepted = ?2",
            params![id, from, last],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        for (revision, hash) in hashes {
            let revision = i64::try_from(*revision)
                .map_err(|_| Error::Malformed(format!("revision {revision}")))?;
            tx.execute(
                "INSERT INTO accepted_revision (space, revision, hash) VALUES (?1, ?2, ?3)
                 ON CONFLICT DO NOTHING",
                params![id, revision, hash.as_slice()],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// The rules, by folder: each one's folder, as its canonical path, and its space's name.
    pub fn rules(&self) -> Result<Vec<(String, String)>, Error> {
        let mut query = self
            .db
            .prepare("SELECT folder, space FROM folder_rule ORDER BY folder")?;
        let rows = query.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Records the rule that binds `folder`, a canonical path, and everything below it to the
    /// space named `space`, in place of the folder's earlier rule; gives that rule's space.
    pub fn set_rule(&self, folder: &str, space: &str) -> Result<Option<String>, Error> {
        let tx = self.db.unchecked_transaction()?;
        let earlier = tx
            .query_row(
                "SELECT space FROM folder_rule WHERE folder = ?1",
                params![folder],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute(
            "INSERT OR REPLACE INTO folder_rule (folder, space) VALUES (?1, ?2)",
            params![folder, space],
        )?;
        tx.commit()?;
        Ok(earlier)
    }

    /// Forgets the rule of `folder`, a canonical path; gives whether it had one.
    pub fn remove_rule(&self, folder: &str) -> Result<bool, Error> {
        let removed = self
            .db
            .execute("DELETE FROM folder_rule WHERE folder = ?1", params![folder])?;
        Ok(removed > 0)
    }

    /// The SHA-256 of the `.skald.toml` the device's user approved in `folder`, a canonical path.
    pub fn approved_config(&self, folder: &str) -> Result<Option<[u8; 32]>, Error> {
        let hash: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT hash FROM approved_config WHERE folder = ?1",
                params![folder],
                |row| row.get(0),
            )
            .optional()?;
        hash.map(|hash| {
            hash.try_into()
                .map_err(|_| Error::Malformed(format!("the approval of {folder:?}")))
        })
        .transpose()
    }

    /// Records that the device's user approved the `.skald.toml` in `folder`, a canonical path,
    /// whose bytes have the SHA-256 `hash`, in place of an earlier approval.
    pub fn approve_config(&self, folder: &str, hash: &[u8; 32]) -> Result<(), Error> {
        self.db.execute(
            "INSERT OR REPLACE INTO approved_config (folder, hash) VALUES (?1, ?2)",
            params![folder, hash.as_slice()],
        )?;
        Ok(())
    }

    /// The last `counter` the device gave content to each of `recipients` in `channel`; 0 for one
    /// it gave none.
    pub fn counters(&self, channel: ChannelId, recipients: &[DeviceId]) -> Result<Vec<u64>, Error> {
        let mut query = self
            .db
            .prepare("SELECT last FROM send_counter WHERE channel = ?1 AND recipient = ?2")?;
        recipients
            .iter()
            .map(|recipient| {
                let last: Option<i64> = query
                    .query_row(params![channel.to_string(), recipient.to_string()], |row| {
                        row.get(0)
                    })
                    .optional()?;
                let last = last.unwrap_or(0);
                u64::try_from(last).map_err(|_| Error::Malformed(format!("the counter {last}")))
            })
            .collect()
    }

    /// Adds `outgoing`, the envelopes of one message in `channel`, to the outbox, after those in
    /// it, and records `counter`, the counter each one's content has, as its recipient's last in
    /// `channel`, all at once. Adds nothing and gives `false` if a counter is not the one after
    /// the recipient's last: another run took it meanwhile.
    pub fn enqueue(&self, channel: ChannelId, outgoing: &[(Outgoing, u64)]) -> Result<bool, Error> {
        let tx = self.db.unchecked_transaction()?;
        let channel = channel.to_string();
        for (envelope, counter) in outgoing {
            let counter = i64::try_from(*counter)
                .map_err(|_| Error::Malformed(format!("the counter {counter}")))?;
            let recipient = envelope.recipient.to_string();
            let changed = if counter == 1 {
                tx.execute(
                    "INSERT INTO send_counter (channel, recipient, last) VALUES (?1, ?2, 1)
                     ON CONFLICT DO NOTHING",
                    params![channel, recipient],
                )?
            } else {
                tx.execute(
                    "UPDATE send_counter SET last = ?3
                     WHERE channel = ?1 AND recipient = ?2 AND last = ?3 - 1",
                    params![channel, recipient, counter],
                )?
            };
            if changed == 0 {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO outbox (msg_id, recipient, space, envelope) VALUES (?1, ?2, ?3, ?4)",
                params![
                    envelope.msg_id.to_string(),
                    recipient,
                    envelope.space.to_string(),
                    envelope.envelope
                ],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// The envelopes in the outbox, in the order they go out.
    pub fn outbox(&self) -> Result<Vec<Outgoing>, Error> {
        let mut query = self
            .db
            .prepare("SELECT msg_id, recipient, space, envelope FROM outbox ORDER BY position")?;
        let rows = query.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get(3)?,
            ))
        })?;
        rows.map(|row| {
            let (msg_id, recipient, space, envelope) = row?;
            let malformed = |what: &str, id: &str| Error::Malformed(format!("the {what} {id:?}"));
            Ok(Outgoing {
                msg_id: msg_id
                    .parse()
                    .map_err(|_| malformed("message ID", &msg_id))?,
                recipient: recipient
                    .parse()
                    .map_err(|_| malformed("device ID", &recipient))?,
                space: space.parse().map_err(|_| malformed("space ID", &space))?,
                envelope,
            })
        })
        .collect()
    }

    /// Takes the envelope of `msg_id` to `recipient` out of the outbox: the server answered it.
    pub fn remove_outgoing(&self, msg_id: MessageId, recipient: DeviceId) -> Result<(), Error> {
        self.db.execute(
            "DELETE FROM outbox WHERE msg_id = ?1 AND recipient = ?2",
            params![msg_id.to_string(), recipient.to_string()],
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
    fn spaces_are_kept_across_opens_with_their_pending_revision_until_taken() {
        let dir = TestDir::new("state-spaces");
        let state = State::open(&dir).unwrap();
        assert_eq!(state.spaces().unwrap(), vec![]);
        let (globex, acme) = (SpaceId::generate(), SpaceId::generate());
        state.add_space(globex, "globex", &[2, 2]).unwrap();
        state.add_space(acme, "acme", &[1, 1]).unwrap();
        assert!(
            state.add_space(SpaceId::generate(), "acme", &[3]).is_err(),
            "one name"
        );
        assert!(state.add_space(acme, "other", &[3]).is_err(), "one ID");
        state.clear_pending(acme, &[1, 2]).unwrap();
        assert!(state.spaces().unwrap()[0].pending.is_some(), "another one");
        state.clear_pending(acme, &[1, 1]).unwrap();

        let space = |id, name: &str, bytes: &[u8], pending: bool| Space {
            id,
            name: name.to_owned(),
            anchor: bytes.to_vec(),
            accepted: bytes.to_vec(),
            pending: pending.then(|| bytes.to_vec()),
            personal: false,
        };
        let state = State::open(&dir).unwrap();
        assert_eq!(
            state.spaces().unwrap(),
            vec![
                space(acme, "acme", &[1, 1], false),
                space(globex, "globex", &[2, 2], true),
            ]
        );
        state.remove_space(globex).unwrap();
        assert_eq!(
            state.spaces().unwrap(),
            vec![space(acme, "acme", &[1, 1], false)]
        );
        state
            .db
            .execute("UPDATE space SET space = 'not an ID'", [])
            .unwrap();
        let error = state.spaces().expect_err("malformed");
        assert!(matches!(error, Error::Malformed(_)), "{error:?}");
    }

    #[test]
    fn personal_space_is_pinned_once_and_kept_across_opens_until_dropped() {
        let dir = TestDir::new("state-personal");
        let state = State::open(&dir).unwrap();
        let (ada, acme) = (SpaceId::generate(), SpaceId::generate());
        state.add_space(acme, "acme", &[1]).unwrap();
        state.add_personal_space(ada, "ada", &[2]).unwrap();
        assert!(
            state
                .add_personal_space(SpaceId::generate(), "other", &[3])
                .is_err(),
            "a second personal space"
        );
        let personal = |state: &State| -> Vec<(String, bool)> {
            let spaces = state.spaces().unwrap();
            spaces.into_iter().map(|s| (s.name, s.personal)).collect()
        };
        let state = State::open(&dir).unwrap();
        assert_eq!(
            personal(&state),
            vec![("acme".to_owned(), false), ("ada".to_owned(), true)]
        );
        state.remove_space(ada).unwrap();
        state
            .add_personal_space(SpaceId::generate(), "other", &[3])
            .unwrap();
        assert_eq!(
            personal(&state),
            vec![("acme".to_owned(), false), ("other".to_owned(), true)]
        );
    }

    #[test]
    fn pending_revision_is_set_only_on_the_base_and_in_place_of_the_one_given() {
        let dir = TestDir::new("state-pending");
        let state = State::open(&dir).unwrap();
        let acme = SpaceId::generate();
        state.add_space(acme, "acme", &[1]).unwrap();
        let pending = |state: &State| state.spaces().unwrap()[0].pending.clone();
        let first_waits = |state: &State| state.spaces().unwrap()[0].first_waits();
        assert!(first_waits(&state), "revision 1 waits");
        state.clear_pending(acme, &[1]).unwrap();
        assert!(!first_waits(&state));

        assert!(
            !state.set_pending(acme, &[9], None, &[2]).unwrap(),
            "another base"
        );
        assert!(!state.set_pending(acme, &[1], Some(&[3]), &[2]).unwrap());
        assert_eq!(pending(&state), None);
        assert!(state.set_pending(acme, &[1], None, &[2]).unwrap());
        assert!(!first_waits(&state), "a later revision waits");
        assert!(
            !state.set_pending(acme, &[1], None, &[4]).unwrap(),
            "one waits"
        );
        assert!(state.set_pending(acme, &[1], Some(&[2]), &[4]).unwrap());
        assert_eq!(pending(&State::open(&dir).unwrap()), Some(vec![4]));
        assert!(
            !state
                .set_pending(SpaceId::generate(), &[1], Some(&[4]), &[5])
                .unwrap()
        );
        assert_eq!(pending(&state), Some(vec![4]));
    }

    #[test]
    fn accepted_revisions_are_recorded_only_after_the_last_accepted_one() {
        let dir = TestDir::new("state-accepted");
        let state = State::open(&dir).unwrap();
        let (acme, globex) = (SpaceId::generate(), SpaceId::generate());
        assert_eq!(state.accepted(acme).unwrap(), None);
        state.add_space(acme, "acme", &[1]).unwrap();
        state.add_space(globex, "globex", &[7]).unwrap();
        let accepted = |state: &State, id, last: &[u8], hashes: &[(u64, [u8; 32])]| {
            assert_eq!(
                state.accepted(id).unwrap(),
                Some(Accepted {
                    last: last.to_vec(),
                    hashes: hashes.iter().copied().collect(),
                })
            );
        };
        accepted(&state, acme, &[1], &[]);

        let hashes = [(1, [1; 32]), (2, [2; 32])];
        assert!(state.accept(acme, &[1], &hashes, &[3]).unwrap());
        // Another run accepted from revision 1 meanwhile: nothing is recorded, not even a hash
        // the state has none of yet.
        assert!(
            !state
                .accept(acme, &[1], &[(1, [1; 32]), (3, [8; 32])], &[4])
                .unwrap()
        );
        accepted(&state, acme, &[3], &hashes);

        let state = State::open(&dir).unwrap();
        accepted(&state, acme, &[3], &hashes);
        assert_eq!(state.spaces().unwrap()[0].anchor, vec![1]);
        accepted(&state, globex, &[7], &[]);
        // A hash already recorded stays.
        assert!(
            state
                .accept(acme, &[3], &[(2, [9; 32]), (3, [3; 32])], &[4])
                .unwrap()
        );
        accepted(
            &state,
            acme,
            &[4],
            &[(1, [1; 32]), (2, [2; 32]), (3, [3; 32])],
        );

        state.remove_space(acme).unwrap();
        assert_eq!(state.accepted(acme).unwrap(), None);
        state.add_space(acme, "acme", &[1]).unwrap();
        accepted(&state, acme, &[1], &[]);
        state
            .db
            .execute(
                "INSERT INTO accepted_revision (space, revision, hash) VALUES ('x', 1, zeroblob(32))",
                [],
            )
            .unwrap();
        for update in [
            "INSERT INTO accepted_revision (space, revision, hash) VALUES ('x', 2, zeroblob(31))",
            "INSERT INTO accepted_revision (space, revision, hash) VALUES ('x', 0, zeroblob(32))",
        ] {
            assert!(state.db.execute(update, []).is_err(), "{update}");
        }
    }

    /// Another run accepts revisions of a space while `accepted` reads it, between its reads of the
    /// last accepted revision and of the hashes: the read holds the revision and the hashes from
    /// before, or both from after.
    #[test]
    fn accepted_revision_and_hashes_are_read_at_once() {
        use rusqlite::trace::{TraceEvent, TraceEventCodes};
        use std::cell::RefCell;

        thread_local! {
            /// The state directory and the space of the other run, until it runs.
            static OTHER_RUN: RefCell<Option<(PathBuf, SpaceId)>> = const { RefCell::new(None) };
            /// What the other run's accepting gave.
            static RAN: RefCell<Option<Result<bool, String>>> = const { RefCell::new(None) };
        }
        fn other_run(event: TraceEvent<'_>) {
            let TraceEvent::Stmt(_, sql) = event else {
                return;
            };
            if !sql.contains("SELECT revision, hash") {
                return;
            }
            let Some((dir, id)) = OTHER_RUN.with(|run| run.borrow_mut().take()) else {
                return;
            };
            let accepted = State::open(&dir)
                .and_then(|state| {
                    state.db.busy_timeout(std::time::Duration::ZERO)?;
                    state.accept(id, &[1], &[(1, [1; 32])], &[2])
                })
                .map_err(|err| err.to_string());
            RAN.with(|ran| *ran.borrow_mut() = Some(accepted));
        }

        let dir = TestDir::new("state-accepted-at-once");
        let state = State::open(&dir).unwrap();
        let acme = SpaceId::generate();
        state.add_space(acme, "acme", &[1]).unwrap();
        OTHER_RUN.with(|run| *run.borrow_mut() = Some((dir.to_path_buf(), acme)));
        state
            .db
            .trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, Some(other_run));

        let read = state.accepted(acme).unwrap().expect("acme");

        state.db.trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, None);
        let ran = RAN.with(|ran| ran.borrow().clone());
        assert!(ran.is_some(), "no other run");
        let before = Accepted {
            last: vec![1],
            hashes: BTreeMap::new(),
        };
        let after = Accepted {
            last: vec![2],
            hashes: [(1, [1; 32])].into_iter().collect(),
        };
        assert!(
            read == before || read == after,
            "{read:?}, the other run: {ran:?}"
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

    #[test]
    fn outbox_keeps_its_envelopes_in_order_and_takes_each_counter_once() {
        let dir = TestDir::new("state-outbox");
        let state = State::open(&dir).unwrap();
        let (channel, space) = (ChannelId::generate(), SpaceId::generate());
        let (bob, carol) = (
            DeviceId::from_signing_key(&[1; 32]),
            DeviceId::from_signing_key(&[2; 32]),
        );
        let outgoing = |msg_id, recipient, envelope: &[u8]| Outgoing {
            msg_id,
            recipient,
            space,
            envelope: envelope.to_vec(),
        };
        let (first, second) = (MessageId::generate(), MessageId::generate());
        assert_eq!(state.counters(channel, &[bob, carol]).unwrap(), [0, 0]);

        let both = [
            (outgoing(first, bob, b"1b"), 1),
            (outgoing(first, carol, b"1c"), 1),
        ];
        assert!(state.enqueue(channel, &both).unwrap());
        // A message with a counter another run took, or one that is not the next: nothing of it
        // is added.
        for wrong in [1, 3] {
            let message = [
                (outgoing(second, carol, b"2c"), 2),
                (outgoing(second, bob, b"2b"), wrong),
            ];
            assert!(!state.enqueue(channel, &message).unwrap(), "{wrong}");
        }
        assert_eq!(state.counters(channel, &[bob, carol]).unwrap(), [1, 1]);
        assert!(
            state
                .enqueue(channel, &[(outgoing(second, bob, b"2b"), 2)])
                .unwrap()
        );
        assert_eq!(state.counters(channel, &[bob, carol]).unwrap(), [2, 1]);
        assert_eq!(state.counters(ChannelId::generate(), &[bob]).unwrap(), [0]);

        state.remove_outgoing(first, bob).unwrap();
        let state = State::open(&dir).unwrap();
        assert_eq!(
            state.outbox().unwrap(),
            [outgoing(first, carol, b"1c"), outgoing(second, bob, b"2b")]
        );
    }

    #[test]
    fn rules_are_kept_across_opens_one_per_folder() {
        let dir = TestDir::new("state-rules");
        let state = State::open(&dir).unwrap();
        assert_eq!(state.rules().unwrap(), []);
        assert_eq!(state.set_rule("/w/b", "clientb").unwrap(), None);
        assert_eq!(state.set_rule("/w/a", "acme").unwrap(), None);
        assert_eq!(
            state.set_rule("/w/a", "clienta").unwrap().as_deref(),
            Some("acme")
        );
        drop(state);

        let state = State::open(&dir).unwrap();
        let rule = |folder: &str, space: &str| (folder.to_owned(), space.to_owned());
        assert_eq!(
            state.rules().unwrap(),
            [rule("/w/a", "clienta"), rule("/w/b", "clientb")]
        );
        assert!(state.remove_rule("/w/b").unwrap());
        assert!(!state.remove_rule("/w/b").unwrap(), "removed already");
        assert!(!state.remove_rule("/w").unwrap(), "no rule of its own");
        assert_eq!(state.rules().unwrap(), [rule("/w/a", "clienta")]);
    }

    #[test]
    fn approval_is_kept_across_opens_one_per_folder() {
        let dir = TestDir::new("state-approvals");
        let state = State::open(&dir).unwrap();
        assert_eq!(state.approved_config("/w/repo").unwrap(), None);
        state.approve_config("/w/repo", &[1; 32]).unwrap();
        state.approve_config("/w/other", &[2; 32]).unwrap();
        state.approve_config("/w/repo", &[3; 32]).unwrap();
        drop(state);

        let state = State::open(&dir).unwrap();
        assert_eq!(state.approved_config("/w/repo").unwrap(), Some([3; 32]));
        assert_eq!(state.approved_config("/w/other").unwrap(), Some([2; 32]));
        assert_eq!(state.approved_config("/w").unwrap(), None);
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
