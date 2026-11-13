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
    removal_limit: Duration,
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

#[cfg(test)]
mod tests {
    use std::env;
    use std::panic::{self, AssertUnwindSafe};
    use std::process::Command;

    use super::*;

    /// A short removal limit, for a directory the test keeps from being removed.
    const SHORT_LIMIT: Duration = Duration::from_millis(300);

    /// Keeps a directory from being removed until it drops: on Unix a subdirectory holds a file
    /// and its owner may not delete it; on Windows a file is open without sharing its deletion.
    struct Block {
        #[cfg(unix)]
        locked: PathBuf,
        #[cfg(windows)]
        _file: fs::File,
    }

    impl Block {
        fn new(dir: &Path) -> Self {
            let locked = dir.join("locked");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::create_dir(&locked).expect("create a subdirectory");
                fs::write(locked.join("file"), "").expect("write a file");
                fs::set_permissions(&locked, fs::Permissions::from_mode(0o500))
                    .expect("forbid deleting the file");
                Block { locked }
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                let file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .share_mode(0)
                    .open(&locked)
                    .expect("open a file without sharing it");
                Block { _file: file }
            }
        }
    }

    impl Drop for Block {
        fn drop(&mut self) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&self.locked, fs::Permissions::from_mode(0o700))
                    .expect("allow deleting the file");
            }
        }
    }

    #[test]
    fn a_directory_that_cannot_be_removed_fails_the_test_after_the_retries() {
        let mut dir = TestDir::new("unremovable");
        dir.removal_limit = SHORT_LIMIT;
        let path = dir.to_path_buf();
        let block = Block::new(&path);
        let started = Instant::now();
        let failed = panic::catch_unwind(AssertUnwindSafe(move || drop(dir)));
        let elapsed = started.elapsed();
        drop(block);
        remove(&path, REMOVAL_LIMIT).expect("remove the test's directory");

        let failed = failed.expect_err("the directory is left, yet its drop did not panic");
        let message = failed.downcast_ref::<String>().expect("a message");
        let expected = format!("cannot remove {}: ", path.display());
        assert!(message.starts_with(&expected), "{message}");
        assert!(elapsed >= SHORT_LIMIT, "gave up after {elapsed:?}");
    }

    #[test]
    fn a_removal_is_retried_until_it_succeeds() {
        let dir = TestDir::new("held");
        let path = dir.to_path_buf();
        let block = Block::new(&path);
        let held = SHORT_LIMIT;
        let release = thread::spawn(move || {
            thread::sleep(held);
            drop(block);
        });
        let started = Instant::now();
        drop(dir);
        let elapsed = started.elapsed();
        release.join().expect("release the directory");
        assert!(!path.exists(), "{} is left", path.display());
        assert!(elapsed >= held, "removed after {elapsed:?}");
    }

    /// Set for this test binary run by [`in_a_failing_test_a_directory_left_is_printed`]: the
    /// test then fails a thread whose directory cannot be removed.
    const CHILD: &str = "SKALD_TEST_DIR_FAILING_TEST";

    #[test]
    fn in_a_failing_test_a_directory_left_is_printed() {
        let name = "unremovable-in-a-failing-test";
        if env::var_os(CHILD).is_some() {
            let mut dir = TestDir::new(name);
            dir.removal_limit = SHORT_LIMIT;
            let path = dir.to_path_buf();
            let block = Block::new(&path);
            let failed = thread::spawn(move || {
                let _dir = dir;
                panic!("the test fails");
            })
            .join();
            drop(block);
            remove(&path, REMOVAL_LIMIT).expect("remove the test's directory");
            let failed = failed.expect_err("the test passed");
            assert_eq!(failed.downcast_ref::<&str>(), Some(&"the test fails"));
            return;
        }

        // This test in this binary, by its name without the crate's.
        let module = module_path!().split_once("::").map_or("", |(_, rest)| rest);
        let test = format!("{module}::in_a_failing_test_a_directory_left_is_printed");
        let output = Command::new(env::current_exe().expect("the test binary"))
            .args([&test, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output()
            .expect("run the test binary");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{}\n{stderr}", output.status);
        let left = env::temp_dir().join(format!("skald-client-{name}-"));
        let expected = format!("cannot remove {}", left.display());
        assert!(
            stderr.lines().any(|line| line.starts_with(&expected)),
            "{stderr}"
        );
        assert!(stderr.contains("the test fails"), "{stderr}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("1 passed"), "{stdout}");
    }
}
