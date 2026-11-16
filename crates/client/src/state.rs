//! The client's local state: an SQLite database in the state directory. It holds no secret in the
//! clear: those go to [`crate::secrets`], and the device's MLS state is kept only sealed. Deleted
//! content is overwritten with zeros (`secure_delete`), and a save of the MLS state checkpoints
//! the WAL with `TRUNCATE`, so no older sealed state stays in the database file or its WAL.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use skald_crypto::SealedState;
use skald_proto::{ChannelId, DeviceId, MessageId, Scheme, SpaceId};

use crate::gaps::{Counters, Source};

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
    // The device's mailbox: its cursor, the highest `seq` it wrote; each entry it wrote that no run
    // has shown yet, by `seq`, as its `message` or `status` frame; the sending device and `msg_id`
    // of each message it showed; and the space and channel of each message it sent, which its
    // statuses report on.
    "ALTER TABLE device ADD COLUMN mailbox_cursor INTEGER NOT NULL DEFAULT 0
        CHECK (mailbox_cursor >= 0);
    CREATE TABLE inbox (
        seq INTEGER PRIMARY KEY CHECK (seq >= 1),
        entry BLOB NOT NULL
    ) STRICT;
    CREATE TABLE received (
        sender TEXT NOT NULL,
        msg_id TEXT NOT NULL,
        PRIMARY KEY (sender, msg_id)
    ) STRICT;
    CREATE TABLE sent (
        msg_id TEXT PRIMARY KEY,
        space TEXT NOT NULL,
        channel TEXT NOT NULL
    ) STRICT;",
    // The device's MLS state: each item by its name, sealed under the state key in key storage.
    "CREATE TABLE mls_state (
        name BLOB PRIMARY KEY,
        sealed BLOB NOT NULL
    ) STRICT",
    // The device's own Commits and Welcomes the server has not answered yet, in the order they go
    // out, by message ID, with their space and channel and the envelope's bytes.
    "CREATE TABLE mls_outbox (
        position INTEGER PRIMARY KEY,
        msg_id TEXT NOT NULL UNIQUE,
        space TEXT NOT NULL,
        channel TEXT NOT NULL,
        envelope BLOB NOT NULL
    ) STRICT",
    // Channel messages under `mls`: the outbox holds them without a recipient device; the last
    // `counter` the device gave its content in each channel, in place of one per recipient device;
    // and what decrypting each `mls` application message of the inbox in its channel's group gave,
    // sealed under the state key, until a run shows it.
    "CREATE TABLE outbox_next (
        position INTEGER PRIMARY KEY,
        msg_id TEXT NOT NULL,
        recipient TEXT,
        space TEXT NOT NULL,
        envelope BLOB NOT NULL,
        UNIQUE (msg_id, recipient)
    ) STRICT;
    INSERT INTO outbox_next (position, msg_id, recipient, space, envelope)
        SELECT position, msg_id, recipient, space, envelope FROM outbox;
    DROP TABLE outbox;
    ALTER TABLE outbox_next RENAME TO outbox;
    DROP TABLE send_counter;
    CREATE TABLE channel_counter (
        channel TEXT PRIMARY KEY,
        last INTEGER NOT NULL CHECK (last >= 1)
    ) STRICT;
    ALTER TABLE inbox ADD COLUMN opened BLOB;",
    // Gap detection: per channel, sending device and scheme (1 `mls`, 2 `hpke`), the counters the
    // device accepted, as `gaps::Counters` encodes them, with the `seq` of the entry it last
    // accepted; and, for each join of a channel's group, the `seq` of the Welcome's entry, or,
    // joining by itself, the `seq` after its cursor.
    // From now on `received` holds only messages without `counter`.
    "CREATE TABLE received_counter (
        channel TEXT NOT NULL,
        sender TEXT NOT NULL,
        scheme INTEGER NOT NULL CHECK (scheme IN (1, 2)),
        counters BLOB NOT NULL,
        seq INTEGER NOT NULL CHECK (seq >= 1),
        PRIMARY KEY (channel, sender, scheme)
    ) STRICT;
    CREATE TABLE channel_joined (
        channel TEXT NOT NULL,
        seq INTEGER NOT NULL CHECK (seq >= 1),
        PRIMARY KEY (channel, seq)
    ) STRICT;",
    // Revocations: each one the device holds as counting, as its CBOR bytes, whether the server
    // gave it, a Commit carried it or the device signed it; one the device signed is `pending` until
    // the server takes it. And whether the device last held each device it checked revoked, so it
    // warns when that changes.
    "CREATE TABLE revocation (
        revocation BLOB PRIMARY KEY,
        pending INTEGER NOT NULL DEFAULT 0 CHECK (pending IN (0, 1))
    ) STRICT;
    CREATE TABLE device_status (
        device TEXT PRIMARY KEY,
        revoked INTEGER NOT NULL CHECK (revoked IN (0, 1))
    ) STRICT;",
    // The channels whose group the device joins by itself even though the latest GroupInfo lists
    // it: one whose Welcome it refused or whose Commit it could not process, until it is in the
    // group of the channel's current epoch.
    "CREATE TABLE rejoin (
        channel TEXT PRIMARY KEY
    ) STRICT",
    // The epoch of each channel's group the device is in, with when it entered it, by the clock of
    // epochs' ages ([`crate::clock`]).
    "CREATE TABLE channel_epoch (
        channel TEXT PRIMARY KEY,
        epoch INTEGER NOT NULL CHECK (epoch >= 0),
        entered INTEGER NOT NULL CHECK (entered >= 0)
    ) STRICT",
    // Handing messages over to agents (`docs/protocol/delivery.md`, "Device to agent"): when the
    // device wrote each inbox entry, in Unix seconds, none for one an older client wrote; each agent
    // the device has run as in each space, with its cursor then, so each entry after it is for the
    // agent; and each message that passed the checks, by its entry's `seq`, with its space, its line
    // as the CLI shows it, sealed under the state key, until when it waits for an agent, in Unix
    // seconds, and whether the person's run has yet to show it, with each agent it waits for. A
    // message is kept until the person's run showed it and no agent waits for it.
    "ALTER TABLE inbox ADD COLUMN written INTEGER;
    CREATE TABLE agent (
        space TEXT NOT NULL,
        name TEXT NOT NULL,
        since INTEGER NOT NULL CHECK (since >= 0),
        PRIMARY KEY (space, name)
    ) STRICT;
    CREATE TABLE kept (
        seq INTEGER PRIMARY KEY CHECK (seq >= 1),
        space TEXT NOT NULL,
        line BLOB NOT NULL,
        expires INTEGER NOT NULL,
        person INTEGER NOT NULL CHECK (person IN (0, 1))
    ) STRICT;
    CREATE TABLE handover (
        seq INTEGER NOT NULL,
        agent TEXT NOT NULL,
        PRIMARY KEY (seq, agent)
    ) STRICT;",
    // The key chain: whether the device judged each epoch of a channel it entered clean, and the
    // links, by epoch and `prev`, it does not upload again: those the server took from it, listed
    // to it or refused.
    "CREATE TABLE epoch_judgment (
        channel TEXT NOT NULL,
        epoch INTEGER NOT NULL CHECK (epoch >= 0),
        clean INTEGER NOT NULL CHECK (clean IN (0, 1)),
        PRIMARY KEY (channel, epoch)
    ) STRICT;
    CREATE TABLE settled_link (
        channel TEXT NOT NULL,
        epoch INTEGER NOT NULL CHECK (epoch >= 0),
        prev INTEGER NOT NULL CHECK (prev >= 0),
        PRIMARY KEY (channel, epoch, prev)
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
    /// Under `hpke`, its recipient device; none under `mls`.
    pub recipient: Option<DeviceId>,
    pub space: SpaceId,
    /// Its bytes, as they are sent every time.
    pub envelope: Vec<u8>,
}

/// A channel message of the device's own under `mls`, as a step puts it in the outbox: its
/// envelope, its channel and its content's `counter` there.
#[derive(Debug, Clone, PartialEq)]
pub struct Counted {
    pub outgoing: Outgoing,
    pub channel: ChannelId,
    pub counter: u64,
}

