//! The local state's lock (`docs/dev/architecture.md`, "Local state"): an OS file lock on a file in
//! the state directory, which a run that changes the local state holds until it exits. The OS frees
//! it when its process dies.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use crate::state;

/// The lock's file in the state directory.
pub const FILE: &str = "state.lock";

/// How often a run waiting for the lock tries it again.
const RETRY: Duration = Duration::from_millis(50);

/// The lock, held until it is dropped.
pub struct Lock {
    _file: File,
}

/// Takes the lock of the state in `dir`, waiting for another run that holds it for up to
/// `timeout`. A missing `dir` is created if `create`; otherwise, without a state in `dir`, there is
/// nothing to lock, and the run holds no lock and leaves no file.
pub fn take(dir: &Path, create: bool, timeout: Duration) -> Result<Option<Lock>, String> {
    let unusable = |err| format!("cannot use the state directory {}: {err}", dir.display());
    if create {
        state::create_dir(dir).map_err(unusable)?;
    } else if !dir.join(state::FILE).is_file() {
        return Ok(None);
    }
    let path = dir.join(FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|err| {
            format!(
                "cannot open the local state's lock {}: {err}",
                path.display()
            )
        })?;
    let deadline = Instant::now().checked_add(timeout);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(Lock { _file: file })),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(err)) => {
                return Err(format!(
                    "cannot take the local state's lock {}: {err}",
                    path.display()
                ));
            }
        }
        let wait = match deadline {
            Some(deadline) => deadline.saturating_duration_since(Instant::now()),
            None => RETRY,
        };
        if wait.is_zero() {
            return Err(format!(
                "another skald run holds the local state's lock: gave up waiting for it after {} \
                 seconds; nothing changed",
                timeout.as_secs()
            ));
        }
        thread::sleep(wait.min(RETRY));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    const SHORT: Duration = Duration::from_millis(300);

    #[test]
    fn a_second_take_waits_for_the_first_then_fails_naming_the_lock() {
        let dir = TestDir::new("lock-waits");
        std::fs::write(dir.join(state::FILE), b"").unwrap();
        let held = take(&dir, false, SHORT).unwrap();
        assert!(held.is_some());

        let started = Instant::now();
        let err = take(&dir, false, SHORT).err().expect("refused");
        assert!(started.elapsed() >= SHORT, "{:?}", started.elapsed());
        assert!(err.contains("lock"), "{err}");

        drop(held);
        assert!(take(&dir, false, SHORT).unwrap().is_some());
    }

    #[test]
    fn a_missing_state_is_locked_only_when_created() {
        let dir = TestDir::new("lock-missing");
        assert!(take(&dir, false, SHORT).unwrap().is_none());
        assert!(!dir.join(FILE).exists());
        let missing = dir.join("state");
        assert!(take(&missing, false, SHORT).unwrap().is_none());
        assert!(!missing.exists());

        assert!(take(&missing, true, SHORT).unwrap().is_some());
        assert!(missing.join(FILE).is_file());
    }
}
