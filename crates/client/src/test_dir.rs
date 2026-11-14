//! Directories for the client's tests in the temporary directory, removed on drop even when a test
//! fails. The unit tests use it, and the integration tests through `#[path]`.

use std::fs;
use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// How long a removal is retried: on Windows a file cannot be removed while a process has it open,
/// such as an antivirus scanning a program a test ran.
const REMOVAL_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct TestDir {
    path: PathBuf,
    /// How long its removal on drop is retried.
    pub removal_limit: Duration,
}

impl TestDir {
    /// A new empty directory named after `name` and this process: `skald-client-<name>-<pid>`.
    pub fn new(name: &str) -> Self {
        Self::named(&format!("skald-client-{name}-{}", std::process::id()))
    }

    /// A new empty directory `name`: on Unix readable only by its owner; on Windows it has the
    /// access rights it inherits from the temporary directory. One an earlier run left is removed
    /// first. From here on it is removed on drop, so a test that fails while it fills the
    /// directory leaves nothing either.
    pub fn named(name: &str) -> Self {
        let dir = TestDir {
            path: std::env::temp_dir().join(name),
            removal_limit: REMOVAL_LIMIT,
        };
        remove(&dir.path, REMOVAL_LIMIT).expect("remove the test directory an earlier run left");
        fs::create_dir(&dir.path).expect("create a test directory");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir.path, fs::Permissions::from_mode(0o700))
                .expect("make the test directory its owner's only");
        }
        dir
    }
}

impl Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

/// A directory that cannot be removed fails the test; in a test failing already, it is reported.
impl Drop for TestDir {
    fn drop(&mut self) {
        if let Err(err) = remove(&self.path, self.removal_limit) {
            let failed = format!("cannot remove {}: {err}", self.path.display());
            if thread::panicking() {
                eprintln!("{failed}");
            } else {
                panic!("{failed}");
            }
        }
    }
}

/// Removes `dir` with everything in it, if it exists, retrying for up to `limit`.
fn remove(dir: &Path, limit: Duration) -> io::Result<()> {
    let started = Instant::now();
    loop {
        match fs::remove_dir_all(dir) {
            Err(_) if !dir.exists() => return Ok(()),
            Err(_) if started.elapsed() < limit => {
                thread::sleep(Duration::from_millis(100));
            }
            result => return result,
        }
    }
}
