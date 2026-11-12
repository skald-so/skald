//! Directories for unit tests, removed on drop even when a test fails.

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};

pub struct TestDir(PathBuf);

impl TestDir {
    /// A new empty directory named after `name` and this process.
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("skald-client-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        crate::state::create_dir(&dir).expect("create a test directory");
        TestDir(dir)
    }
}

impl Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