/// A mailbox entry the device wrote that no run has shown yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub seq: u64,
    /// Its `message` or `status` frame.
    pub frame: Vec<u8>,
    /// What decrypting its `mls` application message gave, sealed, once the device did.
    pub opened: Option<Vec<u8>>,
}

/// How the device keeps a message that passed the checks until the person's run shows it and it
/// is handed over to `agents`, the agents it is for (`docs/protocol/delivery.md`, "Device to
/// agent").
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    pub space: SpaceId,
    /// Its line as the CLI shows it, sealed under the state key.
    pub line: Vec<u8>,
    /// How long it waits for an agent after the device wrote its entry, in seconds.
    pub wait: u64,
    pub agents: Vec<String>,
}

/// A Commit or Welcome of the device's own in the outbox.
#[derive(Debug, Clone, PartialEq)]
pub struct MlsOutgoing {
    pub msg_id: MessageId,
    pub space: SpaceId,
    pub channel: ChannelId,
    /// Its bytes, as they are sent every time.
    pub envelope: Vec<u8>,
}

/// What a step on the MLS state writes with the state it saves, in the same transaction.
#[derive(Debug, Default)]
pub struct MlsChanges {
    /// Envelopes it puts in the outbox, in order.
    pub queued: Vec<MlsOutgoing>,
    /// The message IDs of the envelopes it takes out of the outbox: the server answered them.
    pub answered: Vec<MessageId>,
    /// The `seq` of the mailbox entries it took in.
    pub taken: Vec<u64>,
    /// Channel messages it puts in the outbox, in order, after those in it; each one's counter
    /// becomes its channel's last.
    pub messages: Vec<Counted>,
    /// The `seq` of each mailbox entry whose `mls` application message it decrypted, and what that
    /// gave, sealed.
    pub opened: Vec<(u64, Vec<u8>)>,
    /// Revocations that count that a Commit it processed carried, as their CBOR bytes.
    pub revocations: Vec<Vec<u8>>,
    /// Each channel whose group it joined, with the `seq` of the Welcome's entry, or, joining by
    /// itself, the `seq` after the cursor.
    pub joined: Vec<(ChannelId, u64)>,
    /// Channels whose group the device joins by itself even though the latest GroupInfo lists it.
    pub rejoin: Vec<ChannelId>,
    /// Channels of [`MlsChanges::rejoin`] the device is back in.
    pub rejoined: Vec<ChannelId>,
    /// Each channel whose group it is in at an epoch, with when it entered that epoch; an epoch
    /// recorded already keeps its time.
    pub entered: Vec<(ChannelId, u64, u64)>,
    /// Each epoch of a channel it judged, and whether clean; an epoch judged already keeps its
    /// judgment.
    pub judged: Vec<(ChannelId, u64, bool)>,
}

impl MlsChanges {
    fn is_empty(&self) -> bool {
        self.queued.is_empty()
            && self.answered.is_empty()
            && self.taken.is_empty()
            && self.messages.is_empty()
            && self.opened.is_empty()
            && self.revocations.is_empty()
            && self.joined.is_empty()
            && self.rejoin.is_empty()
            && self.rejoined.is_empty()
            && self.entered.is_empty()
            && self.judged.is_empty()
    }
}

