//! What the readers of agents' session files share: Claude Code's ([`crate::claude_socket::Scan`])
//! and Codex's rollouts ([`crate::codex_confirm::Scan`]).

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use crate::state::State;

/// A reader of a session file, which goes on from where it stopped.
pub trait Reader: Default {
    /// How far into the file it read.
    fn offset(&mut self) -> &mut u64;

    /// Reads `line`, a whole one, adding what it confirms to `confirmed`.
    fn entry(&mut self, line: &[u8], confirmed: &mut Vec<String>);

    /// Reads the whole lines `file` got since, but a line still being written; gives what they
    /// confirm. A file shorter than the offset is read from its start, by a new reader; one that
    /// is gone gives nothing.
    fn read(&mut self, file: &Path) -> Vec<String> {
        let mut confirmed = Vec::new();
        let Ok(mut opened) = File::open(file) else {
            return confirmed;
        };
        let length = opened.metadata().map_or(0, |metadata| metadata.len());
        if length < *self.offset() {
            *self = Self::default();
        }
        let at = *self.offset();
        if length == at || opened.seek(SeekFrom::Start(at)).is_err() {
            return confirmed;
        }
        let mut reader = BufReader::new(opened);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(read) if read > 0 && line.ends_with(b"\n") => {}
                // The end, or a line still being written.
                _ => break,
            }
            *self.offset() += line.len() as u64;
            self.entry(&line, &mut confirmed);
        }
        confirmed
    }
}

/// The state in `dir`, opened once into `state`; `None` if it does not open.
pub fn opened<'s>(state: &'s mut Option<State>, dir: &Path) -> Option<&'s State> {
    if state.is_none() {
        *state = State::open(dir).ok();
    }
    state.as_ref()
}

/// For a reader's tests: what `read` gives on a session file of `first`, then on it after `more`,
/// in a directory of `name`'s.
#[cfg(test)]
pub fn read_twice(
    name: &str,
    (first, more): (&str, &str),
    mut read: impl FnMut(&Path) -> Vec<String>,
) -> (Vec<String>, Vec<String>) {
    use std::io::Write;
    // `cargo test` runs the tests in threads of one process: each file has a directory of its own.
    static FILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let count = FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = crate::test_dir::TestDir::new(&format!("{name}-scan-{count}"));
    let file = dir.join("session.jsonl");
    std::fs::write(&file, first).unwrap();
    let before = read(&file);
    let mut opened = std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap();
    opened.write_all(more.as_bytes()).unwrap();
    (before, read(&file))
}
