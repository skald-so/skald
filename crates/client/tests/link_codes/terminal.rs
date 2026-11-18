//! A stand-in for the helper that runs `skald` at a terminal, a pseudo-terminal on Unix and a
//! pseudo-console on Windows, which comes with the tests of the recovery key
//! (`recovery_key/terminal.rs`). It has that helper's interface, plus [`Terminal::type_line`], and
//! fails every test that starts a run with it. Once that helper is in place, this target takes it
//! instead, with `type_line` added there.

use crate::machine::Machine;

/// A run of `skald` at a terminal.
pub struct Terminal {
    _never: (),
}

/// What a run at a terminal printed, as the person saw it, and how it exited.
#[derive(Debug)]
pub struct Shown {
    pub success: bool,
    pub text: String,
}

/// Why each method fails.
const MISSING: &str =
    "the helper that runs skald at a terminal comes with the recovery key's tests";

impl Terminal {
    /// Starts `skald` with `args` on `machine` at a terminal, in the machine's home, set up as the
    /// machine's other runs are.
    pub fn start(_machine: &Machine, _args: &[&str]) -> Self {
        panic!("{MISSING}")
    }

    /// What it printed so far, as the person sees it.
    pub fn shown(&self) -> String {
        panic!("{MISSING}")
    }

    /// Whether it has exited.
    pub fn exited(&mut self) -> bool {
        panic!("{MISSING}")
    }

    /// Waits until what it printed passes `seen`, for up to the run limit from its start; gives
    /// what it printed then. Fails if it exits first.
    pub fn wait_for(&mut self, _what: &str, _seen: impl Fn(&str) -> bool) -> String {
        panic!("{MISSING}")
    }

    /// Waits until it shows `part`, as [`Terminal::wait_for`] does.
    pub fn wait_to_show(&mut self, _part: &str) -> String {
        panic!("{MISSING}")
    }

    /// The person presses Enter.
    pub fn enter(&self) {
        panic!("{MISSING}")
    }

    /// The person types `text` and presses Enter.
    pub fn type_line(&self, _text: &str) {
        panic!("{MISSING}")
    }

    /// Kills it at once, as the system does; gives what it printed until then.
    pub fn kill(self) -> Shown {
        panic!("{MISSING}")
    }

    /// Waits for it to exit, for up to the run limit from its start; gives what it printed.
    pub fn finish(self) -> Shown {
        panic!("{MISSING}")
    }
}