/// Why the state cannot be used.
#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    /// A newer client wrote the state: it has more migrations than this one knows.
    Newer(u32),
    /// The state holds what this client does not read, for the reason it holds.
    Malformed(String),
    /// Another run kept the WAL from being truncated after a save of the MLS state.
    WalBusy,
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
            Error::WalBusy => f.write_str(
                "another run kept the local state's WAL from being truncated: the older sealed \
                 MLS state stays in it until the next save",
            ),
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
        db.pragma_update(None, "secure_delete", true)?;
        let mode: String =
            db.pragma_update_and_check(None, "journal_mode", "wal", |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(Error::Malformed(format!(
                "the journal mode stays {mode:?}, not WAL"
            )));
        }
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

    /// Runs `step` on the device's sealed MLS state while no other run writes the state, and saves
    /// the state it gives, if any, in place of the old one, which it overwrites and deletes
    /// securely: once saved, the WAL is checkpointed into the database file and truncated. Fails
    /// with what `step` fails with, or what the state fails with, as text.
    pub fn mls_step<T>(
        &self,
        step: impl FnOnce(&SealedState) -> Result<(Option<SealedState>, T), String>,
    ) -> Result<T, String> {
        self.mls_change(|sealed| {
            let (saved, done) = step(sealed)?;
            Ok((saved, MlsChanges::default(), done))
        })
    }

    /// As [`State::mls_step`], `step` also giving the changes to the outbox and the inbox that go
    /// with the state it saves: all of them are written at once, or none.
    pub fn mls_change<T>(
        &self,
        step: impl FnOnce(&SealedState) -> Result<(Option<SealedState>, MlsChanges, T), String>,
    ) -> Result<T, String> {
        let failed = |err: Error| err.to_string();
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)
            .map_err(|err| failed(err.into()))?;
        let items = tx
            .prepare("SELECT name, sealed FROM mls_state ORDER BY name")
            .and_then(|mut query| {
                query
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .and_then(Iterator::collect::<Result<Vec<(Vec<u8>, Vec<u8>)>, _>>)
            })
            .map_err(|err| failed(err.into()))?;
        let (saved, changes, done) = step(&SealedState::from_items(items))?;
        if saved.is_none() && changes.is_empty() {
            return Ok(done);
        }
        let save = || -> Result<(), Error> {
            if let Some(saved) = &saved {
                tx.execute("DELETE FROM mls_state", [])?;
                let mut insert =
                    tx.prepare("INSERT INTO mls_state (name, sealed) VALUES (?1, ?2)")?;
                for (name, sealed) in saved.items() {
                    insert.execute(params![name, sealed])?;
                }
            }
            for queued in &changes.queued {
                tx.execute(
                    "INSERT INTO mls_outbox (msg_id, space, channel, envelope)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        queued.msg_id.to_string(),
                        queued.space.to_string(),
                        queued.channel.to_string(),
                        queued.envelope
                    ],
                )?;
            }
            for msg_id in &changes.answered {
                tx.execute(
                    "DELETE FROM mls_outbox WHERE msg_id = ?1",
                    params![msg_id.to_string()],
                )?;
            }
            for &seq in &changes.taken {
                tx.execute("DELETE FROM inbox WHERE seq = ?1", params![sql_seq(seq)?])?;
            }
            for message in &changes.messages {
                add_message(&tx, message)?;
            }
            for (seq, opened) in &changes.opened {
                tx.execute(
                    "UPDATE inbox SET opened = ?2 WHERE seq = ?1",
                    params![sql_seq(*seq)?, opened],
                )?;
            }
            for revocation in &changes.revocations {
                add_revocation(&tx, revocation, false)?;
            }
            for (channel, seq) in &changes.joined {
                tx.execute(
                    "INSERT INTO channel_joined (channel, seq) VALUES (?1, ?2)
                     ON CONFLICT DO NOTHING",
                    params![channel.to_string(), sql_seq(*seq)?],
                )?;
            }
            for channel in &changes.rejoin {
                tx.execute(
                    "INSERT OR IGNORE INTO rejoin (channel) VALUES (?1)",
                    params![channel.to_string()],
                )?;
            }
            for channel in &changes.rejoined {
                tx.execute(
                    "DELETE FROM rejoin WHERE channel = ?1",
                    params![channel.to_string()],
                )?;
            }
            for (channel, epoch, entered) in &changes.entered {
                tx.execute(
                    "INSERT INTO channel_epoch (channel, epoch, entered) VALUES (?1, ?2, ?3)
                     ON CONFLICT (channel) DO UPDATE
                     SET epoch = excluded.epoch, entered = excluded.entered
                     WHERE epoch != excluded.epoch",
                    params![channel.to_string(), sql_u64(*epoch)?, sql_u64(*entered)?],
                )?;
            }
            for (channel, epoch, clean) in &changes.judged {
                tx.execute(
                    "INSERT INTO epoch_judgment (channel, epoch, clean) VALUES (?1, ?2, ?3)
                     ON CONFLICT DO NOTHING",
                    params![channel.to_string(), sql_u64(*epoch)?, clean],
                )?;
            }
            Ok(())
        };
        save().map_err(failed)?;
        tx.commit().map_err(|err| failed(err.into()))?;
        if saved.is_some() {
            self.truncate_wal().map_err(failed)?;
        }
        Ok(done)
    }

    /// The channels whose group the device joins by itself even though the latest GroupInfo lists
    /// it ([`MlsChanges::rejoin`]).
    pub fn rejoining(&self) -> Result<Vec<ChannelId>, Error> {
        let mut query = self
            .db
            .prepare("SELECT channel FROM rejoin ORDER BY channel")?;
        let rows = query.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|channel| id("channel ID", &channel?)).collect()
    }

    /// The epoch of `channel`'s group the device last recorded it entered, with when
    /// ([`MlsChanges::entered`]).
    pub fn epoch_entered(&self, channel: ChannelId) -> Result<Option<(u64, u64)>, Error> {
        let held: Option<(i64, i64)> = self
            .db
            .query_row(
                "SELECT epoch, entered FROM channel_epoch WHERE channel = ?1",
                params![channel.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        held.map(|(epoch, entered)| {
            let read = |value: i64| {
                u64::try_from(value)
                    .map_err(|_| Error::Malformed(format!("the epoch of the channel {channel}")))
            };
            Ok((read(epoch)?, read(entered)?))
        })
        .transpose()
    }

    /// Whether the device judged each epoch of `channel` it judged clean ([`MlsChanges::judged`]).
    pub fn judgments(&self, channel: ChannelId) -> Result<BTreeMap<u64, bool>, Error> {
        let mut query = self
            .db
            .prepare("SELECT epoch, clean FROM epoch_judgment WHERE channel = ?1")?;
        let rows = query.query_map(params![channel.to_string()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get(1)?))
        })?;
        rows.map(|row| {
            let (epoch, clean) = row?;
            Ok((read_epoch(channel, epoch)?, clean))
        })
        .collect()
    }

    /// The links of `channel`, by epoch and `prev`, the device does not upload again.
    pub fn settled_links(&self, channel: ChannelId) -> Result<BTreeSet<(u64, u64)>, Error> {
        let mut query = self
            .db
            .prepare("SELECT epoch, prev FROM settled_link WHERE channel = ?1")?;
        let rows = query.query_map(params![channel.to_string()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.map(|row| {
            let (epoch, prev) = row?;
            Ok((read_epoch(channel, epoch)?, read_epoch(channel, prev)?))
        })
        .collect()
    }

    /// Keeps `links` of `channel`, by epoch and `prev`, from being uploaded again.
    pub fn settle_links(&self, channel: ChannelId, links: &[(u64, u64)]) -> Result<(), Error> {
        for &(epoch, prev) in links {
            self.db.execute(
                "INSERT INTO settled_link (channel, epoch, prev) VALUES (?1, ?2, ?3)
                 ON CONFLICT DO NOTHING",
                params![channel.to_string(), sql_u64(epoch)?, sql_u64(prev)?],
            )?;
        }
        Ok(())
    }

    /// The device's own Commits and Welcomes in the outbox, in the order they go out.
    pub fn mls_outbox(&self) -> Result<Vec<MlsOutgoing>, Error> {
        let rows = self.envelopes(
            "SELECT msg_id, space, channel, envelope FROM mls_outbox ORDER BY position",
        )?;
        rows.into_iter()
            .map(|([msg_id, space, channel], envelope)| {
                Ok(MlsOutgoing {
                    msg_id: id("message ID", &msg_id)?,
                    space: id("space ID", &space)?,
                    channel: id("channel ID", &channel)?,
                    envelope,
                })
            })
            .collect()
    }

    /// The rows `sql` selects, each as its first three columns, as text, and its fourth, an
    /// envelope's bytes.
    fn envelopes(&self, sql: &str) -> Result<Vec<EnvelopeRow>, Error> {
        let mut query = self.db.prepare(sql)?;
        let rows = query.query_map([], |row| {
            Ok(([row.get(0)?, row.get(1)?, row.get(2)?], row.get(3)?))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Copies the WAL into the database file and truncates it; fails if another run keeps it from
    /// finishing.
    fn truncate_wal(&self) -> Result<(), Error> {
        let busy: i64 = self
            .db
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy == 0 {
            Ok(())
        } else {
            Err(Error::WalBusy)
        }
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
        self.pairs("SELECT folder, space FROM folder_rule ORDER BY folder")
    }

    /// The rows `sql` selects, each as its first two columns.
    fn pairs<A: rusqlite::types::FromSql, B: rusqlite::types::FromSql>(
        &self,
        sql: &str,
    ) -> Result<Vec<(A, B)>, Error> {
        let mut query = self.db.prepare(sql)?;
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

    /// The last `counter` the device gave content in `channel` under `mls`; 0 before the first.
    pub fn channel_counter(&self, channel: ChannelId) -> Result<u64, Error> {
        let last: Option<i64> = self
            .db
            .query_row(
                "SELECT last FROM channel_counter WHERE channel = ?1",
                params![channel.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let last = last.unwrap_or(0);
        u64::try_from(last).map_err(|_| Error::Malformed(format!("the counter {last}")))
    }

    /// The space and channel of the message `msg_id` the device sent, if the state recorded it.
    pub fn sent(&self, msg_id: MessageId) -> Result<Option<(SpaceId, ChannelId)>, Error> {
        let row: Option<(String, String)> = self
            .db
            .query_row(
                "SELECT space, channel FROM sent WHERE msg_id = ?1",
                params![msg_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(space, channel)| {
            let malformed = || Error::Malformed(format!("the sent message {msg_id}"));
            Ok((
                space.parse().map_err(|_| malformed())?,
                channel.parse().map_err(|_| malformed())?,
            ))
        })
        .transpose()
    }

    /// The revocations the device holds, as their CBOR bytes, with whether each waits for the
    /// server to take it.
    pub fn revocations(&self) -> Result<Vec<(Vec<u8>, bool)>, Error> {
        self.pairs("SELECT revocation, pending FROM revocation ORDER BY rowid")
    }

    /// Keeps `revocations`, as their CBOR bytes, among those the device holds; those it signed
    /// wait for the server if `pending`.
    pub fn add_revocations(&self, revocations: &[Vec<u8>], pending: bool) -> Result<(), Error> {
        let tx = self.db.unchecked_transaction()?;
        for revocation in revocations {
            add_revocation(&tx, revocation, pending)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records that the server took `revocation`, which the device signed.
    pub fn revocation_taken(&self, revocation: &[u8]) -> Result<(), Error> {
        self.db.execute(
            "UPDATE revocation SET pending = 0 WHERE revocation = ?1",
            params![revocation],
        )?;
        Ok(())
    }

    /// Records whether each of `devices` is revoked; gives those the device held otherwise before,
    /// each with whether it is revoked now. A device it never held either way is recorded only.
    pub fn device_statuses(
        &self,
        devices: &[(DeviceId, bool)],
    ) -> Result<Vec<(DeviceId, bool)>, Error> {
        let tx = self.db.unchecked_transaction()?;
        let mut changed = Vec::new();
        for &(device, revoked) in devices {
            let id = device.to_string();
            let held: Option<bool> = tx
                .query_row(
                    "SELECT revoked FROM device_status WHERE device = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()?;
            if held == Some(revoked) {
                continue;
            }
            tx.execute(
                "INSERT INTO device_status (device, revoked) VALUES (?1, ?2)
                 ON CONFLICT (device) DO UPDATE SET revoked = excluded.revoked",
                params![id, revoked],
            )?;
            if held.is_some() {
                changed.push((device, revoked));
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// The device's mailbox cursor: the highest `seq` it wrote, 0 before the first.
    pub fn cursor(&self) -> Result<u64, Error> {
        let cursor: Option<i64> = self
            .db
            .query_row("SELECT mailbox_cursor FROM device", [], |row| row.get(0))
            .optional()?;
        let cursor = cursor.unwrap_or(0);
        u64::try_from(cursor).map_err(|_| Error::Malformed(format!("the cursor {cursor}")))
    }

    /// Writes `entries`, mailbox entries by `seq`, each as its frame, those after the cursor, with
    /// the new cursor and when it wrote them, all at once; gives the cursor.
    pub fn write_entries(&self, entries: &[(u64, Vec<u8>)]) -> Result<u64, Error> {
        let written = i64::try_from(unix_now()).unwrap_or(i64::MAX);
        let tx = self.db.unchecked_transaction()?;
        let read: i64 = tx.query_row("SELECT mailbox_cursor FROM device", [], |row| row.get(0))?;
        let mut cursor = read;
        for (seq, entry) in entries {
            let seq =
                i64::try_from(*seq).map_err(|_| Error::Malformed(format!("the seq {seq}")))?;
            if seq <= read {
                continue;
            }
            tx.execute(
                "INSERT INTO inbox (seq, entry, written) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
                params![seq, entry, written],
            )?;
            cursor = cursor.max(seq);
        }
        tx.execute("UPDATE device SET mailbox_cursor = ?1", params![cursor])?;
        tx.commit()?;
        u64::try_from(cursor).map_err(|_| Error::Malformed(format!("the cursor {cursor}")))
    }

    /// The mailbox entries the device wrote that no run has shown yet, in `seq` order.
    pub fn inbox(&self) -> Result<Vec<Entry>, Error> {
        let mut query = self
            .db
            .prepare("SELECT seq, entry, opened FROM inbox ORDER BY seq")?;
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.map(|row| {
            let (seq, frame, opened) = row?;
            let seq = u64::try_from(seq).map_err(|_| Error::Malformed(format!("the seq {seq}")))?;
            Ok(Entry { seq, frame, opened })
        })
        .collect()
    }

    /// Takes the entry `seq` out of the inbox: a run has shown or dropped it.
    pub fn remove_entry(&self, seq: u64) -> Result<(), Error> {
        self.db
            .execute("DELETE FROM inbox WHERE seq = ?1", params![sql_seq(seq)?])?;
        Ok(())
    }

    /// Whether the device showed a message `msg_id` from the device `sender`.
    pub fn received(&self, sender: DeviceId, msg_id: MessageId) -> Result<bool, Error> {
        let found = self
            .db
            .query_row(
                "SELECT 1 FROM received WHERE sender = ?1 AND msg_id = ?2",
                params![sender.to_string(), msg_id.to_string()],
                |_| Ok(()),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Records that the device received the message `msg_id` from the device `sender`, the entry
    /// `seq`, keeps it as `kept` says and takes the entry out of the inbox, at once.
    pub fn keep(
        &self,
        seq: u64,
        (sender, msg_id): (DeviceId, MessageId),
        kept: &Kept,
    ) -> Result<(), Error> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO received (sender, msg_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
            params![sender.to_string(), msg_id.to_string()],
        )?;
        add_kept(&tx, sql_seq(seq)?, kept)?;
        tx.execute("DELETE FROM inbox WHERE seq = ?1", params![sql_seq(seq)?])?;
        tx.commit()?;
        Ok(())
    }

    /// Records that the device has run as the agent `name` in the space `space`, unless it has:
    /// each entry after its cursor now is for the agent.
    pub fn add_agent(&self, space: SpaceId, name: &str) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO agent (space, name, since)
             SELECT ?1, ?2, mailbox_cursor FROM device WHERE true
             ON CONFLICT DO NOTHING",
            params![space.to_string(), name],
        )?;
        Ok(())
    }

    /// The agents the device had run as in the space `space` when it wrote the entry `seq`.
    pub fn agents(&self, space: SpaceId, seq: u64) -> Result<Vec<String>, Error> {
        let mut query = self
            .db
            .prepare("SELECT name FROM agent WHERE space = ?1 AND since < ?2 ORDER BY name")?;
        let names = query.query_map(params![space.to_string(), sql_seq(seq)?], |row| row.get(0))?;
        Ok(names.collect::<Result<_, _>>()?)
    }

    /// The kept messages the person's run has yet to show, by `seq`, each with its sealed line.
    pub fn for_person(&self) -> Result<Vec<(u64, Vec<u8>)>, Error> {
        self.lines(
            "SELECT seq, line FROM kept WHERE person = 1 ORDER BY seq",
            params![],
        )
    }

    /// Records that the person's run showed the kept message `seq`.
    pub fn shown_to_person(&self, seq: u64) -> Result<(), Error> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "UPDATE kept SET person = 0 WHERE seq = ?1",
            params![sql_seq(seq)?],
        )?;
        drop_done(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// The kept messages that wait for the agent `agent` in the space `space`, by `seq`, each with
    /// its sealed line, once those whose wait ended by `now`, in Unix seconds, are dropped for
    /// every agent.
    pub fn for_agent(
        &self,
        space: SpaceId,
        agent: &str,
        now: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, Error> {
        let now = i64::try_from(now).unwrap_or(i64::MAX);
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM handover WHERE seq IN (SELECT seq FROM kept WHERE expires <= ?1)",
            params![now],
        )?;
        drop_done(&tx)?;
        tx.commit()?;
        self.lines(
            "SELECT kept.seq, kept.line FROM kept JOIN handover ON handover.seq = kept.seq
             WHERE kept.space = ?1 AND handover.agent = ?2 ORDER BY kept.seq",
            params![space.to_string(), agent],
        )
    }

    /// Records that the kept message `seq` was handed over to the agent `agent`.
    pub fn handed_over(&self, seq: u64, agent: &str) -> Result<(), Error> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM handover WHERE seq = ?1 AND agent = ?2",
            params![sql_seq(seq)?, agent],
        )?;
        drop_done(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// The `seq` and sealed line of each kept message `sql` selects with `params`.
    fn lines(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<(u64, Vec<u8>)>, Error> {
        let mut query = self.db.prepare(sql)?;
        let rows = query.query_map(params, |row| Ok((row.get::<_, i64>(0)?, row.get(1)?)))?;
        rows.map(|row| {
            let (seq, line) = row?;
            let seq = u64::try_from(seq).map_err(|_| Error::Malformed(format!("the seq {seq}")))?;
            Ok((seq, line))
        })
        .collect()
    }

    /// The counters the device accepted from `source` that hold for the entry `seq`: none before
    /// the first, nor, under `mls`, if the device joined the channel's group after the entry it last
    /// accepted from `source` and by `seq`: by a Welcome before it, or by itself after its cursor.
    pub fn counters(&self, source: &Source, seq: u64) -> Result<Option<Counters>, Error> {
        let channel = source.channel.to_string();
        let row: Option<(Vec<u8>, i64)> = self
            .db
            .query_row(
                "SELECT counters, seq FROM received_counter
                 WHERE channel = ?1 AND sender = ?2 AND scheme = ?3",
                params![channel, source.device.to_string(), source.scheme as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((counters, accepted)) = row else {
            return Ok(None);
        };
        if source.scheme == Scheme::Mls {
            // A Welcome's entry is not `seq`'s; a join by itself takes the `seq` after the cursor.
            let joined: bool = self.db.query_row(
                "SELECT EXISTS (SELECT 1 FROM channel_joined
                 WHERE channel = ?1 AND seq > ?2 AND seq <= ?3)",
                params![channel, accepted, sql_seq(seq)?],
                |row| row.get(0),
            )?;
            if joined {
                return Ok(None);
            }
        }
        Counters::from_bytes(&counters).map(Some).ok_or_else(|| {
            Error::Malformed(format!("the counters of the device {}", source.device))
        })
    }

    /// Records `counters` as those the device accepted from `source` with the entry `seq`, keeps
    /// its message as `kept` says and takes the entry out of the inbox, at once.
    pub fn keep_counted(
        &self,
        seq: u64,
        (source, counters): (&Source, &Counters),
        kept: &Kept,
    ) -> Result<(), Error> {
        let seq = sql_seq(seq)?;
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO received_counter (channel, sender, scheme, counters, seq)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT DO UPDATE SET counters = excluded.counters, seq = excluded.seq",
            params![
                source.channel.to_string(),
                source.device.to_string(),
                source.scheme as i64,
                counters.to_bytes(),
                seq
            ],
        )?;
        add_kept(&tx, seq, kept)?;
        tx.execute("DELETE FROM inbox WHERE seq = ?1", params![seq])?;
        tx.commit()?;
        Ok(())
    }

    /// The envelopes in the outbox, in the order they go out.
    pub fn outbox(&self) -> Result<Vec<Outgoing>, Error> {
        let mut query = self
            .db
            .prepare("SELECT msg_id, recipient, space, envelope FROM outbox ORDER BY position")?;
        let rows = query.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get(3)?,
            ))
        })?;
        rows.map(|row| {
            let (msg_id, recipient, space, envelope) = row?;
            Ok(Outgoing {
                msg_id: id("message ID", &msg_id)?,
                recipient: recipient
                    .map(|recipient| id("device ID", &recipient))
                    .transpose()?,
                space: id("space ID", &space)?,
                envelope,
            })
        })
        .collect()
    }

    /// Takes the envelope of `msg_id` to `recipient`, none under `mls`, out of the outbox: the
    /// server answered it.
    pub fn remove_outgoing(
        &self,
        msg_id: MessageId,
        recipient: Option<DeviceId>,
    ) -> Result<(), Error> {
        self.db.execute(
            "DELETE FROM outbox WHERE msg_id = ?1 AND recipient IS ?2",
            params![
                msg_id.to_string(),
                recipient.map(|device| device.to_string())
            ],
        )?;
        Ok(())
    }
}

/// `seq` as SQLite holds it.
fn sql_seq(seq: u64) -> Result<i64, Error> {
    i64::try_from(seq).map_err(|_| Error::Malformed(format!("the seq {seq}")))
}

/// Now, in Unix seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Keeps the message of the inbox entry `seq` for the person and the agents `kept` names, in `tx`;
/// it waits for them `kept.wait` after the device wrote the entry, or after now if it did not
/// record when.
fn add_kept(tx: &Transaction, seq: i64, kept: &Kept) -> Result<(), Error> {
    let now = i64::try_from(unix_now()).unwrap_or(i64::MAX);
    let wait = i64::try_from(kept.wait).unwrap_or(i64::MAX);
    tx.execute(
        "INSERT INTO kept (seq, space, line, expires, person)
         SELECT ?1, ?2, ?3, COALESCE((SELECT written FROM inbox WHERE seq = ?1), ?4) + ?5, 1
         WHERE true ON CONFLICT DO NOTHING",
        params![seq, kept.space.to_string(), kept.line, now, wait],
    )?;
    for agent in &kept.agents {
        tx.execute(
            "INSERT INTO handover (seq, agent) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
            params![seq, agent],
        )?;
    }
    Ok(())
}

/// Drops, in `tx`, each kept message the person's run showed that no agent waits for.
fn drop_done(tx: &Transaction) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM kept WHERE person = 0 AND seq NOT IN (SELECT seq FROM handover)",
        [],
    )?;
    Ok(())
}

/// `value` as SQLite stores it; fails past `i64`.
fn sql_u64(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::Malformed(format!("the number {value}")))
}

/// Adds `message` to the outbox, after those in it, records its counter as its channel's last and
/// its space and channel as those of its message ID, in `tx`; fails unless the counter is the one
/// after the channel's last.
fn add_message(tx: &Transaction, message: &Counted) -> Result<(), Error> {
    let Counted {
        outgoing,
        channel,
        counter,
    } = message;
    let channel = channel.to_string();
    let last =
        i64::try_from(*counter).map_err(|_| Error::Malformed(format!("the counter {counter}")))?;
    let changed = if last == 1 {
        tx.execute(
            "INSERT INTO channel_counter (channel, last) VALUES (?1, 1) ON CONFLICT DO NOTHING",
            params![channel],
        )?
    } else {
        tx.execute(
            "UPDATE channel_counter SET last = ?2 WHERE channel = ?1 AND last = ?2 - 1",
            params![channel, last],
        )?
    };
    if changed == 0 {
        return Err(Error::Malformed(format!(
            "the counter {counter} is not the next in the channel {channel}"
        )));
    }
    let (msg_id, space) = (outgoing.msg_id.to_string(), outgoing.space.to_string());
    tx.execute(
        "INSERT INTO outbox (msg_id, recipient, space, envelope) VALUES (?1, ?2, ?3, ?4)",
        params![
            msg_id,
            outgoing.recipient.map(|device| device.to_string()),
            space,
            outgoing.envelope
        ],
    )?;
    tx.execute(
        "INSERT INTO sent (msg_id, space, channel) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
        params![msg_id, space, channel],
    )?;
    Ok(())
}

/// `epoch`, as SQLite keeps an epoch of `channel`.
fn read_epoch(channel: ChannelId, epoch: i64) -> Result<u64, Error> {
    u64::try_from(epoch)
        .map_err(|_| Error::Malformed(format!("an epoch of the channel {channel}: {epoch}")))
}

/// A row of an outbox: three IDs, as text, and an envelope's bytes.
type EnvelopeRow = ([String; 3], Vec<u8>);

/// `text`, the `what` the state holds, parsed.
fn id<T: std::str::FromStr>(what: &str, text: &str) -> Result<T, Error> {
    text.parse()
        .map_err(|_| Error::Malformed(format!("the {what} {text:?}")))
}

/// Keeps `revocation`, as its CBOR bytes, in `tx`, waiting for the server if `pending`; one held
/// already keeps waiting only while both wait.
fn add_revocation(tx: &Transaction, revocation: &[u8], pending: bool) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO revocation (revocation, pending) VALUES (?1, ?2)
         ON CONFLICT (revocation) DO UPDATE SET pending = pending AND excluded.pending",
        params![revocation, pending],
    )?;
    Ok(())
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
pub(crate) mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    /// Puts `messages` in the outbox of `state`, as a step on the MLS state that leaves it as it
    /// is.
    pub(crate) fn put(state: &State, messages: &[&Counted]) -> Result<(), String> {
        state.mls_change(|_| {
            let changes = MlsChanges {
                messages: messages.iter().map(|&message| message.clone()).collect(),
                ..MlsChanges::default()
            };
            Ok((None, changes, ()))
        })
    }

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

    /// The messages a step puts in the outbox, each with its channel's next counter, go out in
    /// order; a counter that is not the next fails the step, which writes nothing.
    #[test]
    fn outbox_keeps_its_envelopes_in_order_and_takes_each_counter_once() {
        let dir = TestDir::new("state-outbox");
        let state = State::open(&dir).unwrap();
        let (channel, other, space) = (
            ChannelId::generate(),
            ChannelId::generate(),
            SpaceId::generate(),
        );
        let message = |channel, counter, envelope: &[u8]| Counted {
            outgoing: Outgoing {
                msg_id: MessageId::generate(),
                recipient: None,
                space,
                envelope: envelope.to_vec(),
            },
            channel,
            counter,
        };
        assert_eq!(state.channel_counter(channel).unwrap(), 0);
        let (first, elsewhere) = (message(channel, 1, b"1"), message(other, 1, b"o1"));

        put(&state, &[&first, &elsewhere]).unwrap();
        // A counter another run took, or one that is not the next: nothing of the step is added.
        for wrong in [1, 3] {
            let next = message(other, 2, b"o2");
            let failed = put(&state, &[&next, &message(channel, wrong, b"2")]);
            assert!(failed.is_err(), "{wrong}: {failed:?}");
        }
        assert_eq!(state.channel_counter(channel).unwrap(), 1);
        assert_eq!(state.channel_counter(other).unwrap(), 1);
        let second = message(channel, 2, b"2");
        put(&state, &[&second]).unwrap();
        assert_eq!(state.channel_counter(channel).unwrap(), 2);
        assert_eq!(state.channel_counter(ChannelId::generate()).unwrap(), 0);

        state.remove_outgoing(first.outgoing.msg_id, None).unwrap();
        let state = State::open(&dir).unwrap();
        assert_eq!(
            state.outbox().unwrap(),
            [elsewhere.outgoing, second.outgoing]
        );
    }

    /// An `hpke` envelope an older client left in the outbox keeps its place and its recipient
    /// device, and leaves once the server answered it; the counters per recipient device go.
    #[test]
    fn hpke_envelopes_of_an_older_client_keep_their_recipient() {
        let dir = TestDir::new("state-outbox-hpke");
        let older = MIGRATIONS.len() - 1;
        let db = Connection::open(dir.join(FILE)).unwrap();
        for migration in &MIGRATIONS[..older] {
            db.execute_batch(migration).unwrap();
        }
        let (msg_id, space) = (MessageId::generate(), SpaceId::generate());
        let bob = DeviceId::from_signing_key(&[1; 32]);
        db.execute_batch(&format!("PRAGMA user_version = {older};"))
            .unwrap();
        db.execute(
            "INSERT INTO outbox (position, msg_id, recipient, space, envelope)
             VALUES (7, ?1, ?2, ?3, ?4)",
            params![
                msg_id.to_string(),
                bob.to_string(),
                space.to_string(),
                vec![1_u8]
            ],
        )
        .unwrap();
        drop(db);
        let left = Outgoing {
            msg_id,
            recipient: Some(bob),
            space,
            envelope: vec![1],
        };

        let state = State::open(&dir).unwrap();

        assert_eq!(state.outbox().unwrap(), [left]);
        let counters: i64 = (state.db)
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name = 'send_counter'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(counters, 0, "the counters per recipient device are dropped");
        state.remove_outgoing(msg_id, None).unwrap();
        assert_eq!(state.outbox().unwrap().len(), 1, "taken out as an mls one");
        state.remove_outgoing(msg_id, Some(bob)).unwrap();
        assert_eq!(state.outbox().unwrap(), []);
    }

    #[test]
    fn sent_messages_keep_their_space_and_channel() {
        let dir = TestDir::new("state-sent");
        let state = State::open(&dir).unwrap();
        let (channel, space, msg_id) = (
            ChannelId::generate(),
            SpaceId::generate(),
            MessageId::generate(),
        );
        let message = Counted {
            outgoing: Outgoing {
                msg_id,
                recipient: None,
                space,
                envelope: vec![1],
            },
            channel,
            counter: 1,
        };
        assert_eq!(state.sent(msg_id).unwrap(), None);

        put(&state, &[&message]).unwrap();
        state.remove_outgoing(msg_id, None).unwrap();

        let state = State::open(&dir).unwrap();
        assert_eq!(state.sent(msg_id).unwrap(), Some((space, channel)));
        assert_eq!(state.sent(MessageId::generate()).unwrap(), None);
    }

    #[test]
    fn mailbox_entries_after_the_cursor_wait_in_the_inbox_until_shown() {
        let dir = TestDir::new("state-inbox");
        let state = State::open(&dir).unwrap();
        state
            .add_device(DeviceId::from_signing_key(&[1; 32]), "laptop", "0190")
            .unwrap();
        assert_eq!(state.cursor().unwrap(), 0);
        let entry = |seq: u64| (seq, vec![u8::try_from(seq).unwrap()]);
        let waiting = |seq: u64| Entry {
            seq,
            frame: entry(seq).1,
            opened: None,
        };

        assert_eq!(state.write_entries(&[entry(2), entry(1)]).unwrap(), 2);
        // Those up to the cursor are not written again; the cursor never goes back.
        assert_eq!(state.write_entries(&[entry(1), entry(4)]).unwrap(), 4);
        assert_eq!(state.write_entries(&[entry(3)]).unwrap(), 4);
        let state = State::open(&dir).unwrap();
        assert_eq!(state.cursor().unwrap(), 4);
        assert_eq!(state.inbox().unwrap(), [waiting(1), waiting(2), waiting(4)]);

        let (sender, msg_id) = (DeviceId::from_signing_key(&[2; 32]), MessageId::generate());
        assert!(!state.received(sender, msg_id).unwrap());
        state.keep(1, (sender, msg_id), &kept(&[])).unwrap();
        state.remove_entry(4).unwrap();

        let state = State::open(&dir).unwrap();
        assert_eq!(state.inbox().unwrap(), [waiting(2)]);
        assert!(state.received(sender, msg_id).unwrap());
        assert!(!state.received(sender, MessageId::generate()).unwrap());
        let other = DeviceId::from_signing_key(&[3; 32]);
        assert!(!state.received(other, msg_id).unwrap());
    }

    /// What keeps a message in `space_of_kept()` for `agents`, its line `b"line"`, for a day.
    pub(crate) fn kept(agents: &[&str]) -> Kept {
        Kept {
            space: space_of_kept(),
            line: b"line".to_vec(),
            wait: 86_400,
            agents: agents.iter().map(|&agent| agent.to_owned()).collect(),
        }
    }

    /// The space of [`kept`].
    fn space_of_kept() -> SpaceId {
        "0190b6a2-1f3c-7d4e-8a5b-6c7d8e9f0a1b".parse().unwrap()
    }

    /// An agent the device has run as in a space gets each entry written after that, there only.
    /// A kept message waits for the person's run and each agent it is for until each takes it, and
    /// for an agent until its wait after its entry was written ends; then the state drops it.
    #[test]
    fn kept_messages_wait_for_the_person_and_their_agents_until_handed_over_or_expired() {
        let dir = TestDir::new("state-handover");
        let state = State::open(&dir).unwrap();
        state
            .add_device(DeviceId::from_signing_key(&[1; 32]), "laptop", "0190")
            .unwrap();
        let (space, other) = (space_of_kept(), SpaceId::generate());
        state.write_entries(&[(1, vec![1])]).unwrap();
        state.add_agent(space, "claude").unwrap();
        state.write_entries(&[(2, vec![2]), (3, vec![3])]).unwrap();
        // Running again keeps when it first ran.
        state.add_agent(space, "claude").unwrap();
        state.add_agent(space, "codex").unwrap();
        state.add_agent(other, "gemini").unwrap();
        assert!(state.agents(space, 1).unwrap().is_empty());
        assert_eq!(state.agents(space, 2).unwrap(), ["claude"]);
        assert_eq!(state.agents(space, 4).unwrap(), ["claude", "codex"]);
        assert_eq!(state.agents(other, 4).unwrap(), ["gemini"]);
        let sender = DeviceId::from_signing_key(&[2; 32]);
        state
            .keep(
                2,
                (sender, MessageId::generate()),
                &kept(&["claude", "codex"]),
            )
            .unwrap();
        let mut short = kept(&["claude"]);
        short.wait = 10;
        state
            .keep(3, (sender, MessageId::generate()), &short)
            .unwrap();
        let now = unix_now();
        let lines = |seqs: &[u64]| -> Vec<(u64, Vec<u8>)> {
            seqs.iter().map(|&seq| (seq, b"line".to_vec())).collect()
        };

        let state = State::open(&dir).unwrap();
        assert_eq!(state.for_person().unwrap(), lines(&[2, 3]));
        assert_eq!(
            state.for_agent(space, "claude", now).unwrap(),
            lines(&[2, 3])
        );
        assert!(state.for_agent(other, "claude", now).unwrap().is_empty());
        state.handed_over(2, "claude").unwrap();
        assert_eq!(state.for_agent(space, "claude", now).unwrap(), lines(&[3]));
        assert_eq!(state.for_agent(space, "codex", now).unwrap(), lines(&[2]));
        state.shown_to_person(2).unwrap();
        state.shown_to_person(3).unwrap();
        assert!(state.for_person().unwrap().is_empty());
        // The short wait ends: the message is dropped for every agent.
        assert!(
            state
                .for_agent(space, "claude", now + 11)
                .unwrap()
                .is_empty()
        );
        state.handed_over(2, "codex").unwrap();

        let kept: i64 = state
            .db
            .query_row("SELECT count(*) FROM kept", [], |row| row.get(0))
            .unwrap();
        assert_eq!(kept, 0, "a message no one waits for is kept");
    }

    /// The counters accepted from a sending device are kept per channel and scheme with the entry
    /// that took them out of the inbox. Under `mls`, a Welcome to the channel's group after that
    /// entry drops them for the entries after the Welcome; under `hpke`, it does not.
    #[test]
    fn counters_are_kept_per_source_until_the_device_joins_the_channel_again() {
        let dir = TestDir::new("state-counters");
        let state = State::open(&dir).unwrap();
        state
            .add_device(DeviceId::from_signing_key(&[1; 32]), "laptop", "0190")
            .unwrap();
        state.write_entries(&[(3, vec![3]), (4, vec![4])]).unwrap();
        let (channel, other) = (ChannelId::generate(), ChannelId::generate());
        let source = |channel, scheme| Source {
            channel,
            device: DeviceId::from_signing_key(&[2; 32]),
            scheme,
        };
        let (mls, hpke) = (source(channel, Scheme::Mls), source(channel, Scheme::Hpke));
        let kept = Counters {
            highest: 7,
            missing: vec![(2, 3)],
        };
        assert_eq!(state.counters(&mls, 5).unwrap(), None);

        state
            .keep_counted(3, (&mls, &kept), &self::kept(&[]))
            .unwrap();
        state
            .keep_counted(4, (&hpke, &Counters::default()), &self::kept(&[]))
            .unwrap();

        let state = State::open(&dir).unwrap();
        assert!(state.inbox().unwrap().is_empty());
        assert_eq!(state.counters(&mls, 5).unwrap(), Some(kept.clone()));
        assert_eq!(state.counters(&hpke, 5).unwrap(), Some(Counters::default()));
        assert_eq!(
            state.counters(&source(other, Scheme::Mls), 5).unwrap(),
            None
        );
        state
            .mls_change(|_| {
                let joined = MlsChanges {
                    joined: vec![(channel, 5)],
                    ..MlsChanges::default()
                };
                Ok((None, joined, ()))
            })
            .unwrap();
        assert_eq!(state.counters(&mls, 4).unwrap(), Some(kept.clone()));
        // A join by itself takes the `seq` after the cursor: the first entry after it.
        assert_eq!(state.counters(&mls, 5).unwrap(), None);
        assert_eq!(state.counters(&mls, 6).unwrap(), None);
        assert_eq!(state.counters(&hpke, 6).unwrap(), Some(Counters::default()));
        state
            .keep_counted(6, (&mls, &kept), &self::kept(&[]))
            .unwrap();
        assert_eq!(state.counters(&mls, 7).unwrap(), Some(kept.clone()));

        // Two Welcomes after the entry last accepted: an entry between them follows the first.
        state
            .mls_change(|_| {
                let joined = MlsChanges {
                    joined: vec![(channel, 8), (channel, 12)],
                    ..MlsChanges::default()
                };
                Ok((None, joined, ()))
            })
            .unwrap();
        assert_eq!(state.counters(&mls, 7).unwrap(), Some(kept));
        assert_eq!(state.counters(&mls, 9).unwrap(), None);
        assert_eq!(state.counters(&mls, 13).unwrap(), None);
    }

    /// The channels to rejoin are written with the MLS state and kept across opens until the
    /// device is back in; a change that fails writes none.
    #[test]
    fn channels_to_rejoin_are_kept_until_the_device_is_back_in() {
        let dir = TestDir::new("state-rejoin");
        let state = State::open(&dir).unwrap();
        let (first, second) = (ChannelId::generate(), ChannelId::generate());
        let change = |changes: MlsChanges| state.mls_change(|_| Ok((None, changes, ())));
        assert_eq!(state.rejoining().unwrap(), []);
        change(MlsChanges {
            rejoin: vec![first, second, first],
            ..MlsChanges::default()
        })
        .unwrap();
        let failed = state.mls_change::<()>(|_| Err("refused".to_owned()));
        assert!(failed.is_err());
        let mut both = vec![first, second];
        both.sort();
        assert_eq!(State::open(&dir).unwrap().rejoining().unwrap(), both);

        change(MlsChanges {
            rejoined: vec![first],
            ..MlsChanges::default()
        })
        .unwrap();
        assert_eq!(State::open(&dir).unwrap().rejoining().unwrap(), [second]);
    }

    /// The epoch the device entered in each channel is kept across opens with when it entered it;
    /// the same epoch again keeps its first time, a later one replaces it.
    #[test]
    fn entered_epochs_keep_their_first_time() {
        let dir = TestDir::new("state-epochs");
        let state = State::open(&dir).unwrap();
        let (first, second) = (ChannelId::generate(), ChannelId::generate());
        let enter = |entered: Vec<(ChannelId, u64, u64)>| {
            let changes = MlsChanges {
                entered,
                ..MlsChanges::default()
            };
            state.mls_change(|_| Ok((None, changes, ()))).unwrap();
        };
        assert_eq!(state.epoch_entered(first).unwrap(), None);
        enter(vec![(first, 1, 100), (second, 3, 150)]);
        enter(vec![(first, 1, 200)]);
        let reopened = State::open(&dir).unwrap();
        assert_eq!(reopened.epoch_entered(first).unwrap(), Some((1, 100)));
        enter(vec![(first, 2, 300)]);
        assert_eq!(state.epoch_entered(first).unwrap(), Some((2, 300)));
        assert_eq!(state.epoch_entered(second).unwrap(), Some((3, 150)));
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

    /// The bytes of the state's files in `dir`: its database, its journal and its WAL.
    fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
        ["", "-journal", "-wal"]
            .iter()
            .filter_map(|suffix| {
                let name = format!("{FILE}{suffix}");
                fs::read(dir.join(&name)).ok().map(|bytes| (name, bytes))
            })
            .collect()
    }

    fn sealed(items: &[(&[u8], &[u8])]) -> SealedState {
        SealedState::from_items(
            items
                .iter()
                .map(|(name, sealed)| (name.to_vec(), sealed.to_vec())),
        )
    }

    /// A step gets the saved MLS state and saves the one it gives; a step that gives none, or
    /// fails, leaves it as it was.
    #[test]
    fn mls_step_gets_the_saved_state_and_saves_the_one_it_gives() {
        let dir = TestDir::new("state-mls-step");
        let state = State::open(&dir).unwrap();
        let first = sealed(&[(b"a", &[1; 40]), (b"b", &[2; 40])]);
        let got = state
            .mls_step(|old| Ok((Some(first.clone()), old.clone())))
            .unwrap();
        assert_eq!(got, SealedState::default(), "a new device has none");

        let read = |state: &State| state.mls_step(|old| Ok((None, old.clone()))).unwrap();
        assert_eq!(read(&state), first);
        let failed = state.mls_step::<()>(|_| Err("refused".to_owned()));
        assert_eq!(failed, Err("refused".to_owned()));
        let second = sealed(&[(b"c", &[3; 40])]);
        state.mls_step(|_| Ok((Some(second.clone()), ()))).unwrap();
        assert_eq!(read(&State::open(&dir).unwrap()), second);
    }

    /// A step's changes to the outbox and the inbox are written with the state it saves: the
    /// envelopes it queues go out after those queued before, those answered and the entries taken
    /// in leave, and an entry keeps what decrypting it gave; a change that fails writes none of
    /// them, nor the state.
    #[test]
    fn mls_changes_are_written_with_the_state_or_not_at_all() {
        let dir = TestDir::new("state-mls-changes");
        let state = State::open(&dir).unwrap();
        let read = |state: &State| state.mls_step(|old| Ok((None, old.clone()))).unwrap();
        let seqs = |state: &State| -> Vec<(u64, Option<Vec<u8>>)> {
            state
                .inbox()
                .unwrap()
                .into_iter()
                .map(|entry| (entry.seq, entry.opened))
                .collect()
        };
        let device = DeviceId::from_signing_key(&[7; 32]);
        state.add_device(device, "ada-laptop", "0190").unwrap();
        state
            .write_entries(&[(1, vec![1]), (2, vec![2]), (3, vec![3])])
            .unwrap();
        let outgoing = |byte| MlsOutgoing {
            msg_id: MessageId::generate(),
            space: SpaceId::generate(),
            channel: ChannelId::generate(),
            envelope: vec![byte],
        };
        let (first, second, third) = (outgoing(1), outgoing(2), outgoing(3));
        let changes =
            |queued: &[&MlsOutgoing], answered: &[&MlsOutgoing], taken: &[u64]| MlsChanges {
                queued: queued.iter().map(|&queued| queued.clone()).collect(),
                answered: answered.iter().map(|answered| answered.msg_id).collect(),
                taken: taken.to_vec(),
                ..MlsChanges::default()
            };
        let saved = sealed(&[(b"a", &[1; 40])]);

        state
            .mls_change(|_| {
                Ok((
                    Some(saved.clone()),
                    changes(&[&first, &second], &[], &[2]),
                    (),
                ))
            })
            .unwrap();
        state
            .mls_change(|_| {
                let opened = MlsChanges {
                    opened: vec![(3, vec![9])],
                    ..changes(&[&third], &[&first], &[1])
                };
                Ok((None, opened, ()))
            })
            .unwrap();

        let state = State::open(&dir).unwrap();
        assert_eq!(state.mls_outbox().unwrap(), [second.clone(), third.clone()]);
        assert_eq!(seqs(&state), [(3, Some(vec![9]))]);
        assert_eq!(read(&state), saved);

        // The same message ID again fails the change: nothing of it is written.
        let other = sealed(&[(b"b", &[2; 40])]);
        let failed = state.mls_change(|_| {
            let opened = MlsChanges {
                opened: vec![(3, vec![8])],
                ..changes(&[&second], &[&third], &[])
            };
            Ok((Some(other.clone()), opened, ()))
        });
        assert!(failed.is_err(), "{failed:?}");
        assert_eq!(state.mls_outbox().unwrap(), [second, third]);
        assert_eq!(seqs(&state), [(3, Some(vec![9]))]);
        assert_eq!(read(&state), saved);
    }

    /// Once a save returns, the state it replaced is in neither the database file nor its journal
    /// or WAL, while another run holds the state open, and the WAL is empty.
    #[test]
    fn mls_save_leaves_no_older_state_in_the_files() {
        let dir = TestDir::new("state-mls-save");
        let state = State::open(&dir).unwrap();
        // Old items over several pages, each its own bytes; the new state is smaller.
        let old: Vec<Vec<u8>> = (1..=40u8)
            .map(|item| (0..200u8).map(|byte| byte.wrapping_mul(item) | 1).collect())
            .collect();
        let new = vec![0x5a; 60];
        let names: Vec<Vec<u8>> = (0..old.len()).map(|n| format!("item:{n}").into()).collect();
        let items = names
            .iter()
            .zip(&old)
            .map(|(name, bytes)| (name.clone(), bytes.clone()));
        let old_state = SealedState::from_items(items);
        state
            .mls_step(|_| Ok((Some(old_state.clone()), ())))
            .unwrap();
        let other = State::open(&dir).unwrap();
        let held = |files: &[(String, Vec<u8>)], bytes: &[u8]| {
            files
                .iter()
                .filter(|(_, held)| held.windows(bytes.len()).any(|window| window == bytes))
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(held(&files(&dir), &old[0]), [FILE], "the saved state");

        state
            .mls_step(|_| Ok((Some(sealed(&[(b"group:2", &new)])), ())))
            .unwrap();

        let after = files(&dir);
        for bytes in &old {
            assert_eq!(held(&after, bytes), Vec::<String>::new(), "the older state");
        }
        assert_eq!(held(&after, &new), [FILE], "the new state");
        let wal = after.iter().find(|(name, _)| name.ends_with("-wal"));
        assert_eq!(wal.map(|(_, bytes)| bytes.len()), Some(0), "the WAL");
        drop(other);
    }

    /// A save while another run reads the state, so the WAL cannot be truncated, says so.
    #[test]
    fn mls_save_that_cannot_truncate_the_wal_fails() {
        let dir = TestDir::new("state-mls-busy");
        let state = State::open(&dir).unwrap();
        state.db.busy_timeout(std::time::Duration::ZERO).unwrap();
        let reader = State::open(&dir).unwrap();
        reader.db.execute_batch("BEGIN").unwrap();
        reader.session().unwrap();

        let saved = state.mls_step(|_| Ok((Some(sealed(&[(b"group:1", &[1; 40])])), ())));

        assert_eq!(saved, Err(Error::WalBusy.to_string()));
        reader.db.execute_batch("COMMIT").unwrap();
    }

    #[test]
    fn revocation_the_device_signed_waits_until_the_server_takes_it() {
        let dir = TestDir::new("state-revocations");
        let state = State::open(&dir).unwrap();
        let (signed, given, other) = (vec![1, 2], vec![3, 4], vec![5, 6]);

        state
            .add_revocations(&[signed.clone(), other.clone()], true)
            .unwrap();
        state
            .add_revocations(std::slice::from_ref(&given), false)
            .unwrap();
        state
            .add_revocations(std::slice::from_ref(&given), true)
            .unwrap();
        assert_eq!(
            state.revocations().unwrap(),
            vec![
                (signed.clone(), true),
                (other.clone(), true),
                (given.clone(), false)
            ]
        );

        state.revocation_taken(&signed).unwrap();
        state
            .mls_change(|_| {
                let changes = MlsChanges {
                    revocations: vec![vec![7]],
                    ..MlsChanges::default()
                };
                Ok((None, changes, ()))
            })
            .unwrap();

        assert_eq!(
            state.revocations().unwrap(),
            vec![
                (signed, false),
                (other, true),
                (given, false),
                (vec![7], false)
            ]
        );
    }

    #[test]
    fn device_status_changes_are_given_once_and_first_statuses_are_only_recorded() {
        let dir = TestDir::new("state-device-statuses");
        let state = State::open(&dir).unwrap();
        let (ada, bob) = (
            DeviceId::from_signing_key(&[1; 32]),
            DeviceId::from_signing_key(&[2; 32]),
        );

        assert_eq!(
            state.device_statuses(&[(ada, false), (bob, true)]).unwrap(),
            vec![]
        );
        assert_eq!(
            state.device_statuses(&[(ada, true), (bob, false)]).unwrap(),
            vec![(ada, true), (bob, false)]
        );
        assert_eq!(
            state.device_statuses(&[(ada, true), (bob, false)]).unwrap(),
            vec![]
        );
        assert_eq!(
            state.device_statuses(&[(ada, false)]).unwrap(),
            vec![(ada, false)]
        );
    }

    #[test]
    fn epoch_keeps_its_first_judgment_and_links_stay_settled_per_channel() {
        let dir = TestDir::new("state-key-chain");
        let state = State::open(&dir).unwrap();
        let (one, other) = (ChannelId::generate(), ChannelId::generate());
        let judge = |judged: Vec<(ChannelId, u64, bool)>| {
            state
                .mls_change(|_| {
                    let changes = MlsChanges {
                        judged,
                        ..MlsChanges::default()
                    };
                    Ok((None, changes, ()))
                })
                .unwrap();
        };

        judge(vec![(one, 1, true), (one, 2, false), (other, 1, false)]);
        judge(vec![(one, 1, false), (one, 2, true), (one, 3, true)]);
        state.settle_links(one, &[(3, 1), (2, 1)]).unwrap();
        state.settle_links(one, &[(3, 1)]).unwrap();

        assert_eq!(
            state.judgments(one).unwrap(),
            BTreeMap::from([(1, true), (2, false), (3, true)])
        );
        assert_eq!(
            state.judgments(other).unwrap(),
            BTreeMap::from([(1, false)])
        );
        assert_eq!(
            state.settled_links(one).unwrap(),
            BTreeSet::from([(2, 1), (3, 1)])
        );
        assert_eq!(state.settled_links(other).unwrap(), BTreeSet::new());
    }

    #[test]
    fn given_dir_is_used() {
        let given = PathBuf::from("/some/where");
        assert_eq!(dir(Some(given.clone())).unwrap(), given);
    }
}
