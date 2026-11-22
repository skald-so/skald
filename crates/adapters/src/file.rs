//! Files the client and the adapters share how to write.

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

/// Writes `bytes` to a new file at `path` that, on Unix, only this user can read from the start;
/// removes it if the write fails.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options.open(path)?.write_all(bytes);
    if written.is_err() {
        let _ = fs::remove_file(path);
    }
    written
}
