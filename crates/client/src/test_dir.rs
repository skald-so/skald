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
pub struct TestDir(PathBuf);

impl TestDir {
    /// A new empty directory named after `name` and this process: `skald-client-<name>-<pid>`.
    pub fn new(name: &str) -> Self {
        Self::named(&format!("skald-client-{name}-{}", std::process::id()))
    }

    /// A new empty directory `name`, readable only by its owner; one an earlier run left is
    /// removed first. From here on it is removed on drop, so a test that fails while it fills the
    /// directory leaves nothing either.
    pub fn named(name: &str) -> Self {
        let dir = TestDir(std::env::temp_dir().join(name));
        remove(&dir.0).expect("remove the test directory an earlier run left");
        fs::create_dir(&dir.0).expect("create a test directory");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700))
                .expect("make the test directory its owner's only");
        }
        dir
    }
}

impl Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

/// A directory that cannot be removed fails the test; in a test failing already, it is reported.
impl Drop for TestDir {
    fn drop(&mut self) {
        if let Err(err) = remove(&self.0) {
            let failed = format!("cannot remove {}: {err}", self.0.display());
            if thread::panicking() {
                eprintln!("{failed}");
            } else {
                panic!("{failed}");
            }
        }
    }
}

/// Removes `dir` with everything in it, if it exists, retrying for up to `REMOVAL_LIMIT`.
fn remove(dir: &Path) -> io::Result<()> {
    let started = Instant::now();
    loop {
        match fs::remove_dir_all(dir) {
            Err(_) if !dir.exists() => return Ok(()),
            Err(_) if started.elapsed() < REMOVAL_LIMIT => {
                thread::sleep(Duration::from_millis(100));
            }
            result => return result,
        }
    }
}
