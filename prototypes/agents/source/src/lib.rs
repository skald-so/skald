//! A local message source that stands in for the helper: a test or a person adds a message, a
//! delivery path takes the waiting messages and marks each handed over after the handover, as
//! `docs/protocol/delivery.md`, "Device to agent", says.
//!
//! The source is a directory that several processes share:
//! - `waiting/<msg_id>.json`: messages not yet taken;
//! - `taken/<claim>/<msg_id>.json`: messages a path took and is handing over;
//! - `claims/<claim>.lock`: locked by the [`Source`] that owns the claim while it lives;
//! - `handed/<msg_id>.json`: messages handed over, with the time of the handover;
//! - `records/<msg_id>/<signal>.json`: what is known of a message after its handover, each signal
//!   once: a read confirmation or a vendor's signal ([`Record`]);
//! - `session.json`: the session the source's messages go to, as its `SessionStart` hook told it
//!   ([`Registration`]), readable only by the user: it may hold the session's messaging token;
//! - `recovery.lock`: locked while a handle recovers abandoned claims or removes its own.
//!
//! Moving a file is atomic, so a message is taken by one path only. A path that dies between taking
//! a message and marking it leaves its claim unlocked; the next [`Source::take`] in any process puts
//! the message back, so a crash repeats the handover and never loses it.

use std::fs::{self, File, TryLockError};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A message for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// The message's ID, a UUIDv7: the order of IDs is the order messages were added in.
    pub msg_id: String,
    /// Who sent the message, as the agent should see it.
    pub sender: String,
    pub text: String,
    /// Milliseconds since the Unix epoch.
    pub added_at_ms: u64,
    /// When a path marked the message handed over, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handed_over_at_ms: Option<u64>,
    /// A secret the message carries to the agent, which only a session that saw the message can
    /// give back to acknowledge it ([`Source::check_ack`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack: Option<String>,
}

/// What is known of a handed-over message beyond its handover, such as a read confirmation
/// (`transcript`, `ack`) or a vendor's signal (`socket-held`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub signal: String,
    /// When it was recorded, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// What else it says, such as a reply's text or a receipt's reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A UUIDv7 that orders the records of one millisecond.
    #[serde(default)]
    pub seq: String,
}

/// The session a source's messages go to, as its `SessionStart` hook tells it.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    /// Claude Code's session ID or Codex's thread.
    pub session_id: String,
    /// The session's own file, if the CLI keeps one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Claude Code's inbox socket of the session, `CLAUDE_CODE_MESSAGING_SOCKET`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    /// The session's token for that socket, `CLAUDE_CODE_MESSAGING_TOKEN`: a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// When the hook registered it, in milliseconds since the Unix epoch.
    pub at_ms: u64,
}

/// Why [`Source::check_ack`] refused an acknowledgement, or that it accepted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ack {
    Accepted,
    /// No message has the ID, or the ID is not one the source makes.
    Unknown,
    /// The message waits or is being handed over: no session has seen it.
    NotHandedOver,
    /// The acknowledgement carries no secret.
    Missing,
    /// The secret is not the message's.
    Wrong,
}

impl Ack {
    /// Why the acknowledgement was refused, for the agent; `None` if it was accepted.
    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Accepted => None,
            Self::Unknown => Some("no such message"),
            Self::NotHandedOver => Some("the message was not handed over"),
            Self::Missing => Some("no ack given"),
            Self::Wrong => Some("the ack is not the message's"),
        }
    }
}
/// Where a message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Waiting,
    /// A path took it and has not marked it handed over yet.
    Taken,
    /// Handed over at this time, in milliseconds since the Unix epoch.
    HandedOver {
        at_ms: u64,
    },
}

/// One process's handle to the source directory.
pub struct Source {
    dir: PathBuf,
    claim: String,
    /// Locked while this handle lives: its claim's messages are being handed over.
    _lock: File,
}

const WAITING: &str = "waiting";
const TAKEN: &str = "taken";
const CLAIMS: &str = "claims";
const HANDED: &str = "handed";
const RECORDS: &str = "records";
const TMP: &str = "tmp";
const RECOVERY: &str = "recovery.lock";
const SESSION: &str = "session.json";

