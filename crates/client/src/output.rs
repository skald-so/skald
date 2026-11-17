//! A run's output: standard output and standard error, unless a helper runs the operation for a
//! command ([`crate::helper`]); then each write goes to that command, which shows it. The crate's
//! `print!`, `println!` and `eprintln!` write here (`main.rs`). A run as an agent holds its
//! warnings back for the person ([`holding_warnings`]).

use std::cell::{Cell, RefCell};
use std::fmt;
use std::io::{self, Write};

/// Which of a run's outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

/// What takes the writes of an operation the helper runs.
type Sink<'s> = dyn FnMut(Stream, &[u8]) -> io::Result<()> + 's;

thread_local! {
    /// The sink of the operation running on this thread, if a helper runs it.
    static SINK: RefCell<Option<Box<Sink<'static>>>> = const { RefCell::new(None) };
    /// How many writes of this thread's runs went out.
    static WRITTEN: Cell<u64> = const { Cell::new(0) };
    /// The warnings held back from the run on this thread, if it holds them.
    static HELD: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// What starts a warning's line.
pub const WARNING: &str = "skald: warning: ";

/// Runs `run`, holding back each warning it writes to its error output, a line that starts with
/// [`WARNING`]; gives what it gave and those warnings, each without its line's end.
pub fn holding_warnings<R>(run: impl FnOnce() -> R) -> (R, Vec<String>) {
    /// Stops holding when the run ends, a panic included.
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            HELD.with(|held| held.borrow_mut().take());
        }
    }
    HELD.with(|held| *held.borrow_mut() = Some(Vec::new()));
    let reset = Reset;
    let ran = run();
    let held = HELD
        .with(|held| held.borrow_mut().take())
        .unwrap_or_default();
    drop(reset);
    (ran, held)
}

/// Holds `text` back if the run holds its warnings and it is one; gives whether it did.
fn held(stream: Stream, text: &str) -> bool {
    if stream != Stream::Err || !text.starts_with(WARNING) {
        return false;
    }
    HELD.with(|held| match held.borrow_mut().as_mut() {
        Some(held) => {
            held.push(text.trim_end_matches('\n').to_owned());
            true
        }
        None => false,
    })
}

/// How many writes this thread's runs made so far: a run that makes one shows something.
pub fn written() -> u64 {
    WRITTEN.with(Cell::get)
}

/// Runs `run` with each write of its output going to `sink`.
pub fn into<R>(
    sink: impl FnMut(Stream, &[u8]) -> io::Result<()> + 'static,
    run: impl FnOnce() -> R,
) -> R {
    /// Takes the sink away when the run ends, a panic included.
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            SINK.with(|sink| sink.borrow_mut().take());
        }
    }
    SINK.with(|current| *current.borrow_mut() = Some(Box::new(sink)));
    let _reset = Reset;
    run()
}

/// Writes `bytes` to `stream` and flushes it.
pub fn write(stream: Stream, bytes: &[u8]) -> io::Result<()> {
    let written = sent(stream, bytes);
    if written.is_ok() {
        WRITTEN.with(|count| count.set(count.get() + 1));
    }
    written
}

/// [`write`], uncounted.
fn sent(stream: Stream, bytes: &[u8]) -> io::Result<()> {
    let sunk = SINK.with(|sink| sink.borrow_mut().as_mut().map(|sink| sink(stream, bytes)));
    if let Some(result) = sunk {
        return result;
    }
    match stream {
        Stream::Out => {
            let mut out = io::stdout().lock();
            out.write_all(bytes).and_then(|()| out.flush())
        }
        Stream::Err => io::stderr().lock().write_all(bytes),
    }
}

/// Writes `args` to `stream`, as `print!` and `eprint!` do: a failure to write to the run's own
/// output panics; one to the command a helper runs it for is that command's to see, which has gone.
pub fn print(stream: Stream, args: fmt::Arguments<'_>) {
    let text = args.to_string();
    if held(stream, &text) {
        return;
    }
    let written = write(stream, text.as_bytes());
    let helped = SINK.with(|sink| sink.borrow().is_some());
    if let Err(err) = written
        && !helped
    {
        let name = match stream {
            Stream::Out => "stdout",
            Stream::Err => "stderr",
        };
        panic!("failed printing to {name}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run that holds its warnings holds back each line of its error output that starts as a
    /// warning, and only those, until it ends.
    #[test]
    fn held_run_holds_back_only_its_warnings() {
        let ((), warnings) = holding_warnings(|| {
            assert!(held(Stream::Err, "skald: warning: one\n"));
            assert!(!held(Stream::Err, "skald: dropped #k3vq: why\n"));
            assert!(!held(Stream::Out, "skald: warning: on the output\n"));
            assert!(held(Stream::Err, "skald: warning: two"));
        });

        assert_eq!(warnings, ["skald: warning: one", "skald: warning: two"]);
        assert!(!held(Stream::Err, "skald: warning: after the run\n"));
    }
}
