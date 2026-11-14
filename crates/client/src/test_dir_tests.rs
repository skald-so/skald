//! The tests of [`TestDir`]: here, not in `test_dir.rs`, so that only the client's binary runs
//! them, not every integration test binary that includes that file.

use std::env;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::test_dir::TestDir;

/// A short removal limit, for a directory the test keeps from being removed.
const SHORT_LIMIT: Duration = Duration::from_millis(300);

/// Keeps a directory from being removed until it drops: on Unix a subdirectory holds a file
/// and its owner may not delete it; on Windows a file is open without sharing its deletion.
struct Block {
    #[cfg(unix)]
    locked: std::path::PathBuf,
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
    fs::remove_dir_all(&path).expect("remove the test's directory");

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
        fs::remove_dir_all(&path).expect("remove the test's directory");
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