impl Source {
    /// Opens the source in `dir`, creating it if needed.
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        for sub in [WAITING, TAKEN, CLAIMS, HANDED, RECORDS, TMP] {
            fs::create_dir_all(dir.join(sub))?;
        }
        let claim = Uuid::now_v7().simple().to_string();
        fs::create_dir(dir.join(TAKEN).join(&claim))?;
        // The lock file gets its name only once locked: a claim whose lock is free is abandoned.
        let tmp = dir.join(TMP).join(format!("{claim}.lock"));
        let lock = File::create(&tmp)?;
        lock.lock()?;
        fs::rename(&tmp, dir.join(CLAIMS).join(format!("{claim}.lock")))?;
        Ok(Self {
            dir,
            claim,
            _lock: lock,
        })
    }

    /// The source's directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Adds a message from `sender` and returns it.
    pub fn add(&self, sender: &str, text: &str) -> io::Result<Message> {
        self.add_message(sender, text, None)
    }

    /// Adds a message from `sender` with a fresh random secret for its acknowledgement, and
    /// returns it.
    pub fn add_with_ack(&self, sender: &str, text: &str) -> io::Result<Message> {
        self.add_message(sender, text, Some(Uuid::new_v4().simple().to_string()))
    }

    fn add_message(&self, sender: &str, text: &str, ack: Option<String>) -> io::Result<Message> {
        let message = Message {
            msg_id: Uuid::now_v7().to_string(),
            sender: sender.to_owned(),
            text: text.to_owned(),
            added_at_ms: now_ms(),
            handed_over_at_ms: None,
            ack,
        };
        self.write(&self.dir.join(WAITING), &message)?;
        Ok(message)
    }

    /// Takes every waiting message, oldest first. Each is taken by one caller only; it stays taken
    /// until [`Source::handed_over`] or [`Source::release`]. First puts back the messages of
    /// abandoned claims.
    pub fn take(&self) -> io::Result<Vec<Message>> {
        self.recover()?;
        let mut taken = Vec::new();
        for name in sorted_names(&self.dir.join(WAITING))? {
            let to = self.claimed(&name);
            match fs::rename(self.dir.join(WAITING).join(&name), &to) {
                Ok(()) => taken.push(read(&to)?),
                // Another path took it first.
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(taken)
    }

    /// Marks messages this handle took as handed over.
    pub fn handed_over(&self, msg_ids: &[String]) -> io::Result<()> {
        let at_ms = now_ms();
        for msg_id in msg_ids {
            let from = self.claimed(&file_name(msg_id));
            let mut message = read(&from)?;
            message.handed_over_at_ms = Some(at_ms);
            self.write(&self.dir.join(HANDED), &message)?;
            fs::remove_file(from)?;
        }
        Ok(())
    }

    /// Puts messages this handle took back to waiting: their handover failed.
    pub fn release(&self, msg_ids: &[String]) -> io::Result<()> {
        for msg_id in msg_ids {
            let name = file_name(msg_id);
            fs::rename(self.claimed(&name), self.dir.join(WAITING).join(name))?;
        }
        Ok(())
    }

    /// Takes the waiting messages and hands them over with `hand`: marks them handed over once it
    /// succeeds, or puts them back and returns its error. With none waiting, calls nothing. Returns
    /// the messages handed over.
    pub fn hand_over(
        &self,
        hand: impl FnOnce(&[Message]) -> io::Result<()>,
    ) -> io::Result<Vec<Message>> {
        let messages = self.take()?;
        if messages.is_empty() {
            return Ok(messages);
        }
        let ids: Vec<String> = messages.iter().map(|m| m.msg_id.clone()).collect();
        if let Err(error) = hand(&messages) {
            self.release(&ids)?;
            return Err(error);
        }
        self.handed_over(&ids)?;
        Ok(messages)
    }

    /// Where the message is, or `None` if the source does not have it.
    pub fn state(&self, msg_id: &str) -> io::Result<Option<State>> {
        let name = file_name(msg_id);
        let handed = self.dir.join(HANDED).join(&name);
        if handed.exists() {
            let at_ms = read(&handed)?.handed_over_at_ms.unwrap_or_default();
            return Ok(Some(State::HandedOver { at_ms }));
        }
        if self.dir.join(WAITING).join(&name).exists() {
            return Ok(Some(State::Waiting));
        }
        for claim in fs::read_dir(self.dir.join(TAKEN))? {
            if claim?.path().join(&name).exists() {
                return Ok(Some(State::Taken));
            }
        }
        Ok(None)
    }

    /// Records the session the source's messages go to, replacing an earlier one, in a file only the
    /// user can read.
    pub fn register(&self, registration: &Registration) -> io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let tmp = self.dir.join(TMP).join(Uuid::now_v7().simple().to_string());
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        io::Write::write_all(&mut file, &serde_json::to_vec(registration)?)?;
        fs::rename(tmp, self.dir.join(SESSION))
    }

    /// The session the source's messages go to, if a hook registered one.
    pub fn registration(&self) -> io::Result<Option<Registration>> {
        match fs::read(self.dir.join(SESSION)) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// How many messages wait.
    pub fn waiting_count(&self) -> io::Result<usize> {
        Ok(sorted_names(&self.dir.join(WAITING))?.len())
    }

    /// The handed-over message `msg_id`, with the time of its handover.
    pub fn handed(&self, msg_id: &str) -> io::Result<Message> {
        read(&self.dir.join(HANDED).join(file_name(msg_id)))
    }

    /// Every handed-over message, oldest first.
    pub fn handed_messages(&self) -> io::Result<Vec<Message>> {
        sorted_names(&self.dir.join(HANDED))?
            .iter()
            .map(|name| read(&self.dir.join(HANDED).join(name)))
            .collect()
    }

    /// Puts handed-over messages back to waiting: the handover turned out to fail after it was
    /// marked, such as a prompt the CLI dropped from its queue.
    pub fn requeue(&self, msg_ids: &[String]) -> io::Result<()> {
        for msg_id in msg_ids {
            let from = self.dir.join(HANDED).join(file_name(msg_id));
            let message = read(&from)?;
            let waiting = Message {
                handed_over_at_ms: None,
                ..message
            };
            self.write(&self.dir.join(WAITING), &waiting)?;
            fs::remove_file(from)?;
        }
        Ok(())
    }

    /// Adds the handed-over message `msg_id`, with its ID, sender, text and secret, to `to` as a
    /// waiting message: the next path sends it again. Returns the message as `to` has it.
    pub fn resend(&self, msg_id: &str, to: &Source) -> io::Result<Message> {
        let message = Message {
            handed_over_at_ms: None,
            ..self.handed(msg_id)?
        };
        to.write(&to.dir.join(WAITING), &message)?;
        Ok(message)
    }

    /// Records `signal` for the message `msg_id`, with `detail` if given, unless it is recorded
    /// already; returns whether it was new. `signal` is lowercase letters, digits and `-`.
    pub fn record(&self, msg_id: &str, signal: &str, detail: Option<&str>) -> io::Result<bool> {
        if !signal
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || signal.is_empty()
        {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("{signal:?} is not a signal name"),
            ));
        }
        let dir = self.dir.join(RECORDS).join(checked_id(msg_id)?);
        fs::create_dir_all(&dir)?;
        let seq = Uuid::now_v7().simple().to_string();
        let record = Record {
            signal: signal.to_owned(),
            at_ms: now_ms(),
            detail: detail.map(String::from),
            seq: seq.clone(),
        };
        let tmp = self.dir.join(TMP).join(seq);
        fs::write(&tmp, serde_json::to_vec(&record)?)?;
        // A link fails if the record exists: the first one stays.
        let linked = fs::hard_link(&tmp, dir.join(format!("{signal}.json")));
        fs::remove_file(&tmp)?;
        match linked {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// The records of the message `msg_id`, in the order they were recorded.
    pub fn records(&self, msg_id: &str) -> io::Result<Vec<Record>> {
        let dir = self.dir.join(RECORDS).join(checked_id(msg_id)?);
        let Ok(names) = sorted_names(&dir) else {
            return Ok(Vec::new());
        };
        let mut records = names
            .iter()
            .map(|name| Ok(serde_json::from_slice(&fs::read(dir.join(name))?)?))
            .collect::<io::Result<Vec<Record>>>()?;
        records.sort_by(|a, b| (a.at_ms, &a.seq).cmp(&(b.at_ms, &b.seq)));
        Ok(records)
    }

    /// Checks an acknowledgement of the message `msg_id` with the secret `ack`, as an agent gives
    /// it: accepted only for a handed-over message whose secret it is. `msg_id` may be anything an
    /// agent sends.
    pub fn check_ack(&self, msg_id: &str, ack: Option<&str>) -> io::Result<Ack> {
        let Ok(msg_id) = checked_id(msg_id) else {
            return Ok(Ack::Unknown);
        };
        let message = match self.state(msg_id)? {
            None => return Ok(Ack::Unknown),
            Some(State::Waiting | State::Taken) => return Ok(Ack::NotHandedOver),
            Some(State::HandedOver { .. }) => self.handed(msg_id)?,
        };
        Ok(match (ack, message.ack) {
            (None, _) => Ack::Missing,
            (Some(given), Some(secret)) if same(given.as_bytes(), secret.as_bytes()) => {
                Ack::Accepted
            }
            (Some(_), _) => Ack::Wrong,
        })
    }

    fn claimed(&self, name: &str) -> PathBuf {
        self.dir.join(TAKEN).join(&self.claim).join(name)
    }

    /// Puts the messages of every claim whose owner is gone back to waiting.
    fn recover(&self) -> io::Result<()> {
        let _recovery = self.recovery()?;
        for name in sorted_names(&self.dir.join(CLAIMS))? {
            let Some(claim) = name.strip_suffix(".lock") else {
                continue;
            };
            if claim == self.claim {
                continue;
            }
            let lock = self.dir.join(CLAIMS).join(&name);
            match File::open(&lock)?.try_lock() {
                Ok(()) => {}
                // Its owner lives.
                Err(TryLockError::WouldBlock) => continue,
                Err(TryLockError::Error(error)) => return Err(error),
            }
            let taken = self.dir.join(TAKEN).join(claim);
            for message in sorted_names(&taken)? {
                fs::rename(taken.join(&message), self.dir.join(WAITING).join(&message))?;
            }
            fs::remove_dir(&taken)?;
            fs::remove_file(&lock)?;
        }
        Ok(())
    }

    /// Locks the source's recovery: one handle at a time recovers claims or removes its own.
    fn recovery(&self) -> io::Result<File> {
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(RECOVERY))?;
        lock.lock()?;
        Ok(lock)
    }

    /// Writes `message` into `dir` in one step: readers see the whole file or none.
    fn write(&self, dir: &Path, message: &Message) -> io::Result<()> {
        let tmp = self.dir.join(TMP).join(Uuid::now_v7().simple().to_string());
        fs::write(&tmp, serde_json::to_vec(message)?)?;
        fs::rename(tmp, dir.join(file_name(&message.msg_id)))
    }
}

