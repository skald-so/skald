//! A local message source that stands in for the helper: a test or a person adds a message, a
//! delivery path takes the waiting messages and marks each handed over after the handover, as
//! `docs/protocol/delivery.md`, "Device to agent", says.
//!
//! The source is a directory that several processes share:
//! - `waiting/<msg_id>.json`: messages not yet taken;
//! - `taken/<claim>/<msg_id>.json`: messages a path took and is handing over;
//! - `claims/<claim>.lock`: locked by the [`Source`] that owns the claim while it lives;
//! - `handed/<msg_id>.json`: messages handed over, with the time of the handover;
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
const TMP: &str = "tmp";
const RECOVERY: &str = "recovery.lock";

impl Source {
    /// Opens the source in `dir`, creating it if needed.
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        for sub in [WAITING, TAKEN, CLAIMS, HANDED, TMP] {
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

    /// Adds a message from `sender` and returns it.
    pub fn add(&self, sender: &str, text: &str) -> io::Result<Message> {
        let message = Message {
            msg_id: Uuid::now_v7().to_string(),
            sender: sender.to_owned(),
            text: text.to_owned(),
            added_at_ms: now_ms(),
            handed_over_at_ms: None,
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

    /// The handed-over message `msg_id`, with the time of its handover.
    pub fn handed(&self, msg_id: &str) -> io::Result<Message> {
        read(&self.dir.join(HANDED).join(file_name(msg_id)))
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
