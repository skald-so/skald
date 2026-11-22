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
use skald_proto::{
    AccountId, ChannelId, DeviceId, MessageId, QueueId, ReceiptStatus, Scheme, SpaceId, TaskId,
    WrapRecipient,
};

use crate::gaps::{Counters, Source};

/// The database's file in the state directory.
pub const FILE: &str = "state.sqlite3";

/// The schema's migrations, in order: the database's `user_version` counts those applied. Each is
/// a file of `migrations/`, in the order of their names; `build.rs` embeds them.
const MIGRATIONS: &[&str] = include!(concat!(env!("OUT_DIR"), "/migrations.rs"));

/// What a member service agent's device counts the `hops` of what its runs show under, as an
/// agent's name: its runs send and receive as no agent, and no person shares its device.
pub const SERVICE_AGENT: &str = "";

/// The first `seq` the device gives its own messages it keeps, above any `seq` of its mailbox, so
/// the kept messages of both have one key.
pub const OWN_FIRST: u64 = 1 << 62;

/// The state directory: `dir` if given, otherwise the platform's directory for the user's
/// application state; an empty path, which it never opens, for a stateless service agent's run,
/// which may have no such directory, as on Windows without `LOCALAPPDATA`.
pub fn dir(given: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(dir) = given {
        return Ok(dir);
    }
    if crate::service::stateless() {
        return Ok(PathBuf::new());
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
    /// Not yet: it has not learned whether the account has a master key.
    New,
    /// As the account's first device: it created the master key, pinned its public key `master`,
    /// issued its certificate `cert` and signed the account record `record` with it, and sealed it
    /// to the recovery key in the master backup `backup`, each in CBOR; the server has not taken
    /// them yet.
    Sent {
        master: [u8; 32],
        cert: Vec<u8>,
        record: Vec<u8>,
        backup: Vec<u8>,
    },
    /// The server took the master key and the certificate: the device holds its certificate.
    Certified { master: [u8; 32], cert: Vec<u8> },
    /// It waits for linking by another device of the account.
    Waiting,
    /// Waiting, it took a grant from a run of its link code: the master public key `master` it
    /// pins, its complete certificate `cert` and the link object `link`, in CBOR; the server has
    /// not stored the certificate yet.
    Granted {
        master: [u8; 32],
        cert: Vec<u8>,
        link: Vec<u8>,
    },
    /// Waiting without a session, it took a grant as [`Link::Granted`] holds one, whose issuer and
    /// link object it checks only once it has a session: until they pass, it neither stores nor
    /// pins it.
    Unchecked {
        master: [u8; 32],
        cert: Vec<u8>,
        link: Vec<u8>,
    },
    /// Waiting, it links with the recovery key: it pinned the master public key `master` and
    /// issued its certificate `cert` with the master key, in CBOR; it has not finished linking.
    Recovering { master: [u8; 32], cert: Vec<u8> },
}

/// How a device of a list changed since the device last checked it ([`State::device_statuses`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusChange {
    Revoked,
    /// Valid again.
    Valid,
    /// Valid, and new to a list the device checked before.
    Added,
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

/// An agent of the device in a space, with what its presence holds of it.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRecord {
    pub name: String,
    /// Whether it set itself busy, and its note.
    pub busy: bool,
    pub note: Option<String>,
    /// The state the device last gave it, 1 to 4 as in `agent-state`, and since when, in Unix
    /// seconds; `None` before the first.
    pub shown: Option<(u8, u64)>,
    /// The agent CLI of its last MCP session, as a `name`.
    pub runtime: Option<String>,
}

/// What the state holds of the device's presence in a channel.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelPresence {
    /// The last `revision` the device gave a value there.
    pub revision: u64,
    /// The last value the server took, if any.
    pub uploaded: Option<Uploaded>,
}

/// A value of the device's that the server took: its epoch, `expires` and agents, as JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct Uploaded {
    pub epoch: u64,
    pub expires: u64,
    pub agents: String,
}

/// What the state holds of the revisions the device accepted of a space.
#[derive(Debug, Clone, PartialEq)]
pub struct Accepted {
    /// The last accepted revision, in CBOR.
    pub last: Vec<u8>,
    /// The hash of each earlier accepted revision, by number.
    pub hashes: BTreeMap<u64, [u8; 32]>,
}

/// What the device recorded of a message it sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub space: SpaceId,
    pub channel: ChannelId,
    /// The agent it is from, none for the person or a message an older client sent.
    pub agent: Option<String>,
    /// Its content's `to`, none for everyone or a message an older client sent.
    pub addressee: Option<String>,
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
    /// Whether it goes with `report` true: false for a receipt (`docs/protocol/delivery.md`,
    /// "Receipts"), which is no message the device's user sent.
    pub report: bool,
}

/// A channel message of the device's own, as a step puts it in the outbox: its envelope, its
/// channel and its content's `counter` there; under `mls`, or under `hpke` a copy to its envelope's
/// recipient device, with the counter the device keeps for that device in the channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Counted {
    pub outgoing: Outgoing,
    pub channel: ChannelId,
    pub counter: u64,
    /// The agent it is from, none for the person or a receipt.
    pub agent: Option<String>,
    /// Its content's `to`, none for everyone.
    pub addressee: Option<String>,
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
    pub channel: ChannelId,
    pub msg_id: MessageId,
    /// The message as `forms::Message` encodes it in JSON, sealed under the state key.
    pub line: Vec<u8>,
    /// What a reply to it needs, sealed under the state key ([`crate::reply`]).
    pub target: Vec<u8>,
    /// How long it waits for an agent, and can be replied to, after the device wrote its entry, in
    /// seconds.
    pub wait: u64,
    /// Its content's `hops`.
    pub hops: u64,
    pub agents: Vec<String>,
    /// The agents in `hold` it waits for the person's decision for, until its wait ends.
    pub held: Vec<String>,
    /// The `replied` it gives a message the device sent, if it is such a reply.
    pub replied: Option<Replied>,
    /// What its receipt reports, if its `to` is an agent of this device: its `read` once it is
    /// handed over, if its `to` is one of `agents`, or its `refused` at once; its `held` at once,
    /// then `read` or `refused`, if its `to` is one of `held`.
    pub read: Option<ToRead>,
}

/// A `replied` of the device's message `msg_id` in `space`, as JSON (`forms::Status`), for the
/// person and for `agent`, the agent that sent the message, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Replied {
    pub msg_id: MessageId,
    pub space: SpaceId,
    pub status: String,
    pub agent: Option<String>,
}

/// A message the device can reply to: its `msg_id`, space and channel, and what a reply needs,
/// sealed under the state key, by its entry's `seq`.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub seq: u64,
    pub msg_id: MessageId,
    pub space: SpaceId,
    pub channel: ChannelId,
    pub sealed: Vec<u8>,
    /// Its line, sealed as the kept message's; none for one kept before the device kept lines.
    pub line: Option<Vec<u8>>,
}

/// What the receipt of a kept message reports (`docs/protocol/delivery.md`, "Receipts").
#[derive(Debug, Clone, PartialEq)]
pub struct ToRead {
    pub channel: ChannelId,
    /// The sending device.
    pub sender: DeviceId,
    /// The message's `from`, which the receipt goes `to`.
    pub author: String,
    /// The message's `to`, which the receipt is `from`, and the agent's name.
    pub addressee: String,
    pub agent: String,
    /// Why the agent's inbound mode refused it: the receipt is then a `refused`, due at once.
    pub reason: Option<String>,
    /// Whether the agent holds it for the person's decision: its `held` is then due at once.
    pub held: bool,
}

/// A receipt the device owes: of the message `msg_id`, by its kept entry's `seq`, in `space`, as
/// [`ToRead`] says, with `status`, and the reason of a `refused`, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadDue {
    pub seq: u64,
    pub msg_id: MessageId,
    pub space: SpaceId,
    pub channel: ChannelId,
    pub sender: DeviceId,
    pub author: String,
    pub addressee: String,
    pub status: ReceiptStatus,
    pub reason: Option<String>,
}

/// A Claude Code session a `SessionStart` hook registered ([`crate::claude_socket`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeSession {
    /// Its place in the order of registration.
    pub position: u64,
    /// Claude Code's session ID.
    pub session: String,
    pub space: SpaceId,
    pub agent: String,
    /// The agent's folder, where the session runs.
    pub folder: PathBuf,
    /// Its cross-session socket, and the token that makes a line its own child's.
    pub socket: PathBuf,
    pub token: Option<String>,
    /// Its session file.
    pub file: PathBuf,
}

/// A Codex session a hook registered ([`crate::codex`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSession {
    /// Its thread, Codex's session ID.
    pub thread: String,
    pub space: SpaceId,
    pub agent: String,
    /// The agent's folder, where the session runs.
    pub folder: PathBuf,
    /// Its session file and config home, if its hooks gave them.
    pub file: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// The ID of the Codex process that runs it, if known.
    pub process: Option<u32>,
    /// Whether a turn of it runs: from a hook of the turn until the `Stop` that ends it.
    pub busy: bool,
    /// The place of its last hook's run in the order of the sessions' hooks.
    pub hooked: u64,
}

/// A kept message that waits for the person's decision for the agent `agent`, by its entry's
/// `seq`, with its space, when the device took it in, in Unix seconds, and its sealed line.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub seq: u64,
    pub agent: String,
    pub space: SpaceId,
    pub taken: u64,
    pub line: Vec<u8>,
}

/// A delivery setting the person set for a channel and sender trust level, as the device keeps it
/// ([`crate::delivery`]).
#[derive(Debug, Clone, PartialEq)]
pub struct DeliverySetting {
    pub channel: String,
    /// `own`, `peer` or `service`.
    pub level: String,
    /// `now`, `batch` or `pull`, and the window of `batch`, in seconds.
    pub mode: String,
    pub window: Option<u64>,
    /// `signal` or `inline`.
    pub text: String,
}

/// An agent's inbound mode in a space, as the device keeps it, and the senders on its allowlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Inbound {
    pub agent: String,
    /// `accept`, `hold` or `refuse`.
    pub mode: String,
    pub allowed: Vec<String>,
}

/// A channel message of the device's own, as it waits for the server's `stored` to be kept for the
/// person and the agents of this device it is for (`docs/protocol/delivery.md`, "Device to
/// agent"), by the `seq` it takes from [`State::next_own_seq`].
#[derive(Debug, Clone, PartialEq)]
pub struct OwnMessage {
    pub seq: u64,
    /// Its `msg_id`, space and channel, its line and what a reply needs, sealed, and its wait, as
    /// [`Kept`] has them.
    pub kept: Kept,
    /// Whether the person's run shows it: an agent sent it.
    pub person: bool,
    /// The agent that sent it, none for the person.
    pub sender: Option<String>,
    /// Its `to`, if it has one: the agent of this device it names, none for any other.
    pub to: Option<Option<String>>,
    /// Its `read` status as JSON, which the device keeps once it hands it to the `to` agent.
    pub read: Option<String>,
    /// The agents of this device whose inbound mode refuses it, which it is not kept for.
    pub refusing: Vec<String>,
    /// Its `refused` status as JSON, which the device keeps with it, if its `to` agent refuses it.
    pub refused: Option<String>,
    /// The agents of this device that hold it for the person's decision.
    pub holding: Vec<String>,
    /// Its `held` status as JSON, which the device keeps with it, if its `to` agent holds it.
    pub held: Option<String>,
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

/// The history entry of a channel message of the device's own, as it waits for upload.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryOutgoing {
    pub msg_id: MessageId,
    pub space: SpaceId,
    pub channel: ChannelId,
    /// Its bytes, as they are uploaded every time.
    pub entry: Vec<u8>,
}

/// A task the device enqueued, as it waits for the server's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskOutgoing {
    pub task: TaskId,
    pub space: SpaceId,
    pub channel: ChannelId,
    pub queue: QueueId,
    /// Its `queued-task`, as it is uploaded every time.
    pub record: Vec<u8>,
}

/// A claim the device holds ([`crate::claims`]).
#[derive(Debug, Clone, PartialEq)]
pub struct TaskClaim {
    pub task: TaskId,
    pub space: SpaceId,
    pub channel: ChannelId,
    pub queue: QueueId,
    /// The agent that holds it; none for the person.
    pub agent: Option<String>,
    /// When the device sent the last request the server answered that started its lease, in Unix
    /// seconds by its clock.
    pub since: u64,
    /// The rest of it, sealed under the state key.
    pub sealed: Vec<u8>,
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
    /// becomes its channel's last, or, for a copy, the last to its recipient in its channel.
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
    /// History entries of the channel messages it puts in the outbox, which wait for upload.
    pub entries: Vec<HistoryOutgoing>,
    /// The `seq` of each `read` or `refused` receipt the device owed that a receipt it puts in the
    /// outbox reports.
    pub issued: Vec<u64>,
    /// The `seq` of each `held` receipt the device owed that a receipt it puts in the outbox
    /// reports.
    pub issued_held: Vec<u64>,
    /// The channel messages of [`MlsChanges::messages`] that the device keeps for itself once
    /// the server stores them; each one's `seq` becomes the last it gave.
    pub own: Vec<OwnMessage>,
    /// Each epoch of a channel whose key a wrap to the recovery key gave the device.
    pub recovered: Vec<(ChannelId, u64)>,
    /// Where the channel messages of [`MlsChanges::messages`] go, which the device keeps until
    /// their wait ends.
    pub tracked: Vec<Tracked>,
    /// The tasks whose claims end: their results are in [`MlsChanges::messages`].
    pub ended: Vec<TaskId>,
}

/// Where a channel message of the device's own goes: the devices it goes to, and how long the
/// device keeps their statuses after the server stores it, in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracked {
    pub msg_id: MessageId,
    pub wait: u64,
    pub recipients: Vec<Recipient>,
}

/// A device a message of the device's own goes to, as the device knows it at sending.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipient {
    pub device: DeviceId,
    /// Whether it gets a copy under `hpke`, outside the channel's group.
    pub copy: bool,
    /// Its certificate's `name`, its account and that account's handle in the space, if known.
    pub name: Option<String>,
    pub account: Option<String>,
    pub handle: Option<String>,
}

/// What the device keeps of where a message it sent is, until its wait ends.
#[derive(Debug, Clone, PartialEq)]
pub struct SentStatus {
    pub msg_id: MessageId,
    pub space: SpaceId,
    pub channel: ChannelId,
    /// The agent it is from, none for the person, and its `to`, if any.
    pub agent: Option<String>,
    pub addressee: Option<String>,
    /// When the device took the server's `stored`, and its `to` agent's `held`, `read`, `replied`
    /// and `refused`, in Unix seconds, with that `refused`'s reason, if any.
    pub stored: u64,
    pub held: Option<u64>,
    pub read: Option<u64>,
    pub replied: Option<u64>,
    pub refused: Option<u64>,
    pub reason: Option<String>,
    pub recipients: Vec<RecipientStatus>,
}

/// A device a message of the device's own went to, with its status and that status's time.
#[derive(Debug, Clone, PartialEq)]
pub struct RecipientStatus {
    pub device: DeviceId,
    pub name: Option<String>,
    pub account: Option<String>,
    pub handle: Option<String>,
    pub status: String,
    pub timestamp: u64,
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
            && self.entries.is_empty()
            && self.issued.is_empty()
            && self.issued_held.is_empty()
            && self.own.is_empty()
            && self.recovered.is_empty()
            && self.tracked.is_empty()
            && self.ended.is_empty()
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

    /// A new state in memory only, with the latest schema: no file, temporary ones included.
    pub fn in_memory() -> Result<Self, Error> {
        let mut db = Connection::open_in_memory()?;
        db.pragma_update(None, "temp_store", "memory")?;
        migrate(&mut db)?;
        Ok(State { db })
    }