impl Drop for Source {
    /// Removes the claim if it holds no message; otherwise leaves it for [`Source::take`] to recover.
    fn drop(&mut self) {
        let Ok(_recovery) = self.recovery() else {
            return;
        };
        if fs::remove_dir(self.dir.join(TAKEN).join(&self.claim)).is_ok() {
            let _ = fs::remove_file(self.dir.join(CLAIMS).join(format!("{}.lock", self.claim)));
        }
    }
}

fn file_name(msg_id: &str) -> String {
    format!("{msg_id}.json")
}

/// `msg_id` if it can be one the source makes: letters, digits and `-`, so it names no other path.
fn checked_id(msg_id: &str) -> io::Result<&str> {
    if !msg_id.is_empty()
        && msg_id.len() <= 64
        && msg_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        Ok(msg_id)
    } else {
        Err(io::Error::new(
            ErrorKind::InvalidInput,
            "not a message ID".to_owned(),
        ))
    }
}

/// Whether `a` and `b` are equal, in a time that does not depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn read(path: &Path) -> io::Result<Message> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn sorted_names(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = fs::read_dir(dir)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970");
    u64::try_from(since_epoch.as_millis()).expect("milliseconds fit in u64")
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;

    fn ids(messages: &[Message]) -> Vec<String> {
        messages.iter().map(|m| m.msg_id.clone()).collect()
    }

    fn since_epoch_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .try_into()
            .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn failing_take_is_an_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let message = source.add("alice", "one").unwrap();
        let claim = dir.path().join(TAKEN).join(&source.claim);
        fs::set_permissions(&claim, fs::Permissions::from_mode(0o555)).unwrap();
        assert!(source.take().is_err());
        fs::set_permissions(&claim, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(source.state(&message.msg_id).unwrap(), Some(State::Waiting));
    }

    #[test]
    fn takes_waiting_messages_once_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let before = since_epoch_ms();
        let first = source.add("alice", "one").unwrap();
        assert!((before..=since_epoch_ms()).contains(&first.added_at_ms));
        let second = source.add("bob", "two").unwrap();
        assert_eq!(source.state(&first.msg_id).unwrap(), Some(State::Waiting));

        let taken = source.take().unwrap();
        assert_eq!(taken, vec![first.clone(), second.clone()]);
        assert_eq!(source.state(&first.msg_id).unwrap(), Some(State::Taken));
        assert!(source.take().unwrap().is_empty());

        let before = since_epoch_ms();
        source.handed_over(&ids(&taken)).unwrap();
        let handed = source.handed(&first.msg_id).unwrap();
        let at_ms = handed.handed_over_at_ms.unwrap();
        assert!((before..=since_epoch_ms()).contains(&at_ms));
        assert_eq!(
            source.state(&first.msg_id).unwrap(),
            Some(State::HandedOver { at_ms })
        );
        assert_eq!(
            Message {
                handed_over_at_ms: None,
                ..handed
            },
            first
        );
        assert!(source.take().unwrap().is_empty());

        let third = source.add("carol", "three").unwrap();
        assert_eq!(source.take().unwrap(), vec![third]);
    }

    #[test]
    fn released_message_is_taken_again() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let message = source.add("alice", "one").unwrap();
        let taken = source.take().unwrap();
        source.release(&ids(&taken)).unwrap();
        assert_eq!(source.state(&message.msg_id).unwrap(), Some(State::Waiting));
        assert_eq!(source.take().unwrap(), vec![message]);
    }

    #[test]
    fn hands_over_the_waiting_messages_or_puts_them_back() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let mut calls = 0;
        let none = source.hand_over(|_| {
            calls += 1;
            Ok(())
        });
        assert!(none.unwrap().is_empty());
        assert_eq!(calls, 0, "nothing to hand over");

        let added = [
            source.add("alice", "one").unwrap(),
            source.add("bob", "two").unwrap(),
        ];
        let error = source
            .hand_over(|messages| {
                assert_eq!(messages, added);
                Err(io::Error::other("refused"))
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "refused");
        for message in &added {
            assert_eq!(source.state(&message.msg_id).unwrap(), Some(State::Waiting));
        }

        let handed = source
            .hand_over(|messages| {
                assert_eq!(messages, added);
                // Not handed over before the handover succeeds.
                assert_eq!(
                    source.state(&messages[0].msg_id).unwrap(),
                    Some(State::Taken)
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(handed, added);
        for message in &added {
            assert!(matches!(
                source.state(&message.msg_id).unwrap(),
                Some(State::HandedOver { .. })
            ));
        }
        assert!(source.hand_over(|_| unreachable!()).unwrap().is_empty());
    }

    /// A source in a temporary directory.
    fn opened() -> (tempfile::TempDir, Source) {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        (dir, source)
    }

    #[test]
    fn a_message_with_an_ack_carries_a_fresh_secret() {
        let (_dir, source) = opened();
        let plain = source.add("alice", "one").unwrap();
        assert_eq!(plain.ack, None);
        let first = source.add_with_ack("alice", "two").unwrap();
        let second = source.add_with_ack("alice", "three").unwrap();
        let secret = first.ack.clone().unwrap();
        assert_eq!(secret.len(), 32, "{secret}");
        assert!(secret.bytes().all(|b| b.is_ascii_hexdigit()), "{secret}");
        assert_ne!(first.ack, second.ack);
        assert_eq!(source.take().unwrap(), vec![plain, first, second]);
    }

    #[test]
    fn accepts_only_the_secret_of_a_handed_over_message() {
        let (_dir, source) = opened();
        let message = source.add_with_ack("alice", "one").unwrap();
        let secret = message.ack.clone().unwrap();
        let id = message.msg_id.as_str();
        assert_eq!(
            source.check_ack(id, Some(&secret)).unwrap(),
            Ack::NotHandedOver
        );
        let taken = source.take().unwrap();
        assert_eq!(
            source.check_ack(id, Some(&secret)).unwrap(),
            Ack::NotHandedOver
        );
        source.handed_over(&ids(&taken)).unwrap();
        assert_eq!(source.check_ack(id, Some(&secret)).unwrap(), Ack::Accepted);
        assert_eq!(source.check_ack(id, None).unwrap(), Ack::Missing);
        assert_eq!(source.check_ack(id, Some("")).unwrap(), Ack::Wrong);
        let mut other = secret.clone().into_bytes();
        other[31] = if other[31] == b'0' { b'1' } else { b'0' };
        let other = String::from_utf8(other).unwrap();
        assert_eq!(source.check_ack(id, Some(&other)).unwrap(), Ack::Wrong);
        assert_eq!(
            source.check_ack(id, Some(&secret[..31])).unwrap(),
            Ack::Wrong
        );
        // A message without a secret takes none.
        let plain = source.add("bob", "two").unwrap();
        source.handed_over(&ids(&source.take().unwrap())).unwrap();
        assert_eq!(
            source.check_ack(&plain.msg_id, Some(&secret)).unwrap(),
            Ack::Wrong
        );
        // An ID the source does not make names no file.
        for id in ["nope", "../handed/x", "", &"a".repeat(65)] {
            assert_eq!(source.check_ack(id, Some(&secret)).unwrap(), Ack::Unknown);
        }
        assert_eq!(Ack::Accepted.refusal(), None);
        for refused in [Ack::Unknown, Ack::NotHandedOver, Ack::Missing, Ack::Wrong] {
            assert!(refused.refusal().is_some(), "{refused:?}");
        }
    }

    #[test]
    fn records_each_signal_once_in_order() {
        let (dir, source) = opened();
        let message = source.add("alice", "one").unwrap();
        let id = message.msg_id.as_str();
        assert!(source.records(id).unwrap().is_empty());
        let before = since_epoch_ms();
        assert!(source.record(id, "transcript", None).unwrap());
        assert!(source.record(id, "reply", Some("tok-1")).unwrap());
        // The first record of a signal stays.
        assert!(!source.record(id, "transcript", Some("later")).unwrap());
        let records = source.records(id).unwrap();
        assert_eq!(
            records
                .iter()
                .map(|r| (r.signal.as_str(), r.detail.as_deref()))
                .collect::<Vec<_>>(),
            [("transcript", None), ("reply", Some("tok-1"))]
        );
        assert!(records[0].at_ms >= before && records[1].at_ms >= records[0].at_ms);
        for signal in ["", "Transcript", "a/b", "a.b"] {
            assert_eq!(
                source.record(id, signal, None).unwrap_err().kind(),
                ErrorKind::InvalidInput,
                "{signal}"
            );
        }
        assert!(source.record("../x", "ack", None).is_err());
        assert!(source.records("../x").is_err());
        assert_eq!(fs::read_dir(dir.path().join(TMP)).unwrap().count(), 0);
    }

    #[test]
    fn lists_requeues_and_resends_handed_over_messages() {
        let (_dir, source) = opened();
        let (_next, fallback) = opened();
        let first = source.add_with_ack("alice", "one").unwrap();
        let second = source.add("bob", "two").unwrap();
        assert!(source.handed_messages().unwrap().is_empty());
        source.handed_over(&ids(&source.take().unwrap())).unwrap();
        let handed = source.handed_messages().unwrap();
        assert_eq!(ids(&handed), ids(&[first.clone(), second.clone()]));
        assert!(handed.iter().all(|m| m.handed_over_at_ms.is_some()));

        // Resent by the next path: the same message, waiting there; still handed over here.
        let resent = source.resend(&first.msg_id, &fallback).unwrap();
        assert_eq!(resent, first);
        assert_eq!(fallback.take().unwrap(), vec![first.clone()]);
        assert!(matches!(
            source.state(&first.msg_id).unwrap(),
            Some(State::HandedOver { .. })
        ));
        assert!(source.resend("nope", &fallback).is_err());

        source
            .requeue(std::slice::from_ref(&second.msg_id))
            .unwrap();
        assert_eq!(source.state(&second.msg_id).unwrap(), Some(State::Waiting));
        assert_eq!(source.take().unwrap(), vec![second]);
        assert!(source.requeue(&["nope".to_owned()]).is_err());
    }

    #[test]
    fn registers_the_session_for_the_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, source) = opened();
        assert_eq!(source.registration().unwrap(), None);
        let first = Registration {
            session_id: "s1".to_owned(),
            socket: Some("/tmp/cc-socks/1.sock".to_owned()),
            token: Some("secret".to_owned()),
            at_ms: 5,
            ..Registration::default()
        };
        source.register(&first).unwrap();
        let second = Registration {
            session_id: "s2".to_owned(),
            transcript_path: Some("/t.jsonl".to_owned()),
            cwd: Some("/p".to_owned()),
            at_ms: 6,
            ..Registration::default()
        };
        source.register(&second).unwrap();
        let other = Source::open(dir.path()).unwrap();
        assert_eq!(other.registration().unwrap(), Some(second));
        let mode = fs::metadata(dir.path().join(SESSION))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(fs::read_dir(dir.path().join(TMP)).unwrap().count(), 0);
        fs::write(dir.path().join(SESSION), "{").unwrap();
        assert!(source.registration().is_err());
    }

    #[test]
    fn counts_the_waiting_messages() {
        let (_dir, source) = opened();
        assert_eq!(source.waiting_count().unwrap(), 0);
        source.add("alice", "one").unwrap();
        source.add("bob", "two").unwrap();
        assert_eq!(source.waiting_count().unwrap(), 2);
        source.take().unwrap();
        assert_eq!(source.waiting_count().unwrap(), 0);
    }

    #[test]
    fn unknown_message_has_no_state() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        assert_eq!(source.state("nope").unwrap(), None);
    }

    #[test]
    fn another_handle_does_not_take_a_taken_message() {
        let dir = tempfile::tempdir().unwrap();
        let first = Source::open(dir.path()).unwrap();
        let second = Source::open(dir.path()).unwrap();
        let message = first.add("alice", "one").unwrap();
        assert_eq!(first.take().unwrap(), vec![message.clone()]);
        // The first handle lives: its claim is not abandoned.
        assert!(second.take().unwrap().is_empty());
        assert_eq!(first.state(&message.msg_id).unwrap(), Some(State::Taken));
        // Only the handle that took a message marks it.
        assert!(
            second
                .handed_over(std::slice::from_ref(&message.msg_id))
                .is_err()
        );
        assert!(
            second
                .release(std::slice::from_ref(&message.msg_id))
                .is_err()
        );
    }

    #[test]
    fn abandoned_claim_is_handed_over_again() {
        let dir = tempfile::tempdir().unwrap();
        let crashed = Source::open(dir.path()).unwrap();
        let message = crashed.add("alice", "one").unwrap();
        assert_eq!(crashed.take().unwrap(), vec![message.clone()]);
        // A crash between the handover and marking it.
        drop(crashed);

        let restarted = Source::open(dir.path()).unwrap();
        assert_eq!(restarted.take().unwrap(), vec![message.clone()]);
        restarted
            .handed_over(std::slice::from_ref(&message.msg_id))
            .unwrap();
        assert!(restarted.take().unwrap().is_empty());
        // The abandoned claim is gone.
        assert_eq!(
            fs::read_dir(dir.path().join(CLAIMS)).unwrap().count(),
            1,
            "only the live handle's claim is left"
        );
        assert_eq!(fs::read_dir(dir.path().join(TAKEN)).unwrap().count(), 1);
    }

    #[test]
    fn handle_without_messages_removes_its_claim() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        source.add("alice", "one").unwrap();
        drop(source);
        for sub in [CLAIMS, TAKEN] {
            assert_eq!(
                fs::read_dir(dir.path().join(sub)).unwrap().count(),
                0,
                "{sub}"
            );
        }
    }

    #[test]
    fn concurrent_paths_take_each_message_once() {
        let dir = tempfile::tempdir().unwrap();
        let adder = Source::open(dir.path()).unwrap();
        let added: HashSet<String> = (0..200)
            .map(|i| adder.add("alice", &i.to_string()).unwrap().msg_id)
            .collect();

        let barrier = Arc::new(Barrier::new(4));
        let takers: Vec<_> = (0..4)
            .map(|_| {
                let path = dir.path().to_path_buf();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    let source = Source::open(path).unwrap();
                    barrier.wait();
                    let taken = source.take().unwrap();
                    source.handed_over(&ids(&taken)).unwrap();
                    ids(&taken)
                })
            })
            .collect();
        let mut taken = Vec::new();
        for taker in takers {
            taken.extend(taker.join().unwrap());
        }
        assert_eq!(taken.len(), added.len(), "no message is taken twice");
        assert_eq!(taken.into_iter().collect::<HashSet<_>>(), added);
    }
}