    /// A transaction that takes the write lock as it begins, waiting for another writer up to the
    /// busy timeout. A deferred one that reads before it writes fails at once if another
    /// connection wrote meanwhile, as a Codex hook does without the lock.
    fn write(&self) -> Result<Transaction<'_>, rusqlite::Error> {
        Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)
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
                "SELECT device, name, account, registered, link, master, cert, record, backup \
                 FROM device",
                [],
                |row| {
                    Ok((
                        (
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, bool>(3)?,
                        ),
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<Vec<u8>>>(5)?,
                        row.get::<_, Option<Vec<u8>>>(6)?,
                        row.get::<_, Option<Vec<u8>>>(7)?
                            .zip(row.get::<_, Option<Vec<u8>>>(8)?),
                    ))
                },
            )
            .optional()?;
        let Some(((id, name, account, registered), link, master, cert, sealed)) = row else {
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
            "sent" => {
                let (master, cert) = pinned()?;
                let (record, backup) = sealed.ok_or_else(|| {
                    Error::Malformed("the account record and the master backup".to_owned())
                })?;
                Link::Sent {
                    master,
                    cert,
                    record,
                    backup,
                }
            }
            "certified" => pinned().map(|(master, cert)| Link::Certified { master, cert })?,
            "waiting" => match self.granted()? {
                Some(granted) => granted,
                None => self.recovering()?.unwrap_or(Link::Waiting),
            },
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

    /// Records the machine's own device, new and not registered yet, of no account so far: it
    /// learns its account from the session it gets.
    pub fn add_device_of_no_account(&self, id: DeviceId, name: &str) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO device (id, device, name) VALUES (1, ?1, ?2)",
            params![id.to_string(), name],
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

    /// The grant a waiting device took, if it holds one.
    fn granted(&self) -> Result<Option<Link>, Error> {
        let row = self
            .db
            .query_row(
                "SELECT master, cert, link, checked FROM link_grant",
                [],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(master, cert, link, checked)| {
            let master = master
                .try_into()
                .map_err(|_| Error::Malformed("the granted master key".to_owned()))?;
            Ok(if checked {
                Link::Granted { master, cert, link }
            } else {
                Link::Unchecked { master, cert, link }
            })
        })
        .transpose()
    }

    /// The master key and the certificate of a waiting device that links with the recovery key,
    /// if it is one.
    fn recovering(&self) -> Result<Option<Link>, Error> {
        let row = self
            .db
            .query_row("SELECT master, cert FROM recovery_link", [], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .optional()?;
        row.map(|(master, cert)| {
            let master = master
                .try_into()
                .map_err(|_| Error::Malformed("the master key pinned to recover".to_owned()))?;
            Ok(Link::Recovering { master, cert })
        })
        .transpose()
    }

    /// Records how the device is linked.
    pub fn set_link(&self, link: &Link) -> Result<(), Error> {
        let (name, pinned, sealed) = match link {
            Link::New => ("new", None, None),
            Link::Sent {
                master,
                cert,
                record,
                backup,
            } => ("sent", Some((master, cert)), Some((record, backup))),
            Link::Certified { master, cert } => ("certified", Some((master, cert)), None),
            Link::Waiting
            | Link::Granted { .. }
            | Link::Unchecked { .. }
            | Link::Recovering { .. } => ("waiting", None, None),
        };
        let (master, cert) = pinned.unzip();
        let (record, backup) = sealed.unzip();
        let tx = self.write()?;
        let grant = match link {
            Link::Granted { master, cert, link } => Some((master, cert, link, true)),
            Link::Unchecked { master, cert, link } => Some((master, cert, link, false)),
            _ => None,
        };
        match grant {
            Some((master, cert, link, checked)) => tx.execute(
                "INSERT OR REPLACE INTO link_grant (id, master, cert, link, checked) \
                 VALUES (1, ?1, ?2, ?3, ?4)",
                params![master.as_slice(), cert, link, checked],
            )?,
            None => tx.execute("DELETE FROM link_grant", [])?,
        };
        match link {
            Link::Recovering { master, cert } => tx.execute(
                "INSERT OR REPLACE INTO recovery_link (id, master, cert) VALUES (1, ?1, ?2)",
                params![master.as_slice(), cert],
            )?,
            _ => tx.execute("DELETE FROM recovery_link", [])?,
        };
        tx.execute(
            "UPDATE device SET link = ?1, master = ?2, cert = ?3, record = ?4, backup = ?5",
            params![
                name,
                master.map(|master| master.as_slice()),
                cert,
                record,
                backup
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// How many runs of `skald link` in a row failed on this device.
    pub fn failed_link_runs(&self) -> Result<u32, Error> {
        let failed: Option<i64> = self
            .db
            .query_row("SELECT failed_link_runs FROM device", [], |row| row.get(0))
            .optional()?;
        let failed = failed.unwrap_or(0);
        u32::try_from(failed).map_err(|_| Error::Malformed(format!("the failed runs {failed}")))
    }

    /// Records how many runs of `skald link` in a row failed on this device.
    pub fn set_failed_link_runs(&self, failed: u32) -> Result<(), Error> {
        self.db.execute(
            "UPDATE device SET failed_link_runs = ?1",
            params![i64::from(failed)],
        )?;
        Ok(())
    }

    /// The server a recovery key was made on, if it waits in the secrets to be shown to the
    /// person.
    pub fn recovery_waits(&self) -> Result<Option<String>, Error> {
        Ok(self
            .db
            .query_row("SELECT recovery FROM device", [], |row| row.get(0))
            .optional()?
            .flatten())
    }

    /// Records that a recovery key made on `server` waits in the secrets to be shown to the
    /// person, or with `None` that none does.
    pub fn set_recovery_waits(&self, server: Option<&str>) -> Result<(), Error> {
        self.db
            .execute("UPDATE device SET recovery = ?1", params![server])?;
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
            for entry in &changes.entries {
                tx.execute(
                    "INSERT INTO history_outbox (msg_id, space, channel, entry)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        entry.msg_id.to_string(),
                        entry.space.to_string(),
                        entry.channel.to_string(),
                        entry.entry
                    ],
                )?;
            }
            for &seq in &changes.issued {
                tx.execute(
                    "DELETE FROM read_receipt WHERE seq = ?1",
                    params![sql_seq(seq)?],
                )?;
            }
            for &seq in &changes.issued_held {
                tx.execute(
                    "UPDATE read_receipt SET held_due = 0 WHERE seq = ?1",
                    params![sql_seq(seq)?],
                )?;
            }
            for own in &changes.own {
                add_own(&tx, own)?;
            }
            for (channel, epoch) in &changes.recovered {
                tx.execute(
                    "INSERT INTO recovered_epoch (channel, epoch) VALUES (?1, ?2)
                     ON CONFLICT DO NOTHING",
                    params![channel.to_string(), sql_u64(*epoch)?],
                )?;
            }
            for tracked in &changes.tracked {
                add_tracked(&tx, tracked)?;
            }
            for task in &changes.ended {
                tx.execute(
                    "DELETE FROM task_claim WHERE task = ?1",
                    params![task.to_string()],
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
        let sql = "SELECT epoch, clean FROM epoch_judgment WHERE channel = ?1";
        let rows = self.of_channel(sql, channel, |row| Ok((row.get::<_, i64>(0)?, row.get(1)?)))?;
        (rows.into_iter())
            .map(|(epoch, clean)| Ok((read_epoch(channel, epoch)?, clean)))
            .collect()
    }

    /// The epochs of `channel` whose key a wrap to the recovery key gave the device
    /// ([`MlsChanges::recovered`]).
    pub fn recovered_epochs(&self, channel: ChannelId) -> Result<BTreeSet<u64>, Error> {
        let sql = "SELECT epoch FROM recovered_epoch WHERE channel = ?1";
        let rows = self.of_channel(sql, channel, |row| row.get::<_, i64>(0))?;
        (rows.into_iter())
            .map(|epoch| read_epoch(channel, epoch))
            .collect()
    }

    /// The account record the device took of `account` ([`crate::records`]), in CBOR.
    pub fn account_record(&self, account: AccountId) -> Result<Option<Vec<u8>>, Error> {
        Ok(self
            .db
            .query_row(
                "SELECT record FROM account_record WHERE account = ?1",
                params![account.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Keeps `record`, in CBOR, as the account record the device took of `account`.
    pub fn set_account_record(&self, account: AccountId, record: &[u8]) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO account_record (account, record) VALUES (?1, ?2)
             ON CONFLICT (account) DO UPDATE SET record = excluded.record",
            params![account.to_string(), record],
        )?;
        Ok(())
    }

    /// Whether the person verified `account` by fingerprint for the master key `master`.
    pub fn verified(&self, account: AccountId, master: &[u8; 32]) -> Result<bool, Error> {
        Ok(self
            .db
            .query_row(
                "SELECT 1 FROM verified_account WHERE account = ?1 AND master = ?2",
                params![account.to_string(), master.as_slice()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Records whether the person verified `account` by fingerprint for the master key `master`.
    pub fn set_verified(
        &self,
        account: AccountId,
        master: &[u8; 32],
        verified: bool,
    ) -> Result<(), Error> {
        let sql = if verified {
            "INSERT INTO verified_account (account, master) VALUES (?1, ?2) ON CONFLICT DO NOTHING"
        } else {
            "DELETE FROM verified_account WHERE account = ?1 AND master = ?2"
        };
        self.db
            .execute(sql, params![account.to_string(), master.as_slice()])?;
        Ok(())
    }

    /// The links of `channel`, by epoch and `prev`, the device does not upload again.
    pub fn settled_links(&self, channel: ChannelId) -> Result<BTreeSet<(u64, u64)>, Error> {
        let sql = "SELECT epoch, prev FROM settled_link WHERE channel = ?1";
        let rows = self.of_channel(sql, channel, |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        (rows.into_iter())
            .map(|(epoch, prev)| Ok((read_epoch(channel, epoch)?, read_epoch(channel, prev)?)))
            .collect()
    }

    /// The rows `sql` selects with `channel` as its one parameter, each as `row` reads it.
    fn of_channel<T>(
        &self,
        sql: &str,
        channel: ChannelId,
        row: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, Error> {
        let mut query = self.db.prepare(sql)?;
        let rows = query.query_map(params![channel.to_string()], row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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

    /// The wraps of `channel`, by epoch and recipient, the device does not upload again.
    pub fn settled_wraps(
        &self,
        channel: ChannelId,
    ) -> Result<BTreeSet<(u64, WrapRecipient)>, Error> {
        let sql = "SELECT epoch, kind, recipient FROM settled_wrap WHERE channel = ?1";
        let rows = self.of_channel(sql, channel, |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get(2)?))
        })?;
        (rows.into_iter())
            .map(|(epoch, kind, recipient): (i64, i64, String)| {
                let recipient = match kind {
                    1 => WrapRecipient::Browser(id("browser's device ID", &recipient)?),
                    2 => WrapRecipient::Recovery(id("account ID", &recipient)?),
                    _ => return Err(Error::Malformed(format!("the wrap recipient kind {kind}"))),
                };
                Ok((read_epoch(channel, epoch)?, recipient))
            })
            .collect()
    }

    /// Keeps `wraps` of `channel`, by epoch and recipient, from being uploaded again.
    pub fn settle_wraps(
        &self,
        channel: ChannelId,
        wraps: &[(u64, WrapRecipient)],
    ) -> Result<(), Error> {
        for &(epoch, recipient) in wraps {
            let (kind, recipient) = match recipient {
                WrapRecipient::Browser(device) => (1, device.to_string()),
                WrapRecipient::Recovery(account) => (2, account.to_string()),
            };
            self.db.execute(
                "INSERT INTO settled_wrap (channel, epoch, kind, recipient)
                 VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
                params![channel.to_string(), sql_u64(epoch)?, kind, recipient],
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

    /// Records the space `id` named `name` at `revision`, in CBOR, the revision a link gave the
    /// device or a `service` device's key secret pins, as its anchor and last accepted revision,
    /// with none pending; `personal`, pinned as the device's personal space. A space the device
    /// holds stays as it is.
    pub fn add_linked_space(
        &self,
        id: SpaceId,
        name: &str,
        revision: &[u8],
        personal: bool,
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO space (space, name, anchor, accepted, personal) VALUES (?1, ?2, ?3, ?3, ?4)
             ON CONFLICT (space) DO NOTHING",
            params![id.to_string(), name, revision, personal],
        )?;
        Ok(())
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
        let tx = self.write()?;
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
        let tx = self.write()?;
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
        let tx = self.write()?;
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
        let tx = self.write()?;
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
        self.last_counter(
            "SELECT last FROM channel_counter WHERE channel = ?1",
            params![channel.to_string()],
        )
    }

    /// The last `counter` the device gave content to the device `recipient` in `channel` under
    /// `hpke`, in a copy of a channel message; 0 before the first.
    pub fn copy_counter(&self, channel: ChannelId, recipient: DeviceId) -> Result<u64, Error> {
        self.last_counter(
            "SELECT last FROM copy_counter WHERE channel = ?1 AND recipient = ?2",
            params![channel.to_string(), recipient.to_string()],
        )
    }

    /// The counter `query` reads with `params`; 0 if it reads none.
    fn last_counter(&self, query: &str, params: impl rusqlite::Params) -> Result<u64, Error> {
        let last: Option<i64> = self
            .db
            .query_row(query, params, |row| row.get(0))
            .optional()?;
        let last = last.unwrap_or(0);
        u64::try_from(last).map_err(|_| Error::Malformed(format!("the counter {last}")))
    }

    /// The space and channel of the message `msg_id` the device sent, the agent it is from, none
    /// for the person, and its `to`, if the state recorded it.
    pub fn sent(&self, msg_id: MessageId) -> Result<Option<Sent>, Error> {
        let row: Option<(String, String, Option<String>, Option<String>)> = self
            .db
            .query_row(
                "SELECT space, channel, agent, addressee FROM sent WHERE msg_id = ?1",
                params![msg_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        row.map(|(space, channel, agent, addressee)| {
            let malformed = || Error::Malformed(format!("the sent message {msg_id}"));
            Ok(Sent {
                space: space.parse().map_err(|_| malformed())?,
                channel: channel.parse().map_err(|_| malformed())?,
                agent,
                addressee,
            })
        })
        .transpose()
    }

    /// The last anchors object the device signed, as its CBOR bytes, with whether it waits for the
    /// server to take it.
    pub fn anchors(&self) -> Result<Option<(Vec<u8>, bool)>, Error> {
        Ok(self
            .db
            .query_row("SELECT anchors, pending FROM anchors", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?)
    }

    /// Records `next`, in CBOR, as the device's last anchors object, waiting for the server, in
    /// place of `replaced` or of none. Records nothing and gives `false` if another run changed it
    /// meanwhile.
    pub fn set_anchors(&self, replaced: Option<&[u8]>, next: &[u8]) -> Result<bool, Error> {
        let changed = match replaced {
            Some(replaced) => self.db.execute(
                "UPDATE anchors SET anchors = ?2, pending = 1 WHERE anchors = ?1",
                params![replaced, next],
            )?,
            None => self.db.execute(
                "INSERT INTO anchors (id, anchors, pending) VALUES (1, ?1, 1)
                 ON CONFLICT DO NOTHING",
                params![next],
            )?,
        };
        Ok(changed > 0)
    }

    /// Records that the server took `anchors`, the device's last anchors object, if it still is.
    pub fn anchors_taken(&self, anchors: &[u8]) -> Result<(), Error> {
        self.db.execute(
            "UPDATE anchors SET pending = 0 WHERE anchors = ?1",
            params![anchors],
        )?;
        Ok(())
    }

    /// The `msg_id` of each message the device keeps: those it sent, those that wait to be shown,
    /// but for any an older client kept, and those that can be replied to now.
    pub fn kept_ids(&self) -> Result<Vec<MessageId>, Error> {
        let now = sql_u64(unix_now())?;
        let mut query = self.db.prepare(
            "SELECT msg_id FROM sent UNION SELECT msg_id FROM kept WHERE msg_id IS NOT NULL
             UNION SELECT msg_id FROM reply_target WHERE expires > ?1",
        )?;
        let ids = query.query_map(params![now], |row| row.get::<_, String>(0))?;
        ids.map(|msg_id| id("message ID", &msg_id?)).collect()
    }

    /// Keeps `status`, a status of the device's message `msg_id` in `space`, if known, as JSON,
    /// from the entry `seq`, for the person and for `agent`, the agent that sent the message, if
    /// any, and takes the entry out of the inbox, at once. A `delivered` or `expired` becomes its
    /// device's status of the message while the device keeps where the message is, its time no
    /// earlier than when the device took the server's `stored`: the server's clock is not the
    /// device's.
    pub fn keep_status(
        &self,
        seq: u64,
        (msg_id, space, agent): (MessageId, Option<SpaceId>, Option<&str>),
        status: &str,
    ) -> Result<(), Error> {
        let seq = sql_seq(seq)?;
        let tx = self.write()?;
        tx.execute(
            "INSERT INTO kept_status (seq, msg_id, space, status, person, agent)
             VALUES (?1, ?2, ?3, ?4, 1, ?5) ON CONFLICT DO NOTHING",
            params![
                seq,
                msg_id.to_string(),
                space.map(|space| space.to_string()),
                status,
                agent
            ],
        )?;
        let fields = Fields::of(status);
        if let (Some(device), Some(word @ ("delivered" | "expired"))) =
            (fields.text("device"), fields.text("status"))
        {
            tx.execute(
                "INSERT INTO sent_recipient (msg_id, device, copy, name, status, timestamp)
                 SELECT msg_id, ?2, 0, ?3, ?4, max(coalesce(?5, 0), coalesce(stored, 0))
                 FROM sent_status WHERE msg_id = ?1
                 ON CONFLICT DO UPDATE SET status = excluded.status, timestamp = excluded.timestamp",
                params![
                    msg_id.to_string(),
                    device,
                    fields.text("device_name"),
                    word,
                    fields.time()
                ],
            )?;
        }
        tx.execute("DELETE FROM inbox WHERE seq = ?1", params![seq])?;
        tx.commit()?;
        Ok(())
    }

    /// The kept statuses the person's run has yet to show, in order, by position, each as JSON.
    pub fn statuses_for_person(&self) -> Result<Vec<(u64, String)>, Error> {
        self.lines(
            "SELECT position, status FROM kept_status WHERE person = 1 ORDER BY position",
            params![],
        )
    }

    /// Records that the person's run showed the kept status at `position`.
    pub fn status_shown_to_person(&self, position: u64) -> Result<(), Error> {
        self.db.execute(
            "UPDATE kept_status SET person = 0 WHERE position = ?1",
            params![sql_seq(position)?],
        )?;
        self.drop_done_statuses()
    }

    /// The kept statuses of the messages the agent `agent` sent in the space `space` that its run
    /// has yet to take, in order, by position, each as JSON.
    pub fn statuses_for_agent(
        &self,
        space: SpaceId,
        agent: &str,
    ) -> Result<Vec<(u64, String)>, Error> {
        self.lines(
            "SELECT position, status FROM kept_status WHERE space = ?1 AND agent = ?2
             ORDER BY position",
            params![space.to_string(), agent],
        )
    }

    /// Records that the agent's run took the kept status at `position`.
    pub fn status_taken_by_agent(&self, position: u64) -> Result<(), Error> {
        self.db.execute(
            "UPDATE kept_status SET agent = NULL WHERE position = ?1",
            params![sql_seq(position)?],
        )?;
        self.drop_done_statuses()
    }

    /// Records that the agent `agent` took the server's refusals of its message `msg_id`, kept for
    /// it alone: its own run reported them.
    pub fn refusals_taken(&self, agent: &str, msg_id: MessageId) -> Result<(), Error> {
        self.db.execute(
            "UPDATE kept_status SET agent = NULL
             WHERE seq IS NULL AND person = 0 AND agent = ?1 AND msg_id = ?2",
            params![agent, msg_id.to_string()],
        )?;
        self.drop_done_statuses()
    }

    /// Drops each kept status the person's run showed that no agent has yet to take.
    fn drop_done_statuses(&self) -> Result<(), Error> {
        self.db.execute(
            "DELETE FROM kept_status WHERE person = 0 AND agent IS NULL",
            [],
        )?;
        Ok(())
    }

    /// Keeps `lines`, warnings an agent's run gave, each a full line, for the person's run, after
    /// those kept already; a line kept already is kept once.
    pub fn keep_warnings(&self, lines: &[String]) -> Result<(), Error> {
        let tx = self.write()?;
        for line in lines {
            tx.execute(
                "INSERT INTO kept_warning (line) SELECT ?1 \
                 WHERE NOT EXISTS (SELECT 1 FROM kept_warning WHERE line = ?1)",
                params![line],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The kept warnings, in order, each by its position, a full line.
    pub fn warnings(&self) -> Result<Vec<(u64, String)>, Error> {
        self.lines(
            "SELECT position, line FROM kept_warning ORDER BY position",
            params![],
        )
    }

    /// Records that the person's run showed the kept warning at `position`.
    pub fn warning_shown(&self, position: u64) -> Result<(), Error> {
        self.db.execute(
            "DELETE FROM kept_warning WHERE position = ?1",
            params![sql_seq(position)?],
        )?;
        Ok(())
    }

    /// The highest `revision` the device accepted of the presence value of `device` in `channel`,
    /// with the SHA-256 of that value's bytes; none before the first.
    pub fn presence_revision(
        &self,
        channel: ChannelId,
        device: DeviceId,
    ) -> Result<Option<(u64, [u8; 32])>, Error> {
        let row: Option<(i64, Vec<u8>)> = self
            .db
            .query_row(
                "SELECT revision, hash FROM presence_revision WHERE channel = ?1 AND device = ?2",
                params![channel.to_string(), device.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let malformed = || Error::Malformed(format!("the presence revision of {device}"));
        row.map(|(revision, hash)| {
            Ok((
                u64::try_from(revision).map_err(|_| malformed())?,
                hash.try_into().map_err(|_| malformed())?,
            ))
        })
        .transpose()
    }

    /// Records `revision`, of the value of `device` in `channel` whose bytes have the SHA-256
    /// `hash`, as the highest the device accepted of it, in place of a lower one.
    pub fn accept_presence(
        &self,
        (channel, device): (ChannelId, DeviceId),
        revision: u64,
        hash: &[u8; 32],
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO presence_revision (channel, device, revision, hash) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (channel, device) DO UPDATE SET revision = ?3, hash = ?4
             WHERE revision < ?3",
            params![
                channel.to_string(),
                device.to_string(),
                sql_u64(revision)?,
                hash.as_slice()
            ],
        )?;
        Ok(())
    }

    /// Keeps `line` for the next tool result of the agent `agent` in the space `space`, once.
    pub fn keep_notice(&self, space: SpaceId, agent: &str, line: &str) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO agent_notice (space, agent, line) SELECT ?1, ?2, ?3
             WHERE NOT EXISTS (
                 SELECT 1 FROM agent_notice WHERE space = ?1 AND agent = ?2 AND line = ?3
             )",
            params![space.to_string(), agent, line],
        )?;
        Ok(())
    }

    /// Takes the lines kept for the next tool result of the agent `agent` in the space `space`, in
    /// order: none is given again.
    pub fn take_notices(&self, space: SpaceId, agent: &str) -> Result<Vec<String>, Error> {
        let tx = self.write()?;
        let lines = {
            let mut query = tx.prepare(
                "SELECT line FROM agent_notice WHERE space = ?1 AND agent = ?2 ORDER BY position",
            )?;
            let lines = query.query_map(params![space.to_string(), agent], |row| row.get(0))?;
            lines.collect::<Result<Vec<String>, _>>()?
        };
        tx.execute(
            "DELETE FROM agent_notice WHERE space = ?1 AND agent = ?2",
            params![space.to_string(), agent],
        )?;
        tx.commit()?;
        Ok(lines)
    }

    /// The revocations the device holds, as their CBOR bytes, with whether each waits for the
    /// server to take it.
    pub fn revocations(&self) -> Result<Vec<(Vec<u8>, bool)>, Error> {
        self.pairs("SELECT revocation, pending FROM revocation ORDER BY rowid")
    }

    /// Keeps `revocations`, as their CBOR bytes, among those the device holds; those it signed
    /// wait for the server if `pending`.
    pub fn add_revocations(&self, revocations: &[Vec<u8>], pending: bool) -> Result<(), Error> {
        let tx = self.write()?;
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

    /// Records whether each of `devices`, the devices of one list, is revoked; gives those the
    /// device held otherwise before, each as revoked or valid again now, and, if it held any of
    /// them before, each valid one it never held either way, as added. Any other device it never
    /// held is recorded only.
    pub fn device_statuses(
        &self,
        devices: &[(DeviceId, bool)],
    ) -> Result<Vec<(DeviceId, StatusChange)>, Error> {
        let tx = self.write()?;
        let mut changed = Vec::new();
        let (mut known, mut added) = (false, Vec::new());
        for &(device, revoked) in devices {
            let id = device.to_string();
            let held: Option<bool> = tx
                .query_row(
                    "SELECT revoked FROM device_status WHERE device = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()?;
            known |= held.is_some();
            if held == Some(revoked) {
                continue;
            }
            tx.execute(
                "INSERT INTO device_status (device, revoked) VALUES (?1, ?2)
                 ON CONFLICT (device) DO UPDATE SET revoked = excluded.revoked",
                params![id, revoked],
            )?;
            match held {
                Some(_) if revoked => changed.push((device, StatusChange::Revoked)),
                Some(_) => changed.push((device, StatusChange::Valid)),
                None if !revoked => added.push((device, StatusChange::Added)),
                None => {}
            }
        }
        tx.commit()?;
        if known {
            changed.extend(added);
        }
        Ok(changed)
    }

    /// The certificates the device keeps of `device`, in CBOR, in the order it kept them
    /// ([`crate::kept_certs`]).
    pub fn kept_certs(&self, device: DeviceId) -> Result<Vec<Vec<u8>>, Error> {
        let mut statement = self
            .db
            .prepare("SELECT cert FROM kept_cert WHERE device = ?1 ORDER BY rowid")?;
        let certs = statement
            .query_map(params![device.to_string()], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(certs)
    }

    /// Keeps `cert`, in CBOR, as a certificate of `device`: only if it keeps none of it yet when
    /// `first`.
    pub fn keep_cert(&self, device: DeviceId, cert: &[u8], first: bool) -> Result<(), Error> {
        let id = device.to_string();
        if first {
            self.db.execute(
                "INSERT INTO kept_cert (device, cert) SELECT ?1, ?2
                 WHERE NOT EXISTS (SELECT 1 FROM kept_cert WHERE device = ?1)",
                params![id, cert],
            )?;
        } else {
            self.db.execute(
                "INSERT OR IGNORE INTO kept_cert (device, cert) VALUES (?1, ?2)",
                params![id, cert],
            )?;
        }
        Ok(())
    }

    /// Records `swapped`, the certificates in CBOR the server gives of `device` other than the one
    /// the device kept; gives whether it warned of none of them last, so it warns now. With none,
    /// forgets the one it warned of.
    pub fn swapped_certs(&self, device: DeviceId, swapped: &[Vec<u8>]) -> Result<bool, Error> {
        let id = device.to_string();
        let Some(first) = swapped.first() else {
            self.db
                .execute("DELETE FROM swapped_cert WHERE device = ?1", params![id])?;
            return Ok(false);
        };
        let tx = self.write()?;
        let held: Option<Vec<u8>> = tx
            .query_row(
                "SELECT cert FROM swapped_cert WHERE device = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        if held.is_some_and(|held| swapped.contains(&held)) {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO swapped_cert (device, cert) VALUES (?1, ?2)
             ON CONFLICT (device) DO UPDATE SET cert = excluded.cert",
            params![id, first],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Records `masters`, a set of master keys of one account as text; gives whether it is new.
    pub fn master_warning(&self, masters: &str) -> Result<bool, Error> {
        let added = self.db.execute(
            "INSERT OR IGNORE INTO master_warning (masters) VALUES (?1)",
            params![masters],
        )?;
        Ok(added == 1)
    }

    /// The result of the check of the certificate whose body has the SHA-256 `body` against
    /// GitHub's API at `api` ([`crate::ssh_checks`]), if it was checked: the login GitHub gave if
    /// it passed, `None` if it failed.
    pub fn ssh_check(&self, api: &str, body: &[u8; 32]) -> Result<Option<Option<String>>, Error> {
        Ok(self
            .db
            .query_row(
                "SELECT login FROM ssh_check WHERE api = ?1 AND body = ?2",
                params![api, body],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Keeps the result of that check, `login` if it passed, unless it holds one.
    pub fn keep_ssh_check(
        &self,
        api: &str,
        body: &[u8; 32],
        login: Option<&str>,
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT OR IGNORE INTO ssh_check (api, body, login) VALUES (?1, ?2, ?3)",
            params![api, body, login],
        )?;
        Ok(())
    }

    /// The `seq` the device's next own message takes ([`OwnMessage`]).
    pub fn next_own_seq(&self) -> Result<u64, Error> {
        let last: Option<i64> = self
            .db
            .query_row("SELECT own_seq FROM device", [], |row| row.get(0))
            .optional()?;
        let last = u64::try_from(last.unwrap_or(0))
            .map_err(|_| Error::Malformed("the last own seq".to_owned()))?;
        OWN_FIRST
            .checked_add(last)
            .and_then(|seq| seq.checked_add(1))
            .filter(|&seq| i64::try_from(seq).is_ok())
            .ok_or_else(|| Error::Malformed(format!("the own seq after {last}")))
    }

    /// The highest `seq` of the device's own messages kept for `agent`, or for the person with
    /// none; 0 with none. A later own message kept for it has a higher one.
    pub fn last_own_kept(&self, agent: Option<&str>) -> Result<u64, Error> {
        let first = sql_seq(OWN_FIRST)?;
        let last: i64 = match agent {
            Some(agent) => self.db.query_row(
                "SELECT coalesce(max(seq), 0) FROM handover WHERE agent = ?1 AND seq >= ?2",
                params![agent, first],
                |row| row.get(0),
            )?,
            None => self.db.query_row(
                "SELECT coalesce(max(seq), 0) FROM kept WHERE person = 1 AND seq >= ?1",
                params![first],
                |row| row.get(0),
            )?,
        };
        u64::try_from(last).map_err(|_| Error::Malformed("a kept seq".to_owned()))
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
        let tx = self.write()?;
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
        let tx = self.write()?;
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

    /// Each agent the device has run as in the space `space`, by name, with what its presence
    /// holds of it.
    pub fn agent_statuses(&self, space: SpaceId) -> Result<Vec<AgentRecord>, Error> {
        let mut query = self.db.prepare(
            "SELECT agent.name, status.busy, status.note, status.state, status.since,
                    status.runtime
             FROM agent LEFT JOIN agent_status AS status
                 ON status.space = agent.space AND status.name = agent.name
             WHERE agent.space = ?1 ORDER BY agent.name",
        )?;
        let rows = query.query_map(params![space.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<bool>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        rows.map(|row| {
            let (name, busy, note, state, since, runtime) = row?;
            let shown = match (state, since) {
                (Some(state), Some(since)) => Some((
                    u8::try_from(state)
                        .map_err(|_| Error::Malformed(format!("the agent state {state}")))?,
                    u64::try_from(since)
                        .map_err(|_| Error::Malformed(format!("the time {since}")))?,
                )),
                _ => None,
            };
            Ok(AgentRecord {
                name,
                busy: busy.unwrap_or(false),
                note,
                shown,
                runtime,
            })
        })
        .collect()
    }

    /// Records that the agent `name` in the space `space` set itself busy if `busy`, otherwise
    /// available, with `note`.
    pub fn set_agent_status(
        &self,
        space: SpaceId,
        name: &str,
        busy: bool,
        note: Option<&str>,
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO agent_status (space, name, busy, note) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (space, name) DO UPDATE SET busy = excluded.busy, note = excluded.note",
            params![space.to_string(), name, busy, note],
        )?;
        Ok(())
    }

    /// Records that an MCP session of the agent `name` in the space `space` started, its agent CLI
    /// `runtime`: the agent is available, with no note.
    pub fn agent_session_started(
        &self,
        space: SpaceId,
        name: &str,
        runtime: Option<&str>,
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO agent_status (space, name, runtime) VALUES (?1, ?2, ?3)
             ON CONFLICT (space, name) DO UPDATE SET busy = 0, note = NULL, runtime = excluded.runtime",
            params![space.to_string(), name, runtime],
        )?;
        Ok(())
    }

    /// Records that the device gave the agent `name` in the space `space` the state `state`, 1 to
    /// 4 as in `agent-state`, since `since`.
    pub fn agent_shown(
        &self,
        space: SpaceId,
        name: &str,
        (state, since): (u8, u64),
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO agent_status (space, name, state, since) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (space, name) DO UPDATE SET state = excluded.state, since = excluded.since",
            params![space.to_string(), name, state, sql_u64(since)?],
        )?;
        Ok(())
    }

    /// What the device holds of its presence in `channel`, if it gave a value there.
    pub fn channel_presence(&self, channel: ChannelId) -> Result<Option<ChannelPresence>, Error> {
        let row = self
            .db
            .query_row(
                "SELECT revision, epoch, expires, agents FROM presence WHERE channel = ?1",
                params![channel.to_string()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((revision, epoch, expires, agents)) = row else {
            return Ok(None);
        };
        let number =
            |value: i64| u64::try_from(value).map_err(|_| Error::Malformed(format!("{value}")));
        let uploaded = match (epoch, expires, agents) {
            (Some(epoch), Some(expires), Some(agents)) => Some(Uploaded {
                epoch: number(epoch)?,
                expires: number(expires)?,
                agents,
            }),
            _ => None,
        };
        Ok(Some(ChannelPresence {
            revision: number(revision)?,
            uploaded,
        }))
    }

    /// The `revision` of the device's next value in `channel`, one more than the last it gave,
    /// recorded before it returns, as a counter is.
    pub fn next_presence_revision(&self, channel: ChannelId) -> Result<u64, Error> {
        let revision: i64 = self.db.query_row(
            "INSERT INTO presence (channel, revision) VALUES (?1, 1)
             ON CONFLICT (channel) DO UPDATE SET revision = revision + 1
             RETURNING revision",
            params![channel.to_string()],
            |row| row.get(0),
        )?;
        u64::try_from(revision).map_err(|_| Error::Malformed(format!("the revision {revision}")))
    }

    /// Records that the server took the device's value in `channel` of `uploaded`.
    pub fn presence_uploaded(&self, channel: ChannelId, uploaded: &Uploaded) -> Result<(), Error> {
        self.db.execute(
            "UPDATE presence SET epoch = ?2, expires = ?3, agents = ?4 WHERE channel = ?1",
            params![
                channel.to_string(),
                sql_u64(uploaded.epoch)?,
                sql_u64(uploaded.expires)?,
                uploaded.agents
            ],
        )?;
        Ok(())
    }

    /// Sets the inbound mode of the agent `agent` in the space `space` to `mode`.
    pub fn set_inbound_mode(&self, space: SpaceId, agent: &str, mode: &str) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO inbound_mode (space, agent, mode) VALUES (?1, ?2, ?3)
             ON CONFLICT DO UPDATE SET mode = excluded.mode",
            params![space.to_string(), agent, mode],
        )?;
        Ok(())
    }

    /// Puts `sender` on the allowlist of the agent `agent` in the space `space`; gives whether it
    /// was not on it.
    pub fn allow_sender(&self, space: SpaceId, agent: &str, sender: &str) -> Result<bool, Error> {
        let added = self.db.execute(
            "INSERT INTO allowed_sender (space, agent, sender) VALUES (?1, ?2, ?3)
             ON CONFLICT DO NOTHING",
            params![space.to_string(), agent, sender],
        )?;
        Ok(added > 0)
    }

    /// The inbound mode and allowlist of each agent in the space `space` the device has run as or
    /// keeps either of, by name; `accept` for one without a mode.
    pub fn inbound(&self, space: SpaceId) -> Result<Vec<Inbound>, Error> {
        let space = space.to_string();
        let mut query = self.db.prepare(
            "SELECT name, COALESCE((SELECT mode FROM inbound_mode WHERE space = ?1 AND agent = name),
                 'accept')
             FROM (SELECT name FROM agent WHERE space = ?1
                 UNION SELECT agent FROM inbound_mode WHERE space = ?1
                 UNION SELECT agent FROM allowed_sender WHERE space = ?1)
             ORDER BY name",
        )?;
        let modes = query
            .query_map(params![space], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<(String, String)>, _>>()?;
        let mut query = self.db.prepare(
            "SELECT sender FROM allowed_sender WHERE space = ?1 AND agent = ?2 ORDER BY sender",
        )?;
        modes
            .into_iter()
            .map(|(agent, mode)| {
                let allowed = query
                    .query_map(params![space, agent], |row| row.get(0))?
                    .collect::<Result<_, _>>()?;
                Ok(Inbound {
                    agent,
                    mode,
                    allowed,
                })
            })
            .collect()
    }

    /// Sets the delivery mode of the sender trust level `level` in the channel `channel` to `mode`,
    /// with `window`, in seconds, for `batch`.
    pub fn set_delivery_mode(
        &self,
        channel: ChannelId,
        level: &str,
        mode: &str,
        window: Option<u64>,
    ) -> Result<(), Error> {
        let window = window.map(sql_u64).transpose()?;
        self.db.execute(
            "INSERT INTO delivery_setting (channel, level, mode, window) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT DO UPDATE SET mode = excluded.mode, window = excluded.window",
            params![channel.to_string(), level, mode, window],
        )?;
        Ok(())
    }

    /// Sets the text setting of the sender trust level `level` in the channel `channel` to `text`.
    pub fn set_delivery_text(
        &self,
        channel: ChannelId,
        level: &str,
        text: &str,
    ) -> Result<(), Error> {
        self.db.execute(
            "INSERT INTO delivery_setting (channel, level, text) VALUES (?1, ?2, ?3)
             ON CONFLICT DO UPDATE SET text = excluded.text",
            params![channel.to_string(), level, text],
        )?;
        Ok(())
    }

    /// The delivery settings the person set, by channel and level.
    pub fn delivery_settings(&self) -> Result<Vec<DeliverySetting>, Error> {
        let mut query = self
            .db
            .prepare("SELECT channel, level, mode, window, text FROM delivery_setting")?;
        let rows = query.query_map([], |row| {
            Ok((
                texts::<3>(row, 0)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.map(|row| {
            let ([channel, level, mode], window, text) = row?;
            let window = (window.map(u64::try_from).transpose())
                .map_err(|_| Error::Malformed(format!("the window of {channel}")))?;
            Ok(DeliverySetting {
                channel,
                level,
                mode,
                window,
                text,
            })
        })
        .collect()
    }

    /// Counts one wake in each of `channels`.
    pub fn count_wakes(&self, channels: &BTreeSet<String>) -> Result<(), Error> {
        let tx = self.write()?;
        for channel in channels {
            tx.execute(
                "INSERT INTO channel_wakes (channel, wakes) VALUES (?1, 1)
                 ON CONFLICT DO UPDATE SET wakes = wakes + 1",
                params![channel],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Counts one wake in each of `channels` for a write of the lines `lines` to the socket of the
    /// Claude Code session `session`, under an ID of its own; the session saying that none of them
    /// reached the model takes it back ([`State::not_taken`]).
    pub fn socket_wakes(
        &self,
        session: &str,
        lines: &[String],
        channels: &BTreeSet<String>,
    ) -> Result<(), Error> {
        let write = MessageId::generate().to_string();
        let tx = self.write()?;
        for line in lines {
            tx.execute(
                "INSERT OR REPLACE INTO socket_line (line, session, write) VALUES (?1, ?2, ?3)",
                params![line, session, write],
            )?;
        }
        for channel in channels {
            tx.execute(
                "INSERT OR IGNORE INTO socket_wake (write, channel) VALUES (?1, ?2)",
                params![write, channel],
            )?;
            tx.execute(
                "INSERT INTO channel_wakes (channel, wakes) VALUES (?1, 1)
                 ON CONFLICT DO UPDATE SET wakes = wakes + 1",
                params![channel],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The wakes the device caused in each channel that has any.
    pub fn wakes(&self) -> Result<BTreeMap<String, u64>, Error> {
        let mut query = self
            .db
            .prepare("SELECT channel, wakes FROM channel_wakes")?;
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.map(|row| {
            let (channel, wakes) = row?;
            Ok((channel, u64::try_from(wakes).unwrap_or(0)))
        })
        .collect()
    }

    /// When the device kept each kept message in the space `space`, by `seq`, in Unix
    /// milliseconds; 0 for one an older client kept.
    pub fn kept_came(&self, space: SpaceId) -> Result<BTreeMap<u64, u64>, Error> {
        let came: Vec<(u64, i64)> = self.lines(
            "SELECT seq, COALESCE(came, 0) FROM kept WHERE space = ?1",
            params![space.to_string()],
        )?;
        Ok((came.into_iter())
            .map(|(seq, came)| (seq, u64::try_from(came).unwrap_or(0)))
            .collect())
    }

    /// The kept messages the person's run has yet to show, by `seq`, each with its sealed line.
    pub fn for_person(&self) -> Result<Vec<(u64, Vec<u8>)>, Error> {
        self.lines(
            "SELECT seq, line FROM kept WHERE person = 1 ORDER BY seq",
            params![],
        )
    }

    /// Records that the person's run showed the kept message `seq`, which it can then reply to,
    /// and, if `service`, that a member service agent's run took it: its `hops` count for the
    /// agent's next send under [`SERVICE_AGENT`].
    pub fn shown_to_person(&self, seq: u64, service: bool) -> Result<(), Error> {
        let seq = sql_seq(seq)?;
        let tx = self.write()?;
        if service {
            count_hops(&tx, seq, SERVICE_AGENT)?;
        }
        add_target(&tx, seq)?;
        tx.execute(
            "UPDATE reply_target SET person = 1 WHERE seq = ?1",
            params![seq],
        )?;
        tx.execute("UPDATE kept SET person = 0 WHERE seq = ?1", params![seq])?;
        drop_done(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// The kept messages that wait for the agent `agent` in the space `space`, by `seq`, each with
    /// its sealed line, once those whose wait ended by `now`, in Unix seconds, are dropped for
    /// every agent; but those a session's socket took that wait for its confirmation.
    pub fn for_agent(
        &self,
        space: SpaceId,
        agent: &str,
        now: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, Error> {
        let now = i64::try_from(now).unwrap_or(i64::MAX);
        let tx = self.write()?;
        tx.execute(
            "DELETE FROM handover WHERE seq IN (SELECT seq FROM kept WHERE expires <= ?1)",
            params![now],
        )?;
        drop_done(&tx)?;
        tx.commit()?;
        self.lines(
            "SELECT kept.seq, kept.line FROM kept JOIN handover ON handover.seq = kept.seq
             WHERE kept.space = ?1 AND handover.agent = ?2
                 AND (handover.written IS NULL OR handover.failed = 1)
             ORDER BY kept.seq",
            params![space.to_string(), agent],
        )
    }

    /// Records that the kept message `seq` was handed over to the agent `agent`, which can then
    /// reply to it, counts its `hops` for the agent's next send ([`State::handed_hops`]), and that
    /// its `read` receipt is due if `agent` is its `to`, or, for an own message of the device's,
    /// that it is `read`, at once.
    pub fn handed_over(&self, seq: u64, agent: &str) -> Result<(), Error> {
        let seq = sql_seq(seq)?;
        let tx = self.write()?;
        if add_target(&tx, seq)? {
            tx.execute(
                "INSERT INTO reply_agent (seq, agent) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                params![seq, agent],
            )?;
        }
        tx.execute(
            "DELETE FROM handover WHERE seq = ?1 AND agent = ?2",
            params![seq, agent],
        )?;
        count_hops(&tx, seq, agent)?;
        tx.execute(
            "UPDATE read_receipt SET due = 1 WHERE seq = ?1 AND agent = ?2",
            params![seq, agent],
        )?;
        read_own(&tx, seq, agent, None)?;
        drop_done(&tx)?;
        tx.commit()?;
        Ok(())
    }

    /// The `seq` of each kept message that waits for the agent `agent` whose signal its hook gave.
    pub fn signalled(&self, agent: &str) -> Result<Vec<u64>, Error> {
        let signalled: Vec<(u64, i64)> = self.lines(
            "SELECT seq, signalled FROM handover WHERE agent = ?1 AND signalled = 1 ORDER BY seq",
            params![agent],
        )?;
        Ok(signalled.into_iter().map(|(seq, _)| seq).collect())
    }

    /// Registers the Claude Code session `session`, after every other, in place of its earlier
    /// registration, if any.
    pub fn register_claude(&self, session: &ClaudeSession) -> Result<(), Error> {
        let tx = self.write()?;
        tx.execute(
            "DELETE FROM claude_session WHERE session = ?1",
            params![session.session],
        )?;
        tx.execute(
            "INSERT INTO claude_session (session, space, agent, folder, socket, token, file)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.session,
                session.space.to_string(),
                session.agent,
                path_text(&session.folder)?,
                path_text(&session.socket)?,
                session.token,
                path_text(&session.file)?,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The registered Claude Code sessions, in the order of registration.
    pub fn claude_sessions(&self) -> Result<Vec<ClaudeSession>, Error> {
        let mut query = self.db.prepare(
            "SELECT position, session, space, agent, folder, socket, token, file
             FROM claude_session ORDER BY position",
        )?;
        let rows = query.query_map([], |row| {
            let text = |at| row.get::<_, String>(at);
            let texts = [text(1)?, text(2)?, text(3)?, text(4)?, text(5)?, text(7)?];
            Ok((
                row.get::<_, i64>(0)?,
                texts,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        rows.map(|row| {
            let (position, [session, space, agent, folder, socket, file], token) = row?;
            let malformed = || Error::Malformed(format!("the Claude Code session {session}"));
            Ok(ClaudeSession {
                position: u64::try_from(position).map_err(|_| malformed())?,
                space: space.parse().map_err(|_| malformed())?,
                session,
                agent,
                folder: folder.into(),
                socket: socket.into(),
                token,
                file: file.into(),
            })
        })
        .collect()
    }

    /// Ends the registration of the Claude Code session `session`: what its socket took and its
    /// file did not confirm waits for the other paths.
    pub fn end_claude(&self, session: &str) -> Result<(), Error> {
        let tx = self.write()?;
        tx.execute(
            "UPDATE handover SET failed = 1 WHERE written = ?1",
            params![session],
        )?;
        tx.execute(
            "DELETE FROM claude_session WHERE session = ?1",
            params![session],
        )?;
        tx.execute(
            "DELETE FROM socket_line WHERE session = ?1",
            params![session],
        )?;
        tx.execute(
            "DELETE FROM socket_wake WHERE write NOT IN (SELECT write FROM socket_line)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Registers the Codex session `session`, or updates its registration, as the one whose hook
    /// ran last: a session file or config home it does not give stays as registered.
    pub fn register_codex(&self, session: &CodexSession) -> Result<(), Error> {
        let file = session.file.as_deref().map(path_text).transpose()?;
        let home = session.home.as_deref().map(path_text).transpose()?;
        self.db.execute(
            "INSERT INTO codex_session
                 (thread, space, agent, folder, file, home, process, busy, hooked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                 (SELECT COALESCE(MAX(hooked), 0) + 1 FROM codex_session))
             ON CONFLICT (thread) DO UPDATE SET
                 space = excluded.space, agent = excluded.agent, folder = excluded.folder,
                 file = COALESCE(excluded.file, file), home = COALESCE(excluded.home, home),
                 process = COALESCE(excluded.process, process), busy = excluded.busy,
                 hooked = excluded.hooked",
            params![
                session.thread,
                session.space.to_string(),
                session.agent,
                path_text(&session.folder)?,
                file,
                home,
                session.process,
                session.busy,
            ],
        )?;
        Ok(())
    }

    /// The registered Codex sessions, the one whose hook ran last first.
    pub fn codex_sessions(&self) -> Result<Vec<CodexSession>, Error> {
        let mut query = self.db.prepare(
            "SELECT thread, space, agent, folder, file, home, process, busy, hooked
             FROM codex_session ORDER BY hooked DESC",
        )?;
        let rows = query.query_map([], |row| {
            Ok((
                texts::<4>(row, 0)?,
                [row.get::<_, Option<String>>(4)?, row.get(5)?],
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, bool>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })?;
        rows.map(|row| {
            let ([thread, space, agent, folder], [file, home], process, busy, hooked) = row?;
            let malformed = || Error::Malformed(format!("the Codex session {thread}"));
            Ok(CodexSession {
                space: space.parse().map_err(|_| malformed())?,
                agent,
                folder: folder.into(),
                file: file.map(PathBuf::from),
                home: home.map(PathBuf::from),
                process: process
                    .map(|process| u32::try_from(process).map_err(|_| malformed()))
                    .transpose()?,
                busy,
                hooked: u64::try_from(hooked).map_err(|_| malformed())?,
                thread,
            })
        })
        .collect()
    }

    /// Ends the registration of the Codex session `thread`: the signals that went to it go to
    /// another session of their agent.
    pub fn end_codex(&self, thread: &str) -> Result<(), Error> {
        let tx = self.write()?;
        tx.execute(
            "UPDATE handover SET thread = NULL WHERE thread = ?1",
            params![thread],
        )?;
        tx.execute(
            "DELETE FROM codex_session WHERE thread = ?1",
            params![thread],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The `seq` of each kept message that waits for the agent `agent` whose signal goes to a
    /// Codex session, and that session's thread.
    pub fn codex_signals(&self, agent: &str) -> Result<Vec<(u64, String)>, Error> {
        self.lines(
            "SELECT seq, thread FROM handover WHERE agent = ?1 AND thread IS NOT NULL
             ORDER BY seq",
            params![agent],
        )
    }

    /// Records that a hook of the agent `agent` gave the signal of each of the kept messages
    /// `seqs`.
    pub fn signals_given(&self, seqs: &[u64], agent: &str) -> Result<(), Error> {
        let tx = self.write()?;
        for seq in seqs {
            tx.execute(
                "UPDATE handover SET signalled = 1 WHERE seq = ?1 AND agent = ?2",
                params![sql_seq(*seq)?, agent],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records that the signal of each of the kept messages `seqs` that wait for the agent `agent`
    /// goes to the Codex session `thread`.
    pub fn signal_to(&self, seqs: &[u64], agent: &str, thread: &str) -> Result<(), Error> {
        let tx = self.write()?;
        for seq in seqs {
            tx.execute(
                "UPDATE handover SET thread = ?3 WHERE seq = ?1 AND agent = ?2",
                params![sql_seq(*seq)?, agent, thread],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The `seq` of each kept message that waits for the agent `agent` that a session's socket
    /// took.
    pub fn written(&self, agent: &str) -> Result<Vec<u64>, Error> {
        let written: Vec<(u64, String)> = self.lines(
            "SELECT seq, written FROM handover WHERE agent = ?1 AND written IS NOT NULL
             ORDER BY seq",
            params![agent],
        )?;
        Ok(written.into_iter().map(|(seq, _)| seq).collect())
    }

    /// Records that the socket of the Claude Code session `session` takes the kept messages
    /// `messages` that wait for the agent `agent`, and the signal of `signalled` in its line
    /// `signal`, before they are written to it.
    pub fn socket_takes(
        &self,
        (session, agent): (&str, &str),
        messages: &[u64],
        (signalled, signal): (&[u64], &str),
    ) -> Result<(), Error> {
        let tx = self.write()?;
        for seq in messages {
            tx.execute(
                "UPDATE handover SET written = ?3, failed = 0 WHERE seq = ?1 AND agent = ?2",
                params![sql_seq(*seq)?, agent, session],
            )?;
        }
        for seq in signalled {
            tx.execute(
                "UPDATE handover SET signalled = 1, signal = ?3 WHERE seq = ?1 AND agent = ?2",
                params![sql_seq(*seq)?, agent, signal],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records that a write of the kept messages `seqs` that wait for the agent `agent`, or of
    /// their signal, to a session's socket failed: they wait as before it.
    pub fn socket_failed(&self, seqs: &[u64], agent: &str) -> Result<(), Error> {
        let tx = self.write()?;
        for seq in seqs {
            tx.execute(
                "UPDATE handover SET written = NULL, failed = 0, signalled = 0, signal = NULL
                 WHERE seq = ?1 AND agent = ?2",
                params![sql_seq(*seq)?, agent],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records that the Claude Code session `session` said its line `line` did not reach the
    /// model: the message waits for the other paths, or its signal for the hooks. Once it said so
    /// of each line of a write, the write's wakes are taken back ([`State::socket_wakes`]).
    pub fn not_taken(&self, session: &str, line: &str) -> Result<(), Error> {
        let tx = self.write()?;
        let write: Option<String> = tx
            .query_row(
                "SELECT write FROM socket_line WHERE session = ?1 AND line = ?2",
                params![session, line],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(write) = write {
            tx.execute(
                "DELETE FROM socket_line WHERE session = ?1 AND line = ?2",
                params![session, line],
            )?;
            let left: i64 = tx.query_row(
                "SELECT count(*) FROM socket_line WHERE write = ?1",
                params![write],
                |row| row.get(0),
            )?;
            if left == 0 {
                tx.execute(
                    "UPDATE channel_wakes SET wakes = wakes - 1 WHERE wakes > 0
                     AND channel IN (SELECT channel FROM socket_wake WHERE write = ?1)",
                    params![write],
                )?;
                tx.execute("DELETE FROM socket_wake WHERE write = ?1", params![write])?;
            }
        }
        tx.execute(
            "UPDATE handover SET failed = 1 WHERE written = ?1
                 AND seq IN (SELECT seq FROM kept WHERE msg_id = ?2)",
            params![session, line],
        )?;
        tx.execute(
            "UPDATE handover SET signalled = 0, signal = NULL WHERE signal = ?1",
            params![line],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Records that the session file of the Claude Code session `session` confirms the messages
    /// of `msg_ids` its socket took: each is handed over to its agent ([`State::handed_over`]),
    /// and the wakes of its write stand. Gives how many it confirms.
    pub fn confirmed(&self, session: &str, msg_ids: &[String]) -> Result<usize, Error> {
        let mut confirmed = Vec::new();
        for msg_id in msg_ids {
            let write: Option<String> = (self.db)
                .query_row(
                    "SELECT write FROM socket_line WHERE session = ?1 AND line = ?2",
                    params![session, msg_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(write) = write {
                self.db
                    .execute("DELETE FROM socket_line WHERE write = ?1", params![write])?;
                self.db
                    .execute("DELETE FROM socket_wake WHERE write = ?1", params![write])?;
            }
            let rows: Vec<(u64, String)> = self.lines(
                "SELECT handover.seq, handover.agent FROM handover
                 JOIN kept ON kept.seq = handover.seq
                 WHERE handover.written = ?1 AND kept.msg_id = ?2",
                params![session, msg_id],
            )?;
            confirmed.extend(rows);
        }
        for (seq, agent) in &confirmed {
            self.handed_over(*seq, agent)?;
        }
        Ok(confirmed.len())
    }

    /// Sets the `Stop` continuations in a row of the session of the agent `agent` in the space
    /// `space` to `to`, or, without it, counts one more; gives how many there are now, none for an
    /// agent the device has not run as there.
    pub fn count_stops(&self, space: SpaceId, agent: &str, to: Option<u64>) -> Result<u64, Error> {
        let space = space.to_string();
        match to {
            Some(to) => self.db.execute(
                "UPDATE agent SET stops = ?3 WHERE space = ?1 AND name = ?2",
                params![space, agent, sql_u64(to)?],
            )?,
            None => self.db.execute(
                "UPDATE agent SET stops = stops + 1 WHERE space = ?1 AND name = ?2",
                params![space, agent],
            )?,
        };
        let stops: Option<i64> = (self.db)
            .query_row(
                "SELECT stops FROM agent WHERE space = ?1 AND name = ?2",
                params![space, agent],
                |row| row.get(0),
            )
            .optional()?;
        Ok(stops
            .and_then(|stops| u64::try_from(stops).ok())
            .unwrap_or(0))
    }

    /// The highest `hops` of the messages handed over to the agent `agent` in the space `space`
    /// since the device last sent a message as it, if any was.
    pub fn handed_hops(&self, space: SpaceId, agent: &str) -> Result<Option<u64>, Error> {
        let highest: Option<i64> = self
            .db
            .query_row(
                "SELECT highest FROM agent_hops WHERE space = ?1 AND agent = ?2",
                params![space.to_string(), agent],
                |row| row.get(0),
            )
            .optional()?;
        highest
            .map(|highest| {
                u64::try_from(highest).map_err(|_| Error::Malformed(format!("the hops {highest}")))
            })
            .transpose()
    }

    /// The kept messages that wait for the person's decision for an agent by `now`, in Unix
    /// seconds: those whose wait has not ended, in `seq` order, then by agent.
    pub fn held(&self, now: u64) -> Result<Vec<Held>, Error> {
        let rows = held_rows(&self.db, now, false)?.into_iter();
        rows.map(|(seq, agent, space, taken, line)| {
            let malformed = || Error::Malformed(format!("the held message {seq}"));
            Ok(Held {
                seq: u64::try_from(seq).map_err(|_| malformed())?,
                agent,
                space: space.parse().map_err(|_| malformed())?,
                taken: u64::try_from(taken).map_err(|_| malformed())?,
                line,
            })
        })
        .collect()
    }

    /// Records the person's acceptance of the kept message `seq` for the agent `agent`, if it
    /// waits for that decision by `now`, in Unix seconds: it waits for the agent then, as any
    /// other. Gives whether it waited.
    pub fn accept_held(&self, seq: u64, agent: &str, now: u64) -> Result<bool, Error> {
        let seq = sql_seq(seq)?;
        let tx = self.write()?;
        let waited = tx.execute(
            "DELETE FROM held_message WHERE seq = ?1 AND agent = ?2
             AND seq IN (SELECT seq FROM kept WHERE expires > ?3)",
            params![seq, agent, sql_u64(now)?],
        )?;
        if waited > 0 {
            tx.execute(
                "INSERT INTO handover (seq, agent) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                params![seq, agent],
            )?;
        }
        tx.commit()?;
        Ok(waited > 0)
    }

    /// Records the person's refusal of the kept message `seq` for the agent `agent`, with
    /// `reason`, if any, if it waits for that decision by `now`, in Unix seconds, as
    /// [`refuse_held`] does. Gives whether it waited.
    pub fn refuse_held(
        &self,
        seq: u64,
        agent: &str,
        reason: Option<&str>,
        now: u64,
    ) -> Result<bool, Error> {
        let tx = self.write()?;
        let waited: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM held_message JOIN kept ON kept.seq = held_message.seq
                 WHERE held_message.seq = ?1 AND agent = ?2 AND kept.expires > ?3)",
            params![sql_seq(seq)?, agent, sql_u64(now)?],
            |row| row.get(0),
        )?;
        if waited {
            refuse_held(&tx, sql_seq(seq)?, agent, reason)?;
        }
        tx.commit()?;
        Ok(waited)
    }

    /// Refuses, for `reason`, each kept message that waits for the person's decision for an agent
    /// and whose wait ended by `now`, in Unix seconds, as [`refuse_held`] does.
    pub fn end_held_waits(&self, now: u64, reason: &str) -> Result<(), Error> {
        let tx = self.write()?;
        for (seq, agent, ..) in held_rows(&tx, now, true)? {
            refuse_held(&tx, seq, &agent, Some(reason))?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The messages the person, or else the agent `agent` in the space `space`, can reply to by
    /// `now`, in Unix seconds, in `seq` order, once those whose time ended are dropped.
    pub fn targets(&self, agent: Option<(SpaceId, &str)>, now: u64) -> Result<Vec<Target>, Error> {
        self.kept_targets(agent, true, now)
    }

    /// The messages the person's run showed or the device handed to an agent, or else those
    /// handed to the agent `agent` in the space `space`, that it keeps for replies by `now`, in
    /// Unix seconds, in `seq` order, once those whose time ended are dropped.
    pub fn handed(&self, agent: Option<(SpaceId, &str)>, now: u64) -> Result<Vec<Target>, Error> {
        self.kept_targets(agent, false, now)
    }

    /// The agents the kept message of the entry `seq` was handed to, by name.
    pub fn handed_to(&self, seq: u64) -> Result<Vec<String>, Error> {
        let mut query = self
            .db
            .prepare("SELECT agent FROM reply_agent WHERE seq = ?1 ORDER BY agent")?;
        let names = query.query_map(params![sql_seq(seq)?], |row| row.get(0))?;
        Ok(names.collect::<Result<_, _>>()?)
    }

    /// The `msg_id` of each message the device sent that the server stored and whose wait has not
    /// ended by `now`, in Unix seconds, of the person, or else those the agent `agent` sent in the
    /// space `space`, once where those whose wait ended went is dropped.
    pub fn sent_ids(
        &self,
        agent: Option<(SpaceId, &str)>,
        now: u64,
    ) -> Result<Vec<MessageId>, Error> {
        let now = sql_u64(now)?;
        let tx = self.write()?;
        tx.execute(
            "DELETE FROM sent_status WHERE stored + wait <= ?1",
            params![now],
        )?;
        tx.execute(
            "DELETE FROM sent_recipient WHERE msg_id NOT IN (SELECT msg_id FROM sent_status)",
            [],
        )?;
        tx.commit()?;
        let (space, agent) = agent.unzip();
        let mut query = self.db.prepare(
            "SELECT status.msg_id FROM sent_status AS status JOIN sent USING (msg_id)
             WHERE status.stored IS NOT NULL
                 AND (?1 IS NULL OR (sent.space = ?1 AND sent.agent = ?2))
             ORDER BY status.stored, status.msg_id",
        )?;
        let ids = query.query_map(
            params![space.map(|space| space.to_string()), agent],
            |row| row.get::<_, String>(0),
        )?;
        ids.map(|msg_id| id("message ID", &msg_id?)).collect()
    }

    /// Where the message `msg_id` the device sent is, if the server stored it and the device keeps
    /// it.
    pub fn sent_status(&self, msg_id: MessageId) -> Result<Option<SentStatus>, Error> {
        let row: Option<SentRow> = self
            .db
            .query_row(
                "SELECT sent.space, sent.channel, sent.agent, sent.addressee, status.reason,
                     status.stored, status.held, status.read, status.replied, status.refused
                 FROM sent_status AS status JOIN sent USING (msg_id)
                 WHERE msg_id = ?1 AND status.stored IS NOT NULL",
                params![msg_id.to_string()],
                |row| {
                    Ok((
                        [row.get(0)?, row.get(1)?],
                        [row.get(2)?, row.get(3)?, row.get(4)?],
                        [
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                            row.get(9)?,
                        ],
                    ))
                },
            )
            .optional()?;
        let Some((
            [space, channel],
            [agent, addressee, reason],
            [stored, held, read, replied, refused],
        )) = row
        else {
            return Ok(None);
        };
        let time = |time: Option<i64>| {
            time.map(|time| {
                u64::try_from(time).map_err(|_| Error::Malformed(format!("the time {time}")))
            })
            .transpose()
        };
        let mut query = self.db.prepare(
            "SELECT device, name, account, handle, status, timestamp FROM sent_recipient
             WHERE msg_id = ?1 AND status IS NOT NULL ORDER BY rowid",
        )?;
        let rows = query.query_map(params![msg_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                [row.get(1)?, row.get(2)?, row.get(3)?],
                row.get::<_, String>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })?;
        let recipients = rows
            .map(|row| {
                let (device, [name, account, handle], status, timestamp) = row?;
                Ok(RecipientStatus {
                    device: id("device ID", &device)?,
                    name,
                    account,
                    handle,
                    status,
                    timestamp: time(timestamp)?.unwrap_or(0),
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok(Some(SentStatus {
            msg_id,
            space: id("space ID", &space)?,
            channel: id("channel ID", &channel)?,
            agent,
            addressee,
            stored: time(stored)?.unwrap_or(0),
            held: time(held)?,
            read: time(read)?,
            replied: time(replied)?,
            refused: time(refused)?,
            reason,
            recipients,
        }))
    }

    /// [`State::targets`] if `person_only`, otherwise [`State::handed`].
    fn kept_targets(
        &self,
        agent: Option<(SpaceId, &str)>,
        person_only: bool,
        now: u64,
    ) -> Result<Vec<Target>, Error> {
        let now = sql_u64(now)?;
        let tx = self.write()?;
        tx.execute("DELETE FROM reply_target WHERE expires <= ?1", params![now])?;
        tx.execute(
            "DELETE FROM reply_agent WHERE seq NOT IN (SELECT seq FROM reply_target)",
            [],
        )?;
        tx.commit()?;
        let columns = "SELECT seq, msg_id, space, channel, target, line FROM reply_target";
        let mut query = match agent {
            Some(_) => self.db.prepare(&format!(
                "{columns} WHERE space = ?1
                 AND seq IN (SELECT seq FROM reply_agent WHERE agent = ?2) ORDER BY seq"
            ))?,
            None if person_only => self
                .db
                .prepare(&format!("{columns} WHERE person = 1 ORDER BY seq"))?,
            None => self.db.prepare(&format!("{columns} ORDER BY seq"))?,
        };
        let row = |row: &rusqlite::Row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, Option<Vec<u8>>>(5)?,
            ))
        };
        let rows = match agent {
            Some((space, agent)) => query
                .query_map(params![space.to_string(), agent], row)?
                .collect::<Result<Vec<_>, _>>()?,
            None => query.query_map([], row)?.collect::<Result<Vec<_>, _>>()?,
        };
        rows.into_iter()
            .map(|(seq, msg_id, space, channel, sealed, line)| {
                let malformed = || Error::Malformed(format!("the message to reply to {msg_id}"));
                Ok(Target {
                    seq: u64::try_from(seq).map_err(|_| malformed())?,
                    msg_id: msg_id.parse().map_err(|_| malformed())?,
                    space: space.parse().map_err(|_| malformed())?,
                    channel: channel.parse().map_err(|_| malformed())?,
                    sealed,
                    line,
                })
            })
            .collect()
    }

    /// The `seq` and the value of each row `sql` selects with `params`, such as a kept message's
    /// sealed line.
    fn lines<T: rusqlite::types::FromSql>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<(u64, T)>, Error> {
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
        let tx = self.write()?;
        add_counters(&tx, seq, source, counters)?;
        add_kept(&tx, seq, kept)?;
        tx.execute("DELETE FROM inbox WHERE seq = ?1", params![seq])?;
        tx.commit()?;
        Ok(())
    }

    /// Takes the receipt of the entry `seq` out of the inbox, recording `counted`, the counters
    /// the device accepted from its source with it, or, without `counter`, that it came from the
    /// device `sender` as `msg_id`; and keeps each of `reads`, `read` or `refused` statuses of
    /// messages the device sent, by their `msg_id`, as JSON, for the person and the agent that sent
    /// the message, unless the device took a `read` or `refused` of that message before; or, if
    /// `held`, `held` statuses, unless it took a `held`, `read` or `refused` of it before; all at
    /// once.
    pub fn take_receipt(
        &self,
        seq: u64,
        (counted, (sender, msg_id)): (Option<(&Source, &Counters)>, (DeviceId, MessageId)),
        (held, reads): (bool, &[(MessageId, String)]),
    ) -> Result<(), Error> {
        let seq = sql_seq(seq)?;
        let tx = self.write()?;
        match counted {
            Some((source, counters)) => add_counters(&tx, seq, source, counters)?,
            None => {
                tx.execute(
                    "INSERT INTO received (sender, msg_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                    params![sender.to_string(), msg_id.to_string()],
                )?;
            }
        }
        let first = if held {
            "UPDATE sent SET held = 1 WHERE msg_id = ?1 AND held = 0 AND read = 0"
        } else {
            "UPDATE sent SET read = 1 WHERE msg_id = ?1 AND read = 0"
        };
        for (read, status) in reads {
            let first = tx.execute(first, params![read.to_string()])?;
            if first > 0 {
                tx.execute(
                    "INSERT INTO kept_status (msg_id, space, status, person, agent)
                     SELECT msg_id, space, ?2, 1, agent FROM sent WHERE msg_id = ?1",
                    params![read.to_string(), status],
                )?;
            }
            let fields = Fields::of(status);
            if held {
                tx.execute(
                    "UPDATE sent_status SET held = ?2
                     WHERE msg_id = ?1 AND held IS NULL AND read IS NULL AND refused IS NULL",
                    params![read.to_string(), fields.time()],
                )?;
                continue;
            }
            let refused = (fields.text("status") == Some("refused")).then(|| fields.text("reason"));
            keep_answer(&tx, &read.to_string(), fields.time(), refused)?;
        }
        tx.execute("DELETE FROM inbox WHERE seq = ?1", params![seq])?;
        tx.commit()?;
        Ok(())
    }

    /// The receipts the device owes, in `seq` order, a message's `held` before its `read` or
    /// `refused`.
    pub fn reads_due(&self) -> Result<Vec<ReadDue>, Error> {
        let mut query = self.db.prepare(
            "SELECT seq, msg_id, space, channel, sender, author, addressee, NULL, 1, 0
             FROM read_receipt WHERE held_due = 1
             UNION ALL
             SELECT seq, msg_id, space, channel, sender, author, addressee, reason, 0, refused
             FROM read_receipt WHERE due = 1
             ORDER BY 1, 9 DESC",
        )?;
        let rows = query.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                [
                    row.get::<_, String>(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ],
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                (row.get::<_, bool>(8)?, row.get::<_, bool>(9)?),
            ))
        })?;
        rows.map(|row| {
            let (seq, [msg_id, space, channel, sender], author, addressee, reason, status) = row?;
            Ok(ReadDue {
                seq: u64::try_from(seq).map_err(|_| Error::Malformed(format!("the seq {seq}")))?,
                msg_id: id("message ID", &msg_id)?,
                space: id("space ID", &space)?,
                channel: id("channel ID", &channel)?,
                sender: id("device ID", &sender)?,
                author,
                addressee,
                status: match status {
                    (true, _) => ReceiptStatus::Held,
                    (false, true) => ReceiptStatus::Refused,
                    (false, false) => ReceiptStatus::Read,
                },
                reason,
            })
        })
        .collect()
    }

    /// The envelopes in the outbox, in the order they go out.
    pub fn outbox(&self) -> Result<Vec<Outgoing>, Error> {
        let mut query = self.db.prepare(
            "SELECT msg_id, recipient, space, envelope, report FROM outbox ORDER BY position",
        )?;
        let rows = query.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?;
        rows.map(|row| {
            let (msg_id, recipient, space, envelope, report) = row?;
            Ok(Outgoing {
                msg_id: id("message ID", &msg_id)?,
                recipient: recipient
                    .map(|recipient| id("device ID", &recipient))
                    .transpose()?,
                space: id("space ID", &space)?,
                envelope,
                report,
            })
        })
        .collect()
    }

    /// Takes the envelope of `msg_id` to `recipient`, none under `mls`, out of the outbox: the
    /// server answered it. If it stored the envelope of an own message of the device's
    /// ([`OwnMessage`]), the device keeps that message for itself, and its recipient devices, or
    /// the copy's, are `stored` now. If it refused the envelope, `refused` its refusal as a status,
    /// as JSON, the message's history entry, own message and where it goes go too unless the
    /// envelope is a copy to `recipient`, which alone is then no recipient, and the refusal is kept
    /// for the agent that sent the message, if any, all at once.
    pub fn remove_outgoing(
        &self,
        msg_id: MessageId,
        recipient: Option<DeviceId>,
        refused: Option<&str>,
    ) -> Result<(), Error> {
        let tx = self.write()?;
        tx.execute(
            "DELETE FROM outbox WHERE msg_id = ?1 AND recipient IS ?2",
            params![
                msg_id.to_string(),
                recipient.map(|device| device.to_string())
            ],
        )?;
        // A copy to a `service` device leaves the channel's message as it is.
        match (recipient, refused) {
            (Some(_), _) => {}
            (None, None) => keep_own(&tx, msg_id)?,
            (None, Some(_)) => {
                for table in [
                    "own_message",
                    "history_outbox",
                    "sent_status",
                    "sent_recipient",
                ] {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE msg_id = ?1"),
                        params![msg_id.to_string()],
                    )?;
                }
            }
        }
        track_answer(&tx, msg_id, recipient, refused.is_none())?;
        if let Some(refused) = refused {
            tx.execute(
                "INSERT INTO kept_status (msg_id, space, status, person, agent)
                 SELECT msg_id, space, ?2, 0, agent FROM sent
                 WHERE msg_id = ?1 AND agent IS NOT NULL",
                params![msg_id.to_string(), refused],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The history entries of the device's messages whose envelopes the server stored, which wait
    /// for upload, in the order they went into the outbox.
    pub fn history_outbox(&self) -> Result<Vec<HistoryOutgoing>, Error> {
        let rows = self.envelopes(
            "SELECT msg_id, space, channel, entry FROM history_outbox
             WHERE msg_id NOT IN (SELECT msg_id FROM outbox) ORDER BY position",
        )?;
        rows.into_iter()
            .map(|([msg_id, space, channel], entry)| {
                Ok(HistoryOutgoing {
                    msg_id: id("message ID", &msg_id)?,
                    space: id("space ID", &space)?,
                    channel: id("channel ID", &channel)?,
                    entry,
                })
            })
            .collect()
    }

    /// Takes the history entry of the message `msg_id` out of the outbox: the server answered it.
    pub fn remove_history_entry(&self, msg_id: MessageId) -> Result<(), Error> {
        self.db.execute(
            "DELETE FROM history_outbox WHERE msg_id = ?1",
            params![msg_id.to_string()],
        )?;
        Ok(())
    }

    /// Keeps `outgoing`, a task enqueued as `agent`, if any, until the server answers it, and
    /// whose it is for good; the agent's next message or task counts only the `hops` of what is
    /// handed to it from now on, as a message it sends does.
    pub fn keep_task(&self, outgoing: &TaskOutgoing, agent: Option<&str>) -> Result<(), Error> {
        let tx = self.write()?;
        let space = outgoing.space.to_string();
        tx.execute(
            "INSERT INTO task_outbox (task, space, channel, queue, record)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                outgoing.task.to_string(),
                space,
                outgoing.channel.to_string(),
                outgoing.queue.to_string(),
                outgoing.record
            ],
        )?;
        tx.execute(
            "INSERT INTO enqueued_task (task, space, agent) VALUES (?1, ?2, ?3)",
            params![outgoing.task.to_string(), space, agent],
        )?;
        if let Some(agent) = agent {
            tx.execute(
                "DELETE FROM agent_hops WHERE space = ?1 AND agent = ?2",
                params![space, agent],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The tasks that wait for the server's answer, in the order they were kept.
    pub fn task_outbox(&self) -> Result<Vec<TaskOutgoing>, Error> {
        let mut query = self.db.prepare(
            "SELECT task, space, channel, queue, record FROM task_outbox ORDER BY position",
        )?;
        let rows = query.query_map([], |row| {
            Ok((
                [row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?],
                row.get(4)?,
            ))
        })?;
        let rows: Vec<([String; 4], Vec<u8>)> = rows.collect::<Result<_, _>>()?;
        rows.into_iter()
            .map(|([task, space, channel, queue], record)| {
                Ok(TaskOutgoing {
                    task: id("task ID", &task)?,
                    space: id("space ID", &space)?,
                    channel: id("channel ID", &channel)?,
                    queue: id("queue ID", &queue)?,
                    record,
                })
            })
            .collect()
    }

    /// The tasks the device enqueued in `space` as `agent`.
    pub fn enqueued_by(&self, space: SpaceId, agent: &str) -> Result<BTreeSet<TaskId>, Error> {
        let mut query = self
            .db
            .prepare("SELECT task FROM enqueued_task WHERE space = ?1 AND agent = ?2")?;
        let tasks = query.query_map(params![space.to_string(), agent], |row| {
            row.get::<_, String>(0)
        })?;
        tasks.map(|task| id("task ID", &task?)).collect()
    }

    /// Takes the task `task` out of the outbox: the server answered it.
    pub fn remove_task(&self, task: TaskId) -> Result<(), Error> {
        self.db.execute(
            "DELETE FROM task_outbox WHERE task = ?1",
            params![task.to_string()],
        )?;
        Ok(())
    }

    /// Keeps `claim`, a claim the device now holds, or its new time and sealed part; with `hops`,
    /// the agent, by the name it counts under ([`State::handed_hops`]), and the task's `hops`, the
    /// task counts for that agent's next send as a message handed over to it does.
    pub fn keep_claim(&self, claim: &TaskClaim, hops: Option<(&str, u64)>) -> Result<(), Error> {
        let tx = Transaction::new_unchecked(&self.db, TransactionBehavior::Immediate)?;
        let space = claim.space.to_string();
        tx.execute(
            "INSERT INTO task_claim (task, space, channel, queue, agent, since, sealed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (task) DO UPDATE SET since = excluded.since, sealed = excluded.sealed",
            params![
                claim.task.to_string(),
                space,
                claim.channel.to_string(),
                claim.queue.to_string(),
                claim.agent,
                sql_u64(claim.since)?,
                claim.sealed
            ],
        )?;
        if let Some((agent, hops)) = hops {
            // A sender may set any `hops`; past what SQLite holds, it counts as the highest.
            tx.execute(
                "INSERT INTO agent_hops (space, agent, highest) VALUES (?1, ?2, ?3)
                 ON CONFLICT DO UPDATE SET highest = max(highest, excluded.highest)",
                params![space, agent, i64::try_from(hops).unwrap_or(i64::MAX)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The claims the device holds, of the space `space` if given, in the order of their tasks'
    /// IDs.
    pub fn claims(&self, space: Option<SpaceId>) -> Result<Vec<TaskClaim>, Error> {
        let mut query = self.db.prepare(
            "SELECT task, space, channel, queue, agent, since, sealed FROM task_claim
             WHERE ?1 IS NULL OR space = ?1 ORDER BY task",
        )?;
        let rows = query.query_map(params![space.map(|space| space.to_string())], |row| {
            Ok((
                [
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                ],
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Vec<u8>>(6)?,
            ))
        })?;
        let rows: Vec<_> = rows.collect::<Result<_, _>>()?;
        rows.into_iter()
            .map(|([task, space, channel, queue], agent, since, sealed)| {
                Ok(TaskClaim {
                    task: id("task ID", &task)?,
                    space: id("space ID", &space)?,
                    channel: id("channel ID", &channel)?,
                    queue: id("queue ID", &queue)?,
                    agent,
                    since: u64::try_from(since)
                        .map_err(|_| Error::Malformed(format!("the time {since}")))?,
                    sealed,
                })
            })
            .collect()
    }

    /// Takes the claim of the task `task` out of those the device holds: it ended or is lost.
    pub fn remove_claim(&self, task: TaskId) -> Result<(), Error> {
        self.db.execute(
            "DELETE FROM task_claim WHERE task = ?1",
            params![task.to_string()],
        )?;
        Ok(())
    }
}

/// The `N` text columns of `row` from `first` on.
fn texts<const N: usize>(row: &rusqlite::Row, first: usize) -> rusqlite::Result<[String; N]> {
    let mut texts: [String; N] = std::array::from_fn(|_| String::new());
    for (at, text) in texts.iter_mut().enumerate() {
        *text = row.get(first + at)?;
    }
    Ok(texts)
}

/// `path` as the state keeps it: text; fails on one that is not UTF-8.
fn path_text(path: &Path) -> Result<String, Error> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Malformed(format!("the path {}", path.display())))
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

/// Now, in Unix milliseconds.
pub fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Keeps the message of the inbox entry `seq` for the person and the agents `kept` names, in `tx`;
/// it waits for them `kept.wait` after the device wrote the entry, or after now if it did not
/// record when.
fn add_kept(tx: &Transaction, seq: i64, kept: &Kept) -> Result<(), Error> {
    let now = i64::try_from(unix_now()).unwrap_or(i64::MAX);
    let wait = i64::try_from(kept.wait).unwrap_or(i64::MAX);
    tx.execute(
        "INSERT INTO kept (seq, space, line, expires, person, msg_id, channel, target, hops, came)
         SELECT ?1, ?2, ?3, COALESCE((SELECT written FROM inbox WHERE seq = ?1), ?4) + ?5, 1, ?6,
             ?7, ?8, ?9, ?10
         WHERE true ON CONFLICT DO NOTHING",
        params![
            seq,
            kept.space.to_string(),
            kept.line,
            now,
            wait,
            kept.msg_id.to_string(),
            kept.channel.to_string(),
            kept.target,
            // A sender may set any `hops`; past what SQLite holds, it counts as the highest.
            i64::try_from(kept.hops).unwrap_or(i64::MAX),
            sql_u64(unix_millis())?
        ],
    )?;
    if let Some(replied) = &kept.replied {
        tx.execute(
            "INSERT INTO kept_status (seq, msg_id, space, status, person, agent)
             VALUES (?1, ?2, ?3, ?4, 1, ?5) ON CONFLICT DO NOTHING",
            params![
                seq,
                replied.msg_id.to_string(),
                replied.space.to_string(),
                replied.status,
                replied.agent
            ],
        )?;
        tx.execute(
            "UPDATE sent_status SET replied = ?2 WHERE msg_id = ?1 AND replied IS NULL",
            params![
                replied.msg_id.to_string(),
                Fields::of(&replied.status).time()
            ],
        )?;
    }
    for agent in &kept.agents {
        tx.execute(
            "INSERT INTO handover (seq, agent) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
            params![seq, agent],
        )?;
    }
    for agent in &kept.held {
        tx.execute(
            "INSERT INTO held_message (seq, agent, taken)
             VALUES (?1, ?2, COALESCE((SELECT written FROM inbox WHERE seq = ?1), ?3))
             ON CONFLICT DO NOTHING",
            params![seq, agent, now],
        )?;
    }
    if let Some(read) = &kept.read {
        tx.execute(
            "INSERT INTO read_receipt (seq, msg_id, space, channel, sender, author, addressee,
                 agent, due, reason, refused, held_due)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?9, ?11) ON CONFLICT DO NOTHING",
            params![
                seq,
                kept.msg_id.to_string(),
                kept.space.to_string(),
                read.channel.to_string(),
                read.sender.to_string(),
                read.author,
                read.addressee,
                read.agent,
                read.reason.is_some(),
                read.reason,
                read.held
            ],
        )?;
    }
    Ok(())
}

/// Counts, in `tx`, the `hops` of the kept message `seq` for the next send of `agent`
/// ([`State::handed_hops`]).
fn count_hops(tx: &Transaction, seq: i64, agent: &str) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO agent_hops (space, agent, highest)
         SELECT space, ?2, hops FROM kept WHERE seq = ?1 AND hops IS NOT NULL
         ON CONFLICT DO UPDATE SET highest = max(highest, excluded.highest)",
        params![seq, agent],
    )?;
    Ok(())
}

/// Adds, in `tx`, the kept message `seq` to those the device can reply to, unless it is, or an
/// older client kept it without what a reply needs; gives whether it is one now.
fn add_target(tx: &Transaction, seq: i64) -> Result<bool, Error> {
    tx.execute(
        "INSERT INTO reply_target (seq, msg_id, space, channel, target, expires, person, line)
         SELECT seq, msg_id, space, channel, target, expires, 0, line FROM kept
         WHERE seq = ?1 AND msg_id IS NOT NULL AND channel IS NOT NULL AND target IS NOT NULL
         ON CONFLICT DO NOTHING",
        params![seq],
    )?;
    let found: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM reply_target WHERE seq = ?1)",
        params![seq],
        |row| row.get(0),
    )?;
    Ok(found)
}

/// Records `counters` as those the device accepted from `source` with the entry `seq`, in `tx`.
fn add_counters(
    tx: &Transaction,
    seq: i64,
    source: &Source,
    counters: &Counters,
) -> Result<(), Error> {
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
    Ok(())
}

/// Drops, in `tx`, each kept message the person's run showed that no agent waits for, nor the
/// person's decision for an agent, and each `read` receipt of a message that no longer waits for
/// its agent before it had it, with no `held` due.
fn drop_done(tx: &Transaction) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM kept WHERE person = 0 AND seq NOT IN (SELECT seq FROM handover)
             AND seq NOT IN (SELECT seq FROM held_message)",
        [],
    )?;
    tx.execute(
        "DELETE FROM read_receipt WHERE due = 0 AND held_due = 0
             AND seq NOT IN (SELECT seq FROM handover)
             AND seq NOT IN (SELECT seq FROM held_message)",
        [],
    )?;
    tx.execute(
        "DELETE FROM own_read WHERE seq NOT IN (SELECT seq FROM handover)
             AND seq NOT IN (SELECT seq FROM held_message)",
        [],
    )?;
    Ok(())
}

/// A kept message that waits for the person's decision for an agent: its `seq`, the agent, its
/// space, when the device took it in and its sealed line.
type HeldRow = (i64, String, String, i64, Vec<u8>);

/// The kept messages in `db` that wait for the person's decision for an agent whose wait has, if
/// `ended`, or else has not ended by `now`, in Unix seconds, in `seq` order, then by agent.
fn held_rows(db: &rusqlite::Connection, now: u64, ended: bool) -> Result<Vec<HeldRow>, Error> {
    let mut query = db.prepare(
        "SELECT held.seq, held.agent, kept.space, held.taken, kept.line
         FROM held_message AS held JOIN kept ON kept.seq = held.seq
         WHERE (kept.expires <= ?1) = ?2 ORDER BY held.seq, held.agent",
    )?;
    let rows = query.query_map(params![sql_u64(now)?, ended], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Refuses, in `tx`, the kept message `seq` for the agent `agent`, which waits for the person's
/// decision for it, with `reason`, if any: it no longer waits, and its `refused` is due, with its
/// `held` if that has not gone yet; or, for an own message of the device's, its `refused` is kept
/// as a status for the person and the agent that sent it, stamped now, unless the device has a
/// `read` or `refused` of it already.
fn refuse_held(tx: &Transaction, seq: i64, agent: &str, reason: Option<&str>) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM held_message WHERE seq = ?1 AND agent = ?2",
        params![seq, agent],
    )?;
    tx.execute(
        "UPDATE read_receipt SET due = 1, refused = 1, reason = ?3 WHERE seq = ?1 AND agent = ?2",
        params![seq, agent, reason],
    )?;
    read_own(tx, seq, agent, Some(reason))?;
    drop_done(tx)
}

/// Keeps, in `tx`, the `read` of the device's own message of the kept entry `seq`, now handed to
/// its `to` agent `agent`, as a status for the person and the agent that sent it, stamped now,
/// unless the device has a `read` or `refused` of it already; with `refused`, the `refused` that
/// agent's device gives instead, with its reason, if any.
fn read_own(
    tx: &Transaction,
    seq: i64,
    agent: &str,
    refused: Option<Option<&str>>,
) -> Result<(), Error> {
    let read: Option<(String, String)> = tx
        .query_row(
            "SELECT msg_id, status FROM own_read WHERE seq = ?1 AND agent = ?2",
            params![seq, agent],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((msg_id, status)) = read else {
        return Ok(());
    };
    let first = tx.execute(
        "UPDATE sent SET read = 1 WHERE msg_id = ?1 AND read = 0",
        params![msg_id],
    )?;
    let now = sql_u64(unix_now())?;
    if first > 0 {
        tx.execute(
            "INSERT INTO kept_status (msg_id, space, status, person, agent)
             SELECT msg_id, space,
                 CASE WHEN ?4 = 0 THEN json_set(?2, '$.timestamp', ?3)
                     WHEN ?5 IS NULL THEN json_set(json_remove(?2, '$.reason'), '$.status',
                         'refused', '$.timestamp', ?3)
                     ELSE json_set(?2, '$.status', 'refused', '$.reason', ?5, '$.timestamp', ?3)
                 END, 1, agent
             FROM sent WHERE msg_id = ?1",
            params![msg_id, status, now, refused.is_some(), refused.flatten()],
        )?;
    }
    keep_answer(tx, &msg_id, Some(now), refused)?;
    tx.execute("DELETE FROM own_read WHERE seq = ?1", params![seq])?;
    Ok(())
}

/// Keeps, in `tx`, as the status of the device's message `msg_id`, its `to` agent's `read` at `at`
/// or, with `refused`, its `refused` at `at` with its reason, if any; unless it has either.
fn keep_answer(
    tx: &Transaction,
    msg_id: &str,
    at: Option<i64>,
    refused: Option<Option<&str>>,
) -> Result<(), Error> {
    tx.execute(
        "UPDATE sent_status
         SET read = iif(?3, read, ?2), refused = iif(?3, ?2, refused), reason = ?4
         WHERE msg_id = ?1 AND read IS NULL AND refused IS NULL",
        params![msg_id, at, refused.is_some(), refused.flatten()],
    )?;
    Ok(())
}

/// `value` as SQLite stores it; fails past `i64`.
fn sql_u64(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::Malformed(format!("the number {value}")))
}

/// Adds `message` to the outbox, after those in it, records its counter as its channel's last, or
/// for a copy as the last to its recipient there, and, unless it is a receipt, its space, channel,
/// agent and `to` as those of its message ID, and that its agent sent a message, in `tx`; fails
/// unless the counter is the one after that last.
fn add_message(tx: &Transaction, message: &Counted) -> Result<(), Error> {
    let Counted {
        outgoing,
        channel,
        counter,
        agent,
        addressee,
    } = message;
    let channel = channel.to_string();
    let recipient = outgoing.recipient.map(|device| device.to_string());
    let last =
        i64::try_from(*counter).map_err(|_| Error::Malformed(format!("the counter {counter}")))?;
    let changed = match (&recipient, last) {
        (None, 1) => tx.execute(
            "INSERT INTO channel_counter (channel, last) VALUES (?1, 1) ON CONFLICT DO NOTHING",
            params![channel],
        )?,
        (None, _) => tx.execute(
            "UPDATE channel_counter SET last = ?2 WHERE channel = ?1 AND last = ?2 - 1",
            params![channel, last],
        )?,
        (Some(recipient), 1) => tx.execute(
            "INSERT INTO copy_counter (channel, recipient, last) VALUES (?1, ?2, 1)
             ON CONFLICT DO NOTHING",
            params![channel, recipient],
        )?,
        (Some(recipient), _) => tx.execute(
            "UPDATE copy_counter SET last = ?3
             WHERE channel = ?1 AND recipient = ?2 AND last = ?3 - 1",
            params![channel, recipient, last],
        )?,
    };
    if changed == 0 {
        return Err(Error::Malformed(format!(
            "the counter {counter} is not the next in the channel {channel}{}",
            recipient.map_or_else(String::new, |device| format!(" to the device {device}"))
        )));
    }
    let (msg_id, space) = (outgoing.msg_id.to_string(), outgoing.space.to_string());
    tx.execute(
        "INSERT INTO outbox (msg_id, recipient, space, envelope, report)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![msg_id, recipient, space, outgoing.envelope, outgoing.report],
    )?;
    // A receipt is no message of the device's user: nothing reports on it.
    if outgoing.report {
        tx.execute(
            "INSERT INTO sent (msg_id, space, channel, agent, addressee) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT DO NOTHING",
            params![msg_id, space, channel, agent, addressee],
        )?;
        // The agent's next send counts only what is handed to it from now on; a message as no
        // agent is a member service agent's, or the person's, who has no count.
        tx.execute(
            "DELETE FROM agent_hops WHERE space = ?1 AND agent = ?2",
            params![space, agent.as_deref().unwrap_or(SERVICE_AGENT)],
        )?;
    }
    Ok(())
}

/// `epoch`, as SQLite keeps an epoch of `channel`.
fn read_epoch(channel: ChannelId, epoch: i64) -> Result<u64, Error> {
    u64::try_from(epoch)
        .map_err(|_| Error::Malformed(format!("an epoch of the channel {channel}: {epoch}")))
}

/// Adds `own` to the device's own messages that wait for the server, in `tx`, its `seq` as the last
/// it gave; fails unless that `seq` is the next.
fn add_own(tx: &Transaction, own: &OwnMessage) -> Result<(), Error> {
    let OwnMessage {
        seq,
        kept,
        person,
        sender,
        to,
        read,
        refusing,
        refused,
        holding,
        held,
    } = own;
    let seq = sql_seq(*seq)?;
    let changed = tx.execute(
        "UPDATE device SET own_seq = own_seq + 1 WHERE ?1 + own_seq + 1 = ?2",
        params![sql_u64(OWN_FIRST)?, seq],
    )?;
    if changed == 0 {
        return Err(Error::Malformed(format!(
            "the own seq {seq} is not the next"
        )));
    }
    let replied = kept.replied.as_ref();
    tx.execute(
        "INSERT INTO own_message (msg_id, seq, space, channel, line, target, wait, person, sender,
             addressed, addressee, read, replied, replied_status, replied_agent, refusing, refused,
             holding, held, hops)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
             ?19, ?20)",
        params![
            kept.msg_id.to_string(),
            seq,
            kept.space.to_string(),
            kept.channel.to_string(),
            kept.line,
            kept.target,
            sql_u64(kept.wait)?,
            person,
            sender,
            to.is_some(),
            to.clone().flatten(),
            read,
            replied.map(|replied| replied.msg_id.to_string()),
            replied.map(|replied| replied.status.clone()),
            replied.and_then(|replied| replied.agent.clone()),
            serde_json::to_string(refusing).expect("names encode"),
            refused,
            serde_json::to_string(holding).expect("names encode"),
            held,
            sql_u64(kept.hops)?,
        ],
    )?;
    Ok(())
}

/// Keeps the device's own message `msg_id`, which the server stored, for the person if it is to
/// show it and for each agent of this device it is for, in `tx`: each the device has run as in its
/// space but its sender and those that refuse or hold it, with a `to` only the agent it names; for
/// the person's decision for each that holds it. Its `replied` of the message it answers, if any,
/// is kept too, and its `read` is due once it is handed to its `to` agent; or its `refused` is kept
/// at once, if that agent refuses it; or its `held` at once, if that agent holds it.
fn keep_own(tx: &Transaction, msg_id: MessageId) -> Result<(), Error> {
    let msg_id = msg_id.to_string();
    let now = sql_u64(unix_now())?;
    let kept = tx.execute(
        "INSERT INTO kept (seq, space, line, expires, person, msg_id, channel, target, hops, came)
         SELECT seq, space, line, ?2 + wait, person, msg_id, channel, target, hops, ?3
         FROM own_message WHERE msg_id = ?1 ON CONFLICT DO NOTHING",
        params![msg_id, now, sql_u64(unix_millis())?],
    )?;
    if kept > 0 {
        // Each agent of the device in its space the message is for, but its sender and those
        // that refuse it.
        let agents = "FROM own_message AS own JOIN agent ON agent.space = own.space
             WHERE own.msg_id = ?1 AND agent.name IS NOT own.sender
                 AND (own.addressed = 0 OR agent.name = own.addressee)
                 AND agent.name NOT IN (SELECT value FROM json_each(own.refusing))";
        tx.execute(
            &format!(
                "INSERT INTO handover (seq, agent) SELECT own.seq, agent.name {agents}
                 AND agent.name NOT IN (SELECT value FROM json_each(own.holding))
                 ON CONFLICT DO NOTHING"
            ),
            params![msg_id],
        )?;
        tx.execute(
            &format!(
                "INSERT INTO held_message (seq, agent, taken) SELECT own.seq, agent.name, ?2 {agents}
                 AND agent.name IN (SELECT value FROM json_each(own.holding))
                 ON CONFLICT DO NOTHING"
            ),
            params![msg_id, now],
        )?;
        let held = tx.execute(
            "UPDATE sent SET held = 1 WHERE msg_id = ?1 AND held = 0 AND read = 0
             AND EXISTS (SELECT 1 FROM own_message WHERE msg_id = ?1 AND held IS NOT NULL)",
            params![msg_id],
        )?;
        if held > 0 {
            tx.execute(
                "INSERT INTO kept_status (msg_id, space, status, person, agent)
                 SELECT sent.msg_id, sent.space, json_set(own.held, '$.timestamp', ?2), 1,
                     sent.agent
                 FROM sent JOIN own_message AS own ON own.msg_id = sent.msg_id
                 WHERE sent.msg_id = ?1",
                params![msg_id, now],
            )?;
        }
        tx.execute(
            "UPDATE sent_status SET held = ?2
             WHERE msg_id = ?1 AND held IS NULL AND read IS NULL AND refused IS NULL
                 AND EXISTS (SELECT 1 FROM own_message WHERE msg_id = ?1 AND held IS NOT NULL)",
            params![msg_id, now],
        )?;
        let refused = tx.execute(
            "UPDATE sent SET read = 1 WHERE msg_id = ?1 AND read = 0
             AND EXISTS (SELECT 1 FROM own_message WHERE msg_id = ?1 AND refused IS NOT NULL)",
            params![msg_id],
        )?;
        if refused > 0 {
            tx.execute(
                "INSERT INTO kept_status (msg_id, space, status, person, agent)
                 SELECT sent.msg_id, sent.space, json_set(own.refused, '$.timestamp', ?2), 1,
                     sent.agent
                 FROM sent JOIN own_message AS own ON own.msg_id = sent.msg_id
                 WHERE sent.msg_id = ?1",
                params![msg_id, now],
            )?;
        }
        let refusal: Option<String> = tx
            .query_row(
                "SELECT refused FROM own_message WHERE msg_id = ?1 AND refused IS NOT NULL",
                params![msg_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(refusal) = refusal {
            let reason = Fields::of(&refusal);
            keep_answer(tx, &msg_id, Some(now), Some(reason.text("reason")))?;
        }
        tx.execute(
            "INSERT INTO kept_status (seq, msg_id, space, status, person, agent)
             SELECT seq, replied, space, replied_status, 1, replied_agent FROM own_message
             WHERE msg_id = ?1 AND replied IS NOT NULL ON CONFLICT DO NOTHING",
            params![msg_id],
        )?;
        tx.execute(
            "UPDATE sent_status SET replied = ?2
             WHERE msg_id = (SELECT replied FROM own_message WHERE msg_id = ?1)
                 AND replied IS NULL",
            params![msg_id, now],
        )?;
        tx.execute(
            "INSERT INTO own_read (seq, msg_id, agent, status)
             SELECT own.seq, own.msg_id, own.addressee, own.read FROM own_message AS own
             JOIN (SELECT seq, agent FROM handover UNION SELECT seq, agent FROM held_message)
                 AS waits ON waits.seq = own.seq AND waits.agent = own.addressee
             WHERE own.msg_id = ?1 AND own.read IS NOT NULL ON CONFLICT DO NOTHING",
            params![msg_id],
        )?;
    }
    tx.execute("DELETE FROM own_message WHERE msg_id = ?1", params![msg_id])?;
    drop_done(tx)
}

/// The fields of a status as JSON ([`crate::forms::Status`]); none of one that is not.
struct Fields(serde_json::Value);

impl Fields {
    fn of(status: &str) -> Self {
        Fields(serde_json::from_str(status).unwrap_or_default())
    }

    /// Its field `name`, if it is a string.
    fn text(&self, name: &str) -> Option<&str> {
        self.0[name].as_str()
    }

    /// Its `timestamp`, if it is a time.
    fn time(&self) -> Option<i64> {
        self.0["timestamp"].as_i64()
    }
}

/// Keeps, in `tx`, where the device's message of `tracked` goes, until the server stores it.
fn add_tracked(tx: &Transaction, tracked: &Tracked) -> Result<(), Error> {
    let msg_id = tracked.msg_id.to_string();
    tx.execute(
        "INSERT INTO sent_status (msg_id, wait) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        params![msg_id, sql_u64(tracked.wait)?],
    )?;
    for recipient in &tracked.recipients {
        tx.execute(
            "INSERT INTO sent_recipient (msg_id, device, copy, name, account, handle)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
            params![
                msg_id,
                recipient.device.to_string(),
                recipient.copy,
                recipient.name,
                recipient.account,
                recipient.handle
            ],
        )?;
    }
    Ok(())
}

/// Records, in `tx`, the server's answer to the envelope of the device's message `msg_id` to
/// `recipient`, none under `mls`: if `stored`, its recipient devices, or the copy's, are `stored`
/// now, and the message is stored now; a refused copy's device is no recipient.
fn track_answer(
    tx: &Transaction,
    msg_id: MessageId,
    recipient: Option<DeviceId>,
    stored: bool,
) -> Result<(), Error> {
    let (msg_id, now) = (msg_id.to_string(), sql_u64(unix_now())?);
    let device = recipient.map(|device| device.to_string());
    if !stored {
        tx.execute(
            "DELETE FROM sent_recipient WHERE msg_id = ?1 AND copy = 1 AND device = ?2",
            params![msg_id, device],
        )?;
        return Ok(());
    }
    if recipient.is_none() {
        tx.execute(
            "UPDATE sent_status SET stored = ?2 WHERE msg_id = ?1 AND stored IS NULL",
            params![msg_id, now],
        )?;
    }
    tx.execute(
        "UPDATE sent_recipient SET status = 'stored', timestamp = ?2
         WHERE msg_id = ?1 AND status IS NULL AND copy = (?3 IS NOT NULL)
             AND (?3 IS NULL OR device = ?3)",
        params![msg_id, now, device],
    )?;
    Ok(())
}

/// A row of the device's sent messages it keeps statuses of: the space's and channel's IDs, the
/// agent it is from, its `to` and the reason of its `refused`, and the times of its `stored`,
/// `held`, `read`, `replied` and `refused`.
type SentRow = ([String; 2], [Option<String>; 3], [Option<i64>; 5]);

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

/// Whether the state in `dir` has the schema of this client's last migration, read without
/// migrating it.
pub fn current(dir: &Path) -> bool {
    let latest = MIGRATIONS.len();
    let version = Connection::open(dir.join(FILE)).and_then(|db| {
        db.busy_timeout(std::time::Duration::from_secs(1))?;
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
    });
    version.is_ok_and(|version| usize::try_from(version) == Ok(latest))
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

    /// The value of the pragma `name` in `state`, its column `column`.
    fn pragma<T: rusqlite::types::FromSql>(state: &State, name: &str, column: usize) -> T {
        (state.db)
            .query_row(&format!("PRAGMA {name}"), [], |row| row.get(column))
            .unwrap()
    }

    /// One message is the line of a write to each of two sessions; each counts a wake in `c`.
    fn written_to_two_sessions() -> State {
        let state = State::in_memory().unwrap();
        let line = ["m".to_owned()];
        let channels = BTreeSet::from(["c".to_owned()]);
        state.socket_wakes("a", &line, &channels).unwrap();
        state.socket_wakes("b", &line, &channels).unwrap();
        assert_eq!(state.wakes().unwrap()["c"], 2);
        state
    }

    #[test]
    fn a_line_both_sessions_refuse_takes_back_both_writes_wakes() {
        let state = written_to_two_sessions();
        state.not_taken("a", "m").unwrap();
        state.not_taken("b", "m").unwrap();
        assert_eq!(state.wakes().unwrap()["c"], 0);
    }

    #[test]
    fn a_line_one_session_confirms_keeps_its_wake_and_the_other_s_refusal_takes_back_its_own() {
        let state = written_to_two_sessions();
        state.confirmed("a", &["m".to_owned()]).unwrap();
        state.not_taken("b", "m").unwrap();
        assert_eq!(state.wakes().unwrap()["c"], 1);
        state.not_taken("a", "m").unwrap();
        assert_eq!(state.wakes().unwrap()["c"], 1);
    }

    #[test]
    fn of_two_writes_to_one_session_a_refused_one_takes_back_its_wake_beside_a_confirmed_one() {
        let state = State::in_memory().unwrap();
        let channels = BTreeSet::from(["c".to_owned()]);
        (state.socket_wakes("a", &["m1".to_owned()], &channels)).unwrap();
        (state.socket_wakes("a", &["m2".to_owned()], &channels)).unwrap();
        state.confirmed("a", &["m1".to_owned()]).unwrap();
        state.not_taken("a", "m2").unwrap();
        assert_eq!(state.wakes().unwrap()["c"], 1);
    }

    #[test]
    fn the_migrations_are_the_files_of_migrations_in_the_order_of_their_names() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut files: Vec<PathBuf> = (fs::read_dir(dir).unwrap())
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        let texts: Vec<String> = files
            .iter()
            .map(|file| fs::read_to_string(file).unwrap())
            .collect();
        assert_eq!(MIGRATIONS, texts);
    }

    #[test]
    fn new_state_has_the_latest_schema_and_no_session() {
        let dir = TestDir::new("state-new");
        let state = State::open(&dir).expect("open");
        let version: u32 = pragma(&state, "user_version", 0);
        assert_eq!(version as usize, MIGRATIONS.len());
        assert_eq!(state.session().unwrap(), None);
    }

    #[test]
    fn state_in_memory_has_the_latest_schema_and_keeps_temporaries_in_memory() {
        let state = State::in_memory().expect("open");
        let version: u32 = pragma(&state, "user_version", 0);
        assert_eq!(version as usize, MIGRATIONS.len());
        let file: String = pragma(&state, "database_list", 2);
        assert_eq!(file, "", "an in-memory database has no file");
        let temp_store: u32 = pragma(&state, "temp_store", 0);
        assert_eq!(temp_store, 2, "temporary tables and indices stay in memory");
        state.start_session("http://one.test").unwrap();
        assert!(state.session().unwrap().is_some());
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
    fn anchors_object_replaces_only_the_one_the_run_read() {
        let dir = TestDir::new("state-anchors");
        let state = State::open(&dir).unwrap();
        assert_eq!(state.anchors().unwrap(), None);
        assert!(state.set_anchors(None, &[1]).unwrap());
        assert!(
            !state.set_anchors(None, &[2]).unwrap(),
            "another run's first"
        );
        assert_eq!(state.anchors().unwrap(), Some((vec![1], true)));
        state.anchors_taken(&[2]).unwrap();
        assert_eq!(state.anchors().unwrap(), Some((vec![1], true)), "not it");
        state.anchors_taken(&[1]).unwrap();
        assert_eq!(state.anchors().unwrap(), Some((vec![1], false)));
        assert!(!state.set_anchors(Some(&[2]), &[3]).unwrap(), "replaced");
        assert!(state.set_anchors(Some(&[1]), &[3]).unwrap());
        let state = State::open(&dir).unwrap();
        assert_eq!(state.anchors().unwrap(), Some((vec![3], true)));
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
                record: vec![4, 5],
                backup: vec![6],
            },
            Link::Certified {
                master,
                cert: cert.clone(),
            },
            Link::Waiting,
            Link::Unchecked {
                master,
                cert: cert.clone(),
                link: vec![4, 5],
            },
            Link::Granted {
                master,
                cert: cert.clone(),
                link: vec![4, 5],
            },
            Link::Waiting,
            Link::Recovering {
                master,
                cert: cert.clone(),
            },
            Link::Certified {
                master,
                cert: cert.clone(),
            },
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
        // Whether a recovery key waits is kept apart from the link.
        assert_eq!(state.recovery_waits().unwrap(), None);
        state.set_recovery_waits(Some("https://one.test")).unwrap();
        state.set_link(&Link::Waiting).unwrap();
        let kept = State::open(&dir).unwrap().recovery_waits().unwrap();
        assert_eq!(kept.as_deref(), Some("https://one.test"));
        state.set_recovery_waits(None).unwrap();
        assert_eq!(State::open(&dir).unwrap().recovery_waits().unwrap(), None);
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
            "UPDATE device SET recovery = x'01'",
            "UPDATE device SET master = zeroblob(32), cert = x'01'",
            "UPDATE device SET link = 'linked'",
        ] {
            assert!(state.db.execute(update, []).is_err(), "{update}");
        }
        // A sent master key needs its account record and master backup.
        for update in [
            "UPDATE device SET link = 'sent', master = zeroblob(32), cert = x'01'",
            "UPDATE device SET record = x'02'",
        ] {
            state.db.execute(update, []).unwrap();
            let error = state.device().expect_err("no record and backup");
            assert!(matches!(error, Error::Malformed(_)), "{error:?}");
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
                report: true,
            },
            channel,
            counter,
            agent: None,
            addressee: None,
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

        state
            .remove_outgoing(first.outgoing.msg_id, None, None)
            .unwrap();
        let state = State::open(&dir).unwrap();
        assert_eq!(
            state.outbox().unwrap(),
            [elsewhere.outgoing, second.outgoing]
        );
    }

    /// A message's history entry goes into the outbox with its envelope and waits for upload only
    /// once the server stored the envelope, in order, until it is taken out; a refused envelope
    /// takes its entry with it.
    #[test]
    fn history_entries_wait_for_their_stored_envelopes() {
        let dir = TestDir::new("state-history-outbox");
        let state = State::open(&dir).unwrap();
        let (channel, space) = (ChannelId::generate(), SpaceId::generate());
        let sent = |counter| {
            let msg_id = MessageId::generate();
            let message = Counted {
                outgoing: Outgoing {
                    msg_id,
                    recipient: None,
                    space,
                    envelope: vec![1, 2, 3],
                    report: true,
                },
                channel,
                counter,
                agent: None,
                addressee: None,
            };
            let entry = HistoryOutgoing {
                msg_id,
                space,
                channel,
                entry: vec![4, 5, u8::try_from(counter).unwrap()],
            };
            state
                .mls_change(|_| {
                    let changes = MlsChanges {
                        messages: vec![message],
                        entries: vec![entry.clone()],
                        ..MlsChanges::default()
                    };
                    Ok((None, changes, ()))
                })
                .unwrap();
            entry
        };
        let (first, second, refused) = (sent(1), sent(2), sent(3));
        assert_eq!(state.history_outbox().unwrap(), []);

        for entry in [&second, &first] {
            state.remove_outgoing(entry.msg_id, None, None).unwrap();
        }
        state
            .remove_outgoing(refused.msg_id, None, Some("refused"))
            .unwrap();

        let state = State::open(&dir).unwrap();
        assert_eq!(
            state.history_outbox().unwrap(),
            [first.clone(), second.clone()]
        );
        state.remove_history_entry(first.msg_id).unwrap();
        assert_eq!(state.history_outbox().unwrap(), [second]);
    }

    /// A copy to a `service` device takes the next counter the device keeps for it in the channel,
    /// apart from the channel's; a refused copy leaves the message's history entry.
    #[test]
    fn copies_count_per_recipient_and_a_refused_one_keeps_the_history_entry() {
        let (bot, other) = (
            DeviceId::from_signing_key(&[1; 32]),
            DeviceId::from_signing_key(&[2; 32]),
        );
        let dir = TestDir::new("state-copies");
        let state = State::open(&dir).unwrap();
        let (channel, space) = (ChannelId::generate(), SpaceId::generate());
        let msg_id = MessageId::generate();
        let message = |msg_id, recipient, counter| Counted {
            outgoing: Outgoing {
                msg_id,
                recipient,
                space,
                envelope: vec![1],
                report: true,
            },
            channel,
            counter,
            agent: None,
            addressee: None,
        };
        let entry = HistoryOutgoing {
            msg_id,
            space,
            channel,
            entry: vec![4],
        };
        state
            .mls_change(|_| {
                let changes = MlsChanges {
                    messages: vec![message(msg_id, None, 1), message(msg_id, Some(bot), 1)],
                    entries: vec![entry.clone()],
                    ..MlsChanges::default()
                };
                Ok((None, changes, ()))
            })
            .unwrap();
        let next = MessageId::generate();
        for wrong in [1, 3] {
            let failed = put(&state, &[&message(next, Some(bot), wrong)]);
            assert!(failed.is_err(), "{wrong}: {failed:?}");
        }
        let (to_bot, to_other) = (message(next, Some(bot), 2), message(next, Some(other), 1));
        put(&state, &[&to_bot, &to_other]).unwrap();

        assert_eq!(state.channel_counter(channel).unwrap(), 1);
        assert_eq!(state.copy_counter(channel, bot).unwrap(), 2);
        assert_eq!(state.copy_counter(channel, other).unwrap(), 1);
        assert_eq!(state.copy_counter(ChannelId::generate(), bot).unwrap(), 0);
        state
            .remove_outgoing(msg_id, Some(bot), Some("refused"))
            .unwrap();
        state.remove_outgoing(msg_id, None, None).unwrap();
        assert_eq!(state.history_outbox().unwrap(), [entry]);
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
            report: true,
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
        state.remove_outgoing(msg_id, None, None).unwrap();
        assert_eq!(state.outbox().unwrap().len(), 1, "taken out as an mls one");
        state.remove_outgoing(msg_id, Some(bob), None).unwrap();
        assert_eq!(state.outbox().unwrap(), []);
    }

    #[test]
    fn sent_messages_keep_their_space_channel_agent_and_addressee() {
        let dir = TestDir::new("state-sent");
        let state = State::open(&dir).unwrap();
        let (channel, space, msg_id) = (
            ChannelId::generate(),
            SpaceId::generate(),
            MessageId::generate(),
        );
        let to = "acme/bob/codex@skald.example";
        let message = Counted {
            outgoing: Outgoing {
                msg_id,
                recipient: None,
                space,
                envelope: vec![1],
                report: true,
            },
            channel,
            counter: 1,
            agent: Some("claude".to_owned()),
            addressee: Some(to.to_owned()),
        };
        // A receipt goes out as a message does, with `report` false, and is no sent message.
        let receipt = Counted {
            outgoing: Outgoing {
                msg_id: MessageId::generate(),
                envelope: vec![2],
                report: false,
                ..message.outgoing.clone()
            },
            counter: 2,
            agent: None,
            addressee: Some("acme/bob/user@skald.example".to_owned()),
            ..message.clone()
        };
        assert_eq!(state.sent(msg_id).unwrap(), None);

        put(&state, &[&message, &receipt]).unwrap();
        assert_eq!(
            state.outbox().unwrap(),
            [message.outgoing, receipt.outgoing.clone()]
        );
        state.remove_outgoing(msg_id, None, None).unwrap();

        let state = State::open(&dir).unwrap();
        let sent = Sent {
            space,
            channel,
            agent: Some("claude".to_owned()),
            addressee: Some(to.to_owned()),
        };
        assert_eq!(state.sent(msg_id).unwrap(), Some(sent));
        assert_eq!(state.sent(receipt.outgoing.msg_id).unwrap(), None);
        assert_eq!(state.sent(MessageId::generate()).unwrap(), None);
        assert_eq!(state.kept_ids().unwrap(), [msg_id]);
        assert_eq!(state.channel_counter(channel).unwrap(), 2);
    }

    /// The state in `dir` of a device that keeps, as entries 1, 2 and on, a message for `claude`
    /// with each of `hops`.
    fn keeping_hops(dir: &TestDir, hops: &[u64]) -> State {
        let state = State::open(dir).unwrap();
        state
            .add_device(DeviceId::from_signing_key(&[1; 32]), "laptop", "0190")
            .unwrap();
        let sender = DeviceId::from_signing_key(&[2; 32]);
        for (seq, &hops) in (1..).zip(hops) {
            state
                .write_entries(&[(seq, vec![u8::try_from(seq).unwrap()])])
                .unwrap();
            let mut kept = kept(&["claude"]);
            kept.hops = hops;
            state.keep(seq, (sender, kept.msg_id), &kept).unwrap();
        }
        state
    }

    /// A task waits in the outbox, in order and across a reopening, until it is taken out; one
    /// enqueued as an agent restarts the agent's count of `hops` in its space, one of the person's
    /// changes no count. Each stays the task of the agent it was enqueued as, in its space, after it
    /// leaves the outbox.
    #[test]
    fn tasks_wait_in_order_until_answered_and_restart_the_agent_s_hops() {
        let dir = TestDir::new("state-task-outbox");
        let state = keeping_hops(&dir, &[3]);
        let space = space_of_kept();
        state.handed_over(1, "claude").unwrap();
        let task = |byte| TaskOutgoing {
            task: TaskId::generate(),
            space,
            channel: ChannelId::generate(),
            queue: QueueId::generate(),
            record: vec![byte; 3],
        };
        let (first, second, third) = (task(1), task(2), task(3));
        assert_eq!(state.task_outbox().unwrap(), []);

        state.keep_task(&first, None).unwrap();
        assert_eq!(state.handed_hops(space, "claude").unwrap(), Some(3));
        state.keep_task(&second, Some("claude")).unwrap();
        assert_eq!(state.handed_hops(space, "claude").unwrap(), None);
        state.keep_task(&third, Some("codex")).unwrap();
        assert!(state.keep_task(&first, None).is_err(), "a task kept twice");
        state.remove_task(second.task).unwrap();

        let state = State::open(&dir).unwrap();
        let codex_s = third.task;
        assert_eq!(state.task_outbox().unwrap(), [first, third]);
        let own = |agent| state.enqueued_by(space, agent).unwrap();
        assert_eq!(own("claude"), BTreeSet::from([second.task]));
        assert_eq!(own("codex"), BTreeSet::from([codex_s]));
        assert_eq!(own("gemini"), BTreeSet::new());
        let elsewhere = state.enqueued_by(SpaceId::generate(), "claude").unwrap();
        assert_eq!(elsewhere, BTreeSet::new());
    }

    /// The highest `hops` handed over to an agent in a space counts until the device sends a
    /// message as that agent there; the person's messages and another agent's leave it, and a
    /// `hops` past what SQLite holds counts as the highest it holds.
    #[test]
    fn handed_hops_are_the_highest_handed_over_since_the_agent_s_last_message() {
        let dir = TestDir::new("state-hops");
        let state = keeping_hops(&dir, &[7, 3, u64::MAX]);
        let (space, other) = (space_of_kept(), SpaceId::generate());
        let channel = ChannelId::generate();
        let sends = |counter: u64, (space, agent): (SpaceId, Option<&str>)| {
            let message = Counted {
                outgoing: Outgoing {
                    msg_id: MessageId::generate(),
                    recipient: None,
                    space,
                    envelope: vec![1],
                    report: true,
                },
                channel,
                counter,
                agent: agent.map(str::to_owned),
                addressee: None,
            };
            put(&state, &[&message]).unwrap();
        };
        assert_eq!(state.handed_hops(space, "claude").unwrap(), None);

        state.handed_over(2, "claude").unwrap();
        assert_eq!(state.handed_hops(space, "claude").unwrap(), Some(3));
        state.handed_over(1, "claude").unwrap();
        assert_eq!(state.handed_hops(space, "claude").unwrap(), Some(7));
        assert_eq!(state.handed_hops(space, "codex").unwrap(), None);
        assert_eq!(state.handed_hops(other, "claude").unwrap(), None);
        sends(1, (space, None));
        sends(2, (space, Some("codex")));
        sends(3, (other, Some("claude")));
        assert_eq!(state.handed_hops(space, "claude").unwrap(), Some(7));
        sends(4, (space, Some("claude")));
        assert_eq!(state.handed_hops(space, "claude").unwrap(), None);
        state.handed_over(3, "claude").unwrap();
        let highest = i64::MAX as u64;
        assert_eq!(state.handed_hops(space, "claude").unwrap(), Some(highest));
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
            channel: ChannelId::generate(),
            msg_id: MessageId::generate(),
            line: b"line".to_vec(),
            target: b"target".to_vec(),
            wait: 86_400,
            hops: 0,
            agents: agents.iter().map(|&agent| agent.to_owned()).collect(),
            held: Vec::new(),
            read: None,
            replied: None,
        }
    }

    /// The space of [`kept`].
    fn space_of_kept() -> SpaceId {
        "0190b6a2-1f3c-7d4e-8a5b-6c7d8e9f0a1b".parse().unwrap()
    }

    /// A write while another connection, as a Codex hook's, holds the write lock waits for it and
    /// then succeeds.
    #[test]
    fn write_waits_for_another_connection_that_writes() {
        let dir = TestDir::new("state-write-waits");
        let state = State::open(&dir).unwrap();
        let other = State::open(&dir).unwrap();
        (state.add_device(DeviceId::from_signing_key(&[1; 32]), "laptop", "0190")).unwrap();
        other.db.execute_batch("BEGIN IMMEDIATE").unwrap();
        let session = CodexSession {
            thread: "thread".to_owned(),
            space: SpaceId::generate(),
            agent: "web".to_owned(),
            folder: PathBuf::from("/w"),
            file: None,
            home: None,
            process: None,
            busy: false,
            hooked: 0,
        };
        other.register_codex(&session).unwrap();
        let committed = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            other.db.execute_batch("COMMIT").unwrap();
        });

        assert_eq!(state.write_entries(&[(1, vec![1])]).unwrap(), 1);
        committed.join().unwrap();
    }

    /// The state keeps each agent's status: what it set, what the device last gave it and its
    /// runtime; a new session clears what it set. Each channel's presence counts its revisions
    /// on, and keeps the last value the server took.
    #[test]
    fn agent_statuses_and_presence_revisions_are_kept() {
        let dir = TestDir::new("state-presence");
        let state = State::open(&dir).unwrap();
        state
            .add_device(DeviceId::from_signing_key(&[1; 32]), "laptop", "0190")
            .unwrap();
        let space = SpaceId::generate();
        state.add_agent(space, "codex").unwrap();
        state.add_agent(space, "claude").unwrap();
        state
            .set_agent_status(space, "claude", true, Some("migrating"))
            .unwrap();
        state.agent_shown(space, "claude", (2, 1_700)).unwrap();
        let statuses = state.agent_statuses(space).unwrap();
        assert_eq!(
            statuses,
            [
                AgentRecord {
                    name: "claude".to_owned(),
                    busy: true,
                    note: Some("migrating".to_owned()),
                    shown: Some((2, 1_700)),
                    runtime: None,
                },
                AgentRecord {
                    name: "codex".to_owned(),
                    busy: false,
                    note: None,
                    shown: None,
                    runtime: None,
                },
            ]
        );
        state
            .agent_session_started(space, "claude", Some("claude-code"))
            .unwrap();
        let claude = &state.agent_statuses(space).unwrap()[0];
        assert_eq!((claude.busy, claude.note.as_deref()), (false, None));
        assert_eq!(claude.shown, Some((2, 1_700)));
        assert_eq!(claude.runtime.as_deref(), Some("claude-code"));
        assert!(
            state
                .agent_statuses(SpaceId::generate())
                .unwrap()
                .is_empty()
        );

        let channel = ChannelId::generate();
        assert_eq!(state.channel_presence(channel).unwrap(), None);
        assert_eq!(state.next_presence_revision(channel).unwrap(), 1);
        assert_eq!(state.next_presence_revision(channel).unwrap(), 2);
        let uploaded = Uploaded {
            epoch: 3,
            expires: 2_600,
            agents: "[]".to_owned(),
        };
        state.presence_uploaded(channel, &uploaded).unwrap();
        drop(state);
        let state = State::open(&dir).unwrap();
        assert_eq!(
            state.channel_presence(channel).unwrap(),
            Some(ChannelPresence {
                revision: 2,
                uploaded: Some(uploaded),
            })
        );
        assert_eq!(state.next_presence_revision(channel).unwrap(), 3);
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
        let long = kept(&["claude", "codex"]);
        state
            .keep(2, (sender, MessageId::generate()), &long)
            .unwrap();
        let mut short = kept(&["claude"]);
        short.wait = 10;
        state
            .keep(3, (sender, MessageId::generate()), &short)
            .unwrap();
        let mut ids = state.kept_ids().unwrap();
        ids.sort_unstable();
        let mut kept_ids = vec![long.msg_id, short.msg_id];
        kept_ids.sort_unstable();
        assert_eq!(ids, kept_ids);
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
        state.shown_to_person(2, false).unwrap();
        state.shown_to_person(3, false).unwrap();
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

    /// A message the person's run showed can be replied to by the person; one handed over to an
    /// agent, by that agent in its space only; each until its wait ends, after which it is dropped.
    /// A kept reply's `replied` is kept with it, for the person and the message's agent.
    #[test]
    fn shown_and_handed_over_messages_can_be_replied_to_until_their_wait_ends() {
        let dir = TestDir::new("state-targets");
        let state = State::open(&dir).unwrap();
        state
            .add_device(DeviceId::from_signing_key(&[3; 32]), "laptop", "0192")
            .unwrap();
        let space = space_of_kept();
        state.add_agent(space, "claude").unwrap();
        state.add_agent(space, "codex").unwrap();
        let entries: Vec<(u64, Vec<u8>)> = (1..=4).map(|seq: u8| (seq.into(), vec![seq])).collect();
        state.write_entries(&entries).unwrap();
        let sender = DeviceId::from_signing_key(&[4; 32]);
        let (shown, handed) = (kept(&[]), kept(&["claude", "codex"]));
        let mut short = kept(&["claude"]);
        short.wait = 10;
        let mut reply = kept(&[]);
        let original = MessageId::generate();
        reply.replied = Some(Replied {
            msg_id: original,
            space,
            status: "replied".to_owned(),
            agent: Some("codex".to_owned()),
        });
        for (seq, kept) in [(1, &shown), (2, &handed), (3, &short)] {
            state.keep(seq, (sender, kept.msg_id), kept).unwrap();
        }
        let now = unix_now();
        let seqs = |agent: Option<(SpaceId, &str)>, now: u64| -> Vec<(u64, MessageId)> {
            let targets = state.targets(agent, now).unwrap();
            targets
                .iter()
                .map(|target| (target.seq, target.msg_id))
                .collect()
        };
        assert!(seqs(None, now).is_empty(), "a message not shown yet");

        state.shown_to_person(1, false).unwrap();
        state.handed_over(2, "claude").unwrap();
        state.handed_over(3, "claude").unwrap();
        let target = &state.targets(None, now).unwrap()[0];
        assert_eq!(
            (target.space, target.channel, target.sealed.as_slice()),
            (space, shown.channel, b"target".as_slice())
        );
        assert_eq!(seqs(None, now), [(1, shown.msg_id)]);
        let claude = Some((space, "claude"));
        assert_eq!(seqs(claude, now), [(2, handed.msg_id), (3, short.msg_id)]);
        assert!(seqs(Some((space, "codex")), now).is_empty());
        assert!(seqs(Some((SpaceId::generate(), "claude")), now).is_empty());
        assert!(state.kept_ids().unwrap().contains(&shown.msg_id));
        // The short wait ends.
        assert_eq!(seqs(claude, now + 11), [(2, handed.msg_id)]);
        assert_eq!(seqs(claude, now), [(2, handed.msg_id)]);

        state.keep(4, (sender, reply.msg_id), &reply).unwrap();
        assert_eq!(statuses(state.statuses_for_person()), ["replied"]);
        assert_eq!(
            statuses(state.statuses_for_agent(space, "codex")),
            ["replied"]
        );
        assert!(statuses(state.statuses_for_agent(space, "claude")).is_empty());
    }

    /// The JSON of each of `statuses`.
    fn statuses(statuses: Result<Vec<(u64, String)>, Error>) -> Vec<String> {
        statuses
            .unwrap()
            .into_iter()
            .map(|(_, status)| status)
            .collect()
    }

    /// The state in `dir` of the device `laptop`, with an inbox entry of each of `seqs` that holds
    /// its `seq`.
    fn with_entries(dir: &TestDir, seqs: &[u8]) -> State {
        let state = State::open(dir).unwrap();
        state
            .add_device(DeviceId::from_signing_key(&[1; 32]), "laptop", "0190")
            .unwrap();
        let entries: Vec<(u64, Vec<u8>)> =
            (seqs.iter()).map(|&seq| (seq.into(), vec![seq])).collect();
        state.write_entries(&entries).unwrap();
        state
    }

    /// A kept message to an agent owes its `read` once handed over to that agent, kept with the
    /// mark, until a receipt that reports it goes into the outbox; one whose wait ends before owes
    /// none.
    #[test]
    fn read_is_owed_once_the_to_agent_has_the_message_until_its_receipt_is_queued() {
        let dir = TestDir::new("state-reads");
        let state = with_entries(&dir, &[2, 3]);
        let sender = DeviceId::from_signing_key(&[2; 32]);
        let channel = ChannelId::generate();
        let to_claude = |wait| {
            let mut kept = kept(&["claude"]);
            kept.wait = wait;
            kept.read = Some(ToRead {
                channel,
                sender,
                author: "acme/bob/codex@skald.example".to_owned(),
                addressee: "acme/ada/claude@skald.example".to_owned(),
                agent: "claude".to_owned(),
                reason: None,
                held: false,
            });
            kept
        };
        let (long, short) = (to_claude(86_400), to_claude(10));
        state.keep(2, (sender, long.msg_id), &long).unwrap();
        state.keep(3, (sender, short.msg_id), &short).unwrap();
        let read = ReadDue {
            seq: 2,
            msg_id: long.msg_id,
            space: space_of_kept(),
            channel,
            sender,
            author: "acme/bob/codex@skald.example".to_owned(),
            addressee: "acme/ada/claude@skald.example".to_owned(),
            status: ReceiptStatus::Read,
            reason: None,
        };

        assert!(state.reads_due().unwrap().is_empty());
        state.shown_to_person(2, false).unwrap();
        state.handed_over(2, "codex").unwrap();
        assert!(state.reads_due().unwrap().is_empty(), "not its agent");
        state.handed_over(2, "claude").unwrap();
        let state = State::open(&dir).unwrap();
        assert_eq!(state.reads_due().unwrap(), [read]);
        // The short wait ends before claude has it.
        state
            .for_agent(space_of_kept(), "claude", unix_now() + 11)
            .unwrap();
        state.handed_over(3, "claude").unwrap();
        assert_eq!(state.reads_due().unwrap().len(), 1);
        state
            .mls_change(|_| {
                let issued = MlsChanges {
                    issued: vec![2],
                    ..MlsChanges::default()
                };
                Ok((None, issued, ()))
            })
            .unwrap();
        assert!(state.reads_due().unwrap().is_empty());
        let owed: i64 = state
            .db
            .query_row("SELECT count(*) FROM read_receipt", [], |row| row.get(0))
            .unwrap();
        assert_eq!(owed, 0, "a read is owed of a message its agent never had");
    }

    /// A kept message whose `to` agent refuses it owes its `refused`, with the reason, at once,
    /// though no agent has it, until a receipt that reports it goes into the outbox.
    #[test]
    fn refused_is_owed_once_the_message_is_kept() {
        let dir = TestDir::new("state-refused");
        let state = with_entries(&dir, &[2]);
        let sender = DeviceId::from_signing_key(&[2; 32]);
        let mut refused = kept(&[]);
        refused.read = Some(ToRead {
            channel: refused.channel,
            sender,
            author: "acme/bob/codex@skald.example".to_owned(),
            addressee: "acme/ada/claude@skald.example".to_owned(),
            agent: "claude".to_owned(),
            reason: Some("off its allowlist".to_owned()),
            held: false,
        });

        state.keep(2, (sender, refused.msg_id), &refused).unwrap();
        state.shown_to_person(2, false).unwrap();

        let due = State::open(&dir).unwrap().reads_due().unwrap();
        assert_eq!(due.len(), 1, "{due:?}");
        assert_eq!(due[0].msg_id, refused.msg_id);
        assert_eq!(due[0].reason.as_deref(), Some("off its allowlist"));
        assert!(
            state
                .for_agent(space_of_kept(), "claude", unix_now())
                .unwrap()
                .is_empty()
        );
    }

    /// A kept message whose `to` agent holds it owes its `held` at once and waits for the person's
    /// decision, not the agent, until its wait ends: accepted, it waits for the agent, and handed
    /// over, its `hops` count for the agent's next send; refused, or undecided past its wait, it
    /// owes its `refused` with the reason. A decision counts once.
    #[test]
    fn held_message_waits_for_the_persons_decision() {
        let dir = TestDir::new("state-held");
        let state = with_entries(&dir, &[2, 3, 4]);
        let sender = DeviceId::from_signing_key(&[2; 32]);
        let held = |wait| {
            let mut held = held_for_claude(sender);
            held.wait = wait;
            held
        };
        let (mut accepted, refused, ended) = (held(86_400), held(86_400), held(10));
        accepted.hops = 6;
        state.keep(2, (sender, accepted.msg_id), &accepted).unwrap();
        state.keep(3, (sender, refused.msg_id), &refused).unwrap();
        state.keep(4, (sender, ended.msg_id), &ended).unwrap();
        state.shown_to_person(2, false).unwrap();
        state.shown_to_person(3, false).unwrap();
        state.shown_to_person(4, false).unwrap();
        let now = unix_now();

        let waiting = State::open(&dir).unwrap().held(now).unwrap();
        let waiting: Vec<_> = (waiting.iter())
            .map(|held| (held.seq, &*held.agent))
            .collect();
        assert_eq!(waiting, [(2, "claude"), (3, "claude"), (4, "claude")]);
        let due = state.reads_due().unwrap();
        assert!(
            (due.iter()).all(|due| due.status == ReceiptStatus::Held && due.reason.is_none()),
            "{due:?}"
        );
        assert_eq!(due.len(), 3);
        state
            .mls_change(|_| {
                let issued = MlsChanges {
                    issued_held: vec![2, 3, 4],
                    ..MlsChanges::default()
                };
                Ok((None, issued, ()))
            })
            .unwrap();
        assert!(state.reads_due().unwrap().is_empty());

        assert!(state.accept_held(2, "claude", now).unwrap());
        assert!(
            !state.accept_held(2, "claude", now).unwrap(),
            "decided once"
        );
        assert!(!state.refuse_held(2, "claude", None, now).unwrap());
        let handed: i64 = state
            .db
            .query_row("SELECT count(*) FROM handover WHERE seq = 2", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(handed, 1, "accepted, it waits for its agent");
        assert!(state.reads_due().unwrap().is_empty());
        assert_eq!(state.handed_hops(space_of_kept(), "claude").unwrap(), None);
        state.handed_over(2, "claude").unwrap();
        assert_eq!(
            state.handed_hops(space_of_kept(), "claude").unwrap(),
            Some(6)
        );
        let due = state.reads_due().unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!((due[0].seq, due[0].status), (2, ReceiptStatus::Read));

        assert!(state.refuse_held(3, "claude", Some("spam"), now).unwrap());
        assert!(
            !state.refuse_held(3, "claude", None, now).unwrap(),
            "decided once"
        );
        let due = State::open(&dir).unwrap().reads_due().unwrap();
        let refusal = due.iter().find(|due| due.seq == 3).unwrap();
        assert_eq!(refusal.status, ReceiptStatus::Refused);
        assert_eq!(refusal.reason.as_deref(), Some("spam"));

        let later = now + 11;
        assert!(
            !state.accept_held(4, "claude", later).unwrap(),
            "its wait ended"
        );
        state.end_held_waits(now, "no one").unwrap();
        assert_eq!(state.held(now).unwrap().len(), 1, "its wait goes on");
        state.end_held_waits(later, "no one").unwrap();
        assert!(state.held(now).unwrap().is_empty());
        let due = state.reads_due().unwrap();
        let ended = due.iter().find(|due| due.seq == 4).unwrap();
        assert_eq!(ended.status, ReceiptStatus::Refused);
        assert_eq!(ended.reason.as_deref(), Some("no one"));
    }

    /// What keeps a message from `sender` `to` claude, which holds it, as [`kept`] does.
    fn held_for_claude(sender: DeviceId) -> Kept {
        let mut held = kept(&[]);
        held.held = vec!["claude".to_owned()];
        held.read = Some(ToRead {
            channel: held.channel,
            sender,
            author: "acme/bob/codex@skald.example".to_owned(),
            addressee: "acme/ada/claude@skald.example".to_owned(),
            agent: "claude".to_owned(),
            reason: None,
            held: true,
        });
        held
    }

    /// A message held before its `held` goes owes the `held` first, then its `refused`.
    #[test]
    fn held_goes_before_the_refused_of_the_same_message() {
        let dir = TestDir::new("state-held-order");
        let state = with_entries(&dir, &[2]);
        let sender = DeviceId::from_signing_key(&[2; 32]);
        let held = held_for_claude(sender);
        state.keep(2, (sender, held.msg_id), &held).unwrap();
        assert!(state.refuse_held(2, "claude", None, unix_now()).unwrap());

        let due = State::open(&dir).unwrap().reads_due().unwrap();
        let statuses: Vec<_> = due
            .iter()
            .map(|due| (due.status, due.reason.clone()))
            .collect();
        assert_eq!(
            statuses,
            [(ReceiptStatus::Held, None), (ReceiptStatus::Refused, None)]
        );
    }

    /// The person sets an agent's mode and puts senders on its allowlist in a space; the list
    /// holds each agent the device has run as there or keeps a mode or a sender of, `accept`
    /// without a mode.
    #[test]
    fn inbound_modes_and_allowlists_are_kept_per_agent_and_space() {
        let dir = TestDir::new("state-inbound");
        let state = with_entries(&dir, &[]);
        let (space, other) = (space_of_kept(), SpaceId::generate());
        state.add_agent(space, "codex").unwrap();
        state.set_inbound_mode(space, "claude", "refuse").unwrap();
        state.set_inbound_mode(other, "claude", "accept").unwrap();
        assert!(state.allow_sender(space, "claude", "bob").unwrap());
        assert!(!state.allow_sender(space, "claude", "bob").unwrap());
        assert!(state.allow_sender(space, "claude", "ada/codex").unwrap());
        assert!(state.allow_sender(space, "api", "bob").unwrap());

        let inbound = |agent: &str, mode: &str, allowed: &[&str]| Inbound {
            agent: agent.to_owned(),
            mode: mode.to_owned(),
            allowed: allowed.iter().map(|&sender| sender.to_owned()).collect(),
        };
        assert_eq!(
            State::open(&dir).unwrap().inbound(space).unwrap(),
            [
                inbound("api", "accept", &["bob"]),
                inbound("claude", "refuse", &["ada/codex", "bob"]),
                inbound("codex", "accept", &[]),
            ]
        );
        state.set_inbound_mode(space, "claude", "accept").unwrap();
        assert_eq!(state.inbound(space).unwrap()[1].mode, "accept");
        assert_eq!(
            state.inbound(other).unwrap(),
            [inbound("claude", "accept", &[])]
        );
    }

    /// Puts in the outbox of `state` the device's own message in `channel` with `counter`, from
    /// `sender`, the person for none, `to` as [`OwnMessage::to`] says, with `replied` if given, by
    /// the next own `seq`; gives its `msg_id` and `seq`.
    fn put_own(
        state: &State,
        (channel, counter): (ChannelId, u64),
        (sender, to): (Option<&str>, Option<Option<&str>>),
        replied: Option<Replied>,
    ) -> (MessageId, u64) {
        put_own_refused(state, (channel, counter), (sender, to), replied, &[])
    }

    /// As [`put_own`], with `refusing` the agents whose inbound mode refuses it: its `refused`
    /// replaces its `read` if its `to` agent is one of them.
    fn put_own_refused(
        state: &State,
        (channel, counter): (ChannelId, u64),
        (sender, to): (Option<&str>, Option<Option<&str>>),
        replied: Option<Replied>,
        refusing: &[&str],
    ) -> (MessageId, u64) {
        let seq = state.next_own_seq().unwrap();
        let mut kept = kept(&[]);
        kept.channel = channel;
        kept.replied = replied;
        let message = Counted {
            outgoing: Outgoing {
                msg_id: kept.msg_id,
                recipient: None,
                space: kept.space,
                envelope: vec![1],
                report: true,
            },
            channel,
            counter,
            agent: sender.map(str::to_owned),
            addressee: None,
        };
        let own = OwnMessage {
            seq,
            kept: kept.clone(),
            person: sender.is_some(),
            sender: sender.map(str::to_owned),
            to: to.map(|to| to.map(str::to_owned)),
            read: matches!(to, Some(Some(to)) if !refusing.contains(&to))
                .then(|| r#"{"status":"read","timestamp":0}"#.to_owned()),
            refusing: refusing.iter().map(|&agent| agent.to_owned()).collect(),
            refused: matches!(to, Some(Some(to)) if refusing.contains(&to))
                .then(|| r#"{"status":"refused","reason":"off","timestamp":0}"#.to_owned()),
            holding: Vec::new(),
            held: None,
        };
        state
            .mls_change(|_| {
                let changes = MlsChanges {
                    messages: vec![message],
                    own: vec![own],
                    ..MlsChanges::default()
                };
                Ok((None, changes, ()))
            })
            .unwrap();
        (kept.msg_id, seq)
    }

    /// The device's own message is kept only with the server's `stored` of it, once, for the person
    /// if an agent sent it and for each agent of its space but its sender, with a `to` only the
    /// agent of this device it names; a refused one, for no one. Its `read` is kept for the person
    /// and its sender once the `to` agent has it, once, and a reply's `replied` with its `stored`.
    #[test]
    fn own_messages_are_kept_with_their_stored_for_whom_they_are_for() {
        let dir = TestDir::new("state-own");
        let state = with_entries(&dir, &[]);
        let space = space_of_kept();
        for agent in ["api", "web", "docs"] {
            state.add_agent(space, agent).unwrap();
        }
        state.add_agent(SpaceId::generate(), "elsewhere").unwrap();
        let channel = ChannelId::generate();
        let (to_web, to_web_seq) =
            put_own(&state, (channel, 1), (Some("api"), Some(Some("web"))), None);
        let (to_all, to_all_seq) = put_own(&state, (channel, 2), (None, None), None);
        let (to_bob, to_bob_seq) = put_own(&state, (channel, 3), (Some("api"), Some(None)), None);
        let (refused, refused_seq) = put_own(&state, (channel, 4), (Some("api"), None), None);
        let replied = Replied {
            msg_id: to_web,
            space,
            status: "replied".to_owned(),
            agent: Some("api".to_owned()),
        };
        let reply = put_own(
            &state,
            (channel, 5),
            (Some("web"), Some(Some("api"))),
            Some(replied),
        )
        .0;
        assert_eq!(to_web_seq, OWN_FIRST + 1);
        assert_eq!(refused_seq, to_web_seq + 3);
        let waiting = |agent| -> Vec<u64> {
            let kept = state.for_agent(space, agent, unix_now()).unwrap();
            kept.into_iter().map(|(seq, _)| seq).collect()
        };
        let person = || -> Vec<u64> {
            state
                .for_person()
                .unwrap()
                .into_iter()
                .map(|(seq, _)| seq)
                .collect()
        };
        assert!(
            person().is_empty() && waiting("web").is_empty(),
            "kept before stored"
        );

        for msg_id in [to_web, to_all, to_bob, to_web] {
            state.remove_outgoing(msg_id, None, None).unwrap();
        }
        state
            .remove_outgoing(refused, None, Some("refused"))
            .unwrap();

        assert_eq!(person(), [to_web_seq, to_bob_seq]);
        assert_eq!(waiting("web"), [to_web_seq, to_all_seq]);
        assert_eq!(waiting("api"), [to_all_seq]);
        assert_eq!(waiting("docs"), [to_all_seq]);
        assert!(
            state
                .for_agent(space, "elsewhere", unix_now())
                .unwrap()
                .is_empty()
        );
        let left: i64 = state
            .db
            .query_row("SELECT count(*) FROM own_message", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 1, "only the reply waits for the server");
        state.handed_over(to_web_seq, "docs").unwrap();
        assert!(
            statuses(state.statuses_for_person()).is_empty(),
            "read by another"
        );
        state.handed_over(to_web_seq, "web").unwrap();
        state.handed_over(to_web_seq, "web").unwrap();
        let read = statuses(state.statuses_for_person());
        let [read] = read.as_slice() else {
            panic!("not one read: {read:?}");
        };
        let read: serde_json::Value = serde_json::from_str(read).unwrap();
        assert_eq!(read["status"], "read");
        assert!(read["timestamp"].as_u64().unwrap() > 0, "{read}");
        let api = statuses(state.statuses_for_agent(space, "api"));
        assert_eq!(
            api.iter().filter(|it| it.contains(r#""read""#)).count(),
            1,
            "{api:?}"
        );

        state.remove_outgoing(reply, None, None).unwrap();
        let shown = statuses(state.statuses_for_person());
        assert_eq!(shown.last().map(String::as_str), Some("replied"));
        assert_eq!(
            waiting("api"),
            [to_all_seq, to_web_seq + 4],
            "the reply to api only"
        );
        assert_eq!(waiting("web"), [to_all_seq]);
    }

    /// Where a message of the device's own goes is kept once the server stores it: each group
    /// member `stored`, then as the server's statuses say, no earlier than `stored`, a copy's
    /// device once its copy is stored and none once refused, with the `to` agent's `read`; only
    /// for its sending agent in its space, or the person, until its wait ends. A refused message
    /// keeps none.
    #[test]
    fn sent_statuses_follow_the_server_and_the_to_agent() {
        let dir = TestDir::new("state-sent-status");
        let state = with_entries(&dir, &[1, 2, 3]);
        let (space, channel) = (SpaceId::generate(), ChannelId::generate());
        let devices = [1, 2, 3].map(|byte| DeviceId::from_signing_key(&[byte + 10; 32]));
        let recipient = |device: DeviceId, copy: bool| Recipient {
            device,
            copy,
            name: Some(format!("box {copy}")),
            account: Some("0190".to_owned()),
            handle: (!copy).then(|| "bob".to_owned()),
        };
        let send = |counter: u64| {
            let message = Counted {
                outgoing: Outgoing {
                    msg_id: MessageId::generate(),
                    recipient: None,
                    space,
                    envelope: vec![1],
                    report: true,
                },
                channel,
                counter,
                agent: Some("claude".to_owned()),
                addressee: Some("acme/bob/codex@skald.example".to_owned()),
            };
            let msg_id = message.outgoing.msg_id;
            let tracked = Tracked {
                msg_id,
                wait: 600,
                recipients: vec![
                    recipient(devices[0], false),
                    recipient(devices[1], true),
                    recipient(devices[2], true),
                ],
            };
            state
                .mls_change(|_| {
                    let changes = MlsChanges {
                        messages: vec![message],
                        tracked: vec![tracked],
                        ..MlsChanges::default()
                    };
                    Ok((None, changes, ()))
                })
                .unwrap();
            msg_id
        };
        let (sent, refused) = (send(1), send(2));
        let ids = |agent: Option<(SpaceId, &str)>, now: u64| state.sent_ids(agent, now).unwrap();
        assert!(ids(None, unix_now()).is_empty(), "before stored");
        assert_eq!(state.sent_status(sent).unwrap(), None);

        let before = unix_now();
        state.remove_outgoing(sent, None, None).unwrap();
        state.remove_outgoing(sent, Some(devices[1]), None).unwrap();
        state
            .remove_outgoing(sent, Some(devices[2]), Some("refused"))
            .unwrap();
        state
            .remove_outgoing(refused, None, Some("refused"))
            .unwrap();
        let status = |device: DeviceId, word: &str, at: u64| {
            format!(
                r#"{{"status":"{word}","msg_id":"{sent}","device":"{device}","device_name":"x","timestamp":{at}}}"#
            )
        };
        state
            .keep_status(
                1,
                (sent, Some(space), Some("claude")),
                &status(devices[0], "delivered", before + 1000),
            )
            .unwrap();
        state
            .keep_status(
                2,
                (sent, Some(space), Some("claude")),
                &status(devices[1], "expired", 7),
            )
            .unwrap();
        let source = (None, (devices[0], MessageId::generate()));
        state
            .take_receipt(3, source, (false, &[(sent, status(devices[0], "read", 9))]))
            .unwrap();

        let now = unix_now();
        assert_eq!(ids(None, now), [sent]);
        assert_eq!(ids(Some((space, "claude")), now), [sent]);
        assert!(ids(Some((space, "codex")), now).is_empty());
        assert!(ids(Some((SpaceId::generate(), "claude")), now).is_empty());
        let kept = state.sent_status(sent).unwrap().unwrap();
        assert!((before..=now).contains(&kept.stored), "{}", kept.stored);
        assert_eq!((kept.space, kept.channel), (space, channel));
        assert_eq!(kept.agent.as_deref(), Some("claude"));
        assert_eq!((kept.read, kept.replied), (Some(9), None));
        let recipients: Vec<_> = (kept.recipients.iter())
            .map(|it| (it.device, it.status.as_str(), it.handle.as_deref()))
            .collect();
        assert_eq!(
            recipients,
            [
                (devices[0], "delivered", Some("bob")),
                (devices[1], "expired", None)
            ]
        );
        assert_eq!(kept.recipients[0].timestamp, before + 1000);
        assert_eq!(
            kept.recipients[1].timestamp, kept.stored,
            "not before stored"
        );
        assert_eq!(state.sent_status(refused).unwrap(), None);

        assert!(ids(None, kept.stored + 600).is_empty(), "after its wait");
        assert_eq!(state.sent_status(sent).unwrap(), None);
    }

    /// Keeps, in `state`, where the device's message `msg_id` goes, with no recipient device.
    fn track(state: &State, msg_id: MessageId) {
        let tracked = Tracked {
            msg_id,
            wait: 600,
            recipients: Vec::new(),
        };
        state
            .mls_change(|_| {
                let changes = MlsChanges {
                    tracked: vec![tracked],
                    ..MlsChanges::default()
                };
                Ok((None, changes, ()))
            })
            .unwrap();
    }

    /// The `to` agent's `held` of a message of the device's own is kept from its receipt, and its
    /// `refused` with its reason, from its receipt or, on one device, as the server stores the
    /// message; the first of `read` and `refused` stays.
    #[test]
    fn refused_is_kept_with_its_reason_by_receipt_and_on_one_device() {
        let dir = TestDir::new("state-sent-refused");
        let state = with_entries(&dir, &[1, 2]);
        let space = space_of_kept();
        for agent in ["api", "web"] {
            state.add_agent(space, agent).unwrap();
        }
        let channel = ChannelId::generate();
        let afar = put_own(&state, (channel, 1), (Some("api"), None), None).0;
        let here = (Some("api"), Some(Some("web")));
        let here = put_own_refused(&state, (channel, 2), here, None, &["web"]).0;
        for msg_id in [afar, here] {
            track(&state, msg_id);
        }
        let before = unix_now();
        for msg_id in [afar, here] {
            state.remove_outgoing(msg_id, None, None).unwrap();
        }
        let sender = (
            None,
            (DeviceId::from_signing_key(&[2; 32]), MessageId::generate()),
        );
        let refused = r#"{"status":"refused","reason":"no","timestamp":9}"#.to_owned();
        let held = r#"{"status":"held","timestamp":8}"#.to_owned();
        state
            .take_receipt(1, sender, (true, &[(afar, held)]))
            .unwrap();
        let sender = (
            None,
            (DeviceId::from_signing_key(&[2; 32]), MessageId::generate()),
        );
        state
            .take_receipt(3, sender, (false, &[(afar, refused)]))
            .unwrap();
        let read = r#"{"status":"read","timestamp":10}"#.to_owned();
        let sender = (
            None,
            (DeviceId::from_signing_key(&[2; 32]), MessageId::generate()),
        );
        state
            .take_receipt(2, sender, (false, &[(afar, read)]))
            .unwrap();

        let afar = state.sent_status(afar).unwrap().unwrap();
        assert_eq!(
            (afar.held, afar.read, afar.refused),
            (Some(8), None, Some(9))
        );
        assert_eq!(afar.reason.as_deref(), Some("no"));
        let here = state.sent_status(here).unwrap().unwrap();
        assert_eq!(here.read, None);
        assert!(here.refused.is_some_and(|at| at >= before), "{here:?}");
        assert_eq!(here.reason.as_deref(), Some("off"));
    }

    /// On one device, the `to` agent's reply to a message of the device's own makes it `replied`,
    /// at the time the server stores the reply.
    #[test]
    fn reply_on_one_device_makes_the_message_replied() {
        let dir = TestDir::new("state-replied-here");
        let state = with_entries(&dir, &[]);
        let space = space_of_kept();
        for agent in ["api", "web"] {
            state.add_agent(space, agent).unwrap();
        }
        let channel = ChannelId::generate();
        let asked = put_own(&state, (channel, 1), (Some("api"), Some(Some("web"))), None).0;
        track(&state, asked);
        state.remove_outgoing(asked, None, None).unwrap();
        let replied = |state: &State| state.sent_status(asked).unwrap().unwrap().replied;
        assert_eq!(replied(&state), None, "replied before the reply");

        let reply = Replied {
            msg_id: asked,
            space,
            status: "replied".to_owned(),
            agent: Some("api".to_owned()),
        };
        let answer = (Some("web"), Some(Some("api")));
        let answer = put_own(&state, (channel, 2), answer, Some(reply)).0;
        assert_eq!(
            replied(&state),
            None,
            "replied before the server stored the reply"
        );
        let before = unix_now();
        state.remove_outgoing(answer, None, None).unwrap();

        let at = replied(&state).expect("not replied");
        assert!((before..=unix_now()).contains(&at), "{at}");
    }

    /// A receipt is taken out of the inbox with its counters, or as received without them, and
    /// keeps a `read` of each message it lists for the person and the agent that sent it, once per
    /// message.
    #[test]
    fn receipt_keeps_each_read_once() {
        let dir = TestDir::new("state-receipt");
        let state = with_entries(&dir, &[1, 2, 3]);
        let (space, channel) = (SpaceId::generate(), ChannelId::generate());
        let sent = |counter: u8| Counted {
            outgoing: Outgoing {
                msg_id: MessageId::generate(),
                recipient: None,
                space,
                envelope: vec![counter],
                report: true,
            },
            channel,
            counter: counter.into(),
            agent: Some("claude".to_owned()),
            addressee: Some("acme/bob/codex@skald.example".to_owned()),
        };
        let (first, second) = (sent(1), sent(2));
        put(&state, &[&first, &second]).unwrap();
        let (first, second) = (first.outgoing.msg_id, second.outgoing.msg_id);
        let sender = DeviceId::from_signing_key(&[2; 32]);
        let source = Source {
            channel,
            device: sender,
            scheme: Scheme::Mls,
        };
        let counters = Counters {
            highest: 1,
            missing: Vec::new(),
        };
        let read = |msg_id: MessageId| (msg_id, format!("read {msg_id}"));

        state
            .take_receipt(
                1,
                (Some((&source, &counters)), (sender, MessageId::generate())),
                (false, &[read(first)]),
            )
            .unwrap();
        state
            .take_receipt(
                2,
                (None, (sender, MessageId::generate())),
                (false, &[read(first), read(second)]),
            )
            .unwrap();

        let state = State::open(&dir).unwrap();
        assert_eq!(
            state
                .inbox()
                .unwrap()
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            [3]
        );
        assert_eq!(state.counters(&source, 3).unwrap(), Some(counters));
        let statuses: Vec<String> = (state.statuses_for_person().unwrap().into_iter())
            .map(|(_, status)| status)
            .collect();
        assert_eq!(statuses, [read(first).1, read(second).1]);
        let for_claude: Vec<String> = (state.statuses_for_agent(space, "claude").unwrap())
            .into_iter()
            .map(|(_, status)| status)
            .collect();
        assert_eq!(for_claude, statuses);
    }

    /// The highest presence revision of a device in a channel only rises, with the hash of its
    /// value; another device or channel has its own.
    #[test]
    fn presence_revisions_only_rise() {
        let dir = TestDir::new("state-presence");
        let state = State::open(&dir).unwrap();
        let (channel, other) = (ChannelId::generate(), ChannelId::generate());
        let device = DeviceId::from_signing_key(&[5; 32]);

        assert_eq!(state.presence_revision(channel, device).unwrap(), None);
        state
            .accept_presence((channel, device), 2, &[2; 32])
            .unwrap();
        state
            .accept_presence((channel, device), 1, &[1; 32])
            .unwrap();
        assert_eq!(
            state.presence_revision(channel, device).unwrap(),
            Some((2, [2; 32]))
        );
        state
            .accept_presence((channel, device), 3, &[3; 32])
            .unwrap();
        assert_eq!(
            state.presence_revision(channel, device).unwrap(),
            Some((3, [3; 32]))
        );
        assert_eq!(state.presence_revision(other, device).unwrap(), None);
    }

    /// A line kept for an agent's next result is given once, to that agent in that space, and a
    /// line kept twice before it is given once.
    #[test]
    fn notices_are_taken_once_by_their_agent() {
        let dir = TestDir::new("state-notices");
        let state = State::open(&dir).unwrap();
        let (space, other) = (SpaceId::generate(), SpaceId::generate());
        state.keep_notice(space, "api", "first").unwrap();
        state.keep_notice(space, "api", "second").unwrap();
        state.keep_notice(space, "api", "first").unwrap();
        state.keep_notice(other, "api", "elsewhere").unwrap();
        state.keep_notice(space, "docs", "another agent").unwrap();

        assert_eq!(
            state.take_notices(space, "api").unwrap(),
            ["first", "second"]
        );
        assert!(state.take_notices(space, "api").unwrap().is_empty());
        assert_eq!(state.take_notices(other, "api").unwrap(), ["elsewhere"]);
        assert_eq!(
            state.take_notices(space, "docs").unwrap(),
            ["another agent"]
        );
    }

    /// A kept status waits for the person's run, and for the run of the agent that sent its
    /// message in its space, until each takes it; then the state drops it. A refusal of an agent's
    /// message waits only for the agent, until it takes it or its own run reported it; one of the
    /// person's is not kept. A kept warning waits for the person's run, in order.
    #[test]
    fn kept_statuses_and_warnings_wait_until_taken() {
        let dir = TestDir::new("state-statuses");
        let state = State::open(&dir).unwrap();
        let this = DeviceId::from_signing_key(&[9; 32]);
        state.add_device(this, "desktop", "0191").unwrap();
        let entries: Vec<(u64, Vec<u8>)> = (4..=6).map(|seq: u8| (seq.into(), vec![seq])).collect();
        state.write_entries(&entries).unwrap();
        let (space, other) = (SpaceId::generate(), SpaceId::generate());
        let id = MessageId::generate;
        state
            .keep_status(4, (id(), Some(space), Some("claude")), "first")
            .unwrap();
        state
            .keep_status(5, (id(), Some(space), None), "second")
            .unwrap();
        state.keep_status(6, (id(), None, None), "third").unwrap();
        let channel = ChannelId::generate();
        let sent = |counter, agent: Option<&str>| {
            let message = Counted {
                outgoing: Outgoing {
                    msg_id: MessageId::generate(),
                    recipient: None,
                    space,
                    envelope: vec![counter],
                    report: true,
                },
                channel,
                counter: counter.into(),
                agent: agent.map(str::to_owned),
                addressee: None,
            };
            put(&state, &[&message]).unwrap();
            message.outgoing.msg_id
        };
        let (of_claude, of_person) = (sent(1, Some("claude")), sent(2, None));
        state
            .remove_outgoing(of_claude, None, Some("refused"))
            .unwrap();
        state
            .remove_outgoing(of_person, None, Some("refused too"))
            .unwrap();
        let rows = |rows: &[(u64, &str)]| -> Vec<(u64, String)> {
            rows.iter()
                .map(|&(position, status)| (position, status.to_owned()))
                .collect()
        };

        let state = State::open(&dir).unwrap();
        assert!(state.inbox().unwrap().is_empty());
        assert_eq!(
            state.statuses_for_person().unwrap(),
            rows(&[(1, "first"), (2, "second"), (3, "third")])
        );
        let claude = |space| state.statuses_for_agent(space, "claude").unwrap();
        assert_eq!(claude(space), rows(&[(1, "first"), (4, "refused")]));
        assert!(claude(other).is_empty());
        assert!(state.statuses_for_agent(space, "codex").unwrap().is_empty());
        state.status_shown_to_person(1).unwrap();
        state.status_shown_to_person(2).unwrap();
        assert_eq!(state.statuses_for_person().unwrap(), rows(&[(3, "third")]));
        assert_eq!(claude(space), rows(&[(1, "first"), (4, "refused")]));
        state.status_taken_by_agent(1).unwrap();
        assert_eq!(claude(space), rows(&[(4, "refused")]));
        state.refusals_taken("codex", of_claude).unwrap();
        assert_eq!(claude(space), rows(&[(4, "refused")]));
        state.refusals_taken("claude", of_claude).unwrap();
        assert!(claude(space).is_empty());
        let count: i64 = (state.db)
            .query_row("SELECT count(*) FROM kept_status", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "only the third waits");

        state
            .keep_warnings(&["a".to_owned(), "b".to_owned()])
            .unwrap();
        state
            .keep_warnings(&["c".to_owned(), "a".to_owned()])
            .unwrap();
        let warnings = state.warnings().unwrap();
        let lines: Vec<&str> = warnings.iter().map(|(_, line)| line.as_str()).collect();
        assert_eq!(
            lines,
            ["a", "b", "c"],
            "a line waiting already is kept once"
        );
        state.warning_shown(warnings[0].0).unwrap();
        state.keep_warnings(&["a".to_owned()]).unwrap();
        let lines: Vec<String> = (state.warnings().unwrap().into_iter())
            .map(|(_, line)| line)
            .collect();
        assert_eq!(lines, ["b", "c", "a"]);
    }

    /// The counters accepted from a sending device are kept per channel and scheme with the entry
    /// that took them out of the inbox. Under `mls`, a Welcome to the channel's group after that
    /// entry drops them for the entries after the Welcome; under `hpke`, it does not.
    #[test]
    fn counters_are_kept_per_source_until_the_device_joins_the_channel_again() {
        let dir = TestDir::new("state-counters");
        let state = with_entries(&dir, &[3, 4]);
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
            vec![(ada, StatusChange::Revoked), (bob, StatusChange::Valid)]
        );
        assert_eq!(
            state.device_statuses(&[(ada, true), (bob, false)]).unwrap(),
            vec![]
        );
        assert_eq!(
            state.device_statuses(&[(ada, false)]).unwrap(),
            vec![(ada, StatusChange::Valid)]
        );
    }

    /// The first certificate of a device is kept only while none is, later ones always, once
    /// each; a swap is warned of once, until no swapped certificate is given.
    #[test]
    fn kept_certificates_and_swaps_are_recorded() {
        let dir = TestDir::new("state-kept-certs");
        let state = State::open(&dir).unwrap();
        let ada = DeviceId::from_signing_key(&[1; 32]);

        state.keep_cert(ada, &[1], true).unwrap();
        state.keep_cert(ada, &[2], true).unwrap();
        state.keep_cert(ada, &[3], false).unwrap();
        state.keep_cert(ada, &[3], false).unwrap();
        assert_eq!(state.kept_certs(ada).unwrap(), vec![vec![1], vec![3]]);
        let other = DeviceId::from_signing_key(&[2; 32]);
        assert!(state.kept_certs(other).unwrap().is_empty());

        assert!(state.swapped_certs(ada, &[vec![4]]).unwrap());
        assert!(!state.swapped_certs(ada, &[vec![5], vec![4]]).unwrap());
        assert!(state.swapped_certs(ada, &[vec![5]]).unwrap());
        assert!(!state.swapped_certs(ada, &[]).unwrap());
        assert!(state.swapped_certs(ada, &[vec![5]]).unwrap());

        assert!(state.master_warning("a").unwrap());
        assert!(!state.master_warning("a").unwrap());
        assert!(state.master_warning("b").unwrap());
    }

    /// A valid device new to a list the device checked before is given as added; one new to a
    /// list it never checked, or revoked, is recorded only.
    #[test]
    fn a_valid_device_new_to_a_checked_list_is_added() {
        let dir = TestDir::new("state-device-added");
        let state = State::open(&dir).unwrap();
        let [ada, laptop, stolen, bob] =
            [1, 2, 3, 4].map(|key| DeviceId::from_signing_key(&[key; 32]));

        assert_eq!(state.device_statuses(&[(ada, false)]).unwrap(), vec![]);
        assert_eq!(
            state
                .device_statuses(&[(ada, false), (laptop, false), (stolen, true)])
                .unwrap(),
            vec![(laptop, StatusChange::Added)]
        );
        assert_eq!(state.device_statuses(&[(bob, false)]).unwrap(), vec![]);
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
    fn wraps_stay_settled_per_channel_epoch_and_recipient() {
        let dir = TestDir::new("state-wraps");
        let state = State::open(&dir).unwrap();
        let (one, other) = (ChannelId::generate(), ChannelId::generate());
        let browser = WrapRecipient::Browser(DeviceId::from_signing_key(&[3; 32]));
        let recovery = WrapRecipient::Recovery(skald_proto::AccountId::generate());

        state
            .settle_wraps(one, &[(2, browser), (1, browser), (2, recovery)])
            .unwrap();
        state.settle_wraps(one, &[(2, browser)]).unwrap();

        assert_eq!(
            state.settled_wraps(one).unwrap(),
            BTreeSet::from([(1, browser), (2, browser), (2, recovery)])
        );
        assert_eq!(state.settled_wraps(other).unwrap(), BTreeSet::new());
    }

    /// A claim is kept by its task, its time and sealed part updated in place, and raises the hops
    /// handed to its agent; it goes once removed, or with the step that puts its result in the
    /// outbox.
    #[test]
    fn claims_are_kept_by_task_and_end_with_their_result() {
        let dir = TestDir::new("state-claims");
        let state = State::open(&dir).unwrap();
        let space = SpaceId::generate();
        let claim = |since| TaskClaim {
            task: TaskId::generate(),
            space,
            channel: ChannelId::generate(),
            queue: QueueId::generate(),
            agent: Some("codex".to_owned()),
            since,
            sealed: vec![1],
        };
        let (first, second) = (claim(5), claim(6));
        state.keep_claim(&first, Some(("codex", 3))).unwrap();
        state.keep_claim(&second, None).unwrap();
        let first = TaskClaim {
            since: 9,
            sealed: vec![2],
            ..first
        };
        state.keep_claim(&first, Some(("codex", 1))).unwrap();
        assert_eq!(state.handed_hops(space, "codex").unwrap(), Some(3));
        let mut both = vec![first.clone(), second.clone()];
        both.sort_by_key(|claim| claim.task.to_string());
        assert_eq!(state.claims(Some(space)).unwrap(), both);
        assert_eq!(state.claims(Some(SpaceId::generate())).unwrap(), []);
        state.remove_claim(first.task).unwrap();
        state
            .mls_change(|_| {
                let changes = MlsChanges {
                    ended: vec![second.task],
                    ..MlsChanges::default()
                };
                Ok((None, changes, ()))
            })
            .unwrap();
        assert_eq!(state.claims(None).unwrap(), []);
    }

    #[test]
    fn given_dir_is_used() {
        let given = PathBuf::from("/some/where");
        assert_eq!(dir(Some(given.clone())).unwrap(), given);
    }
}
