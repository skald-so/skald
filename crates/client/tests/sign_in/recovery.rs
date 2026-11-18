//! The account's first device's recovery key, shown at the person's terminal: a test's sign-in into
//! a pipe leaves it waiting, and every later run would say so.

// The recovery key's tests load it at their root too.
#[path = "../recovery_key/terminal.rs"]
#[allow(dead_code, clippy::duplicate_mod)]
mod terminal;

use super::machine::{Finished, Machine, SERVER, SIGN_IN};
use terminal::Terminal;

/// Once `signed_in`, a sign-in of `machine` on `server` into a pipe, says a recovery key waits: the
/// person's `skald sign-in` at a terminal shows it and the person presses Enter, so it is deleted.
pub fn shown_if_waiting(machine: &Machine, server: &str, signed_in: &Finished) {
    let command = format!("skald sign-in --server {server}");
    let waits = signed_in
        .shown()
        .lines()
        .any(|line| line.contains(&command) && line.contains("recovery key"));
    if !waits {
        return;
    }
    let mut run = Terminal::start(machine, &[SIGN_IN, SERVER, server]);
    run.wait_to_show("Press Enter");
    run.enter();
    let shown = run.finish();
    assert!(
        shown.success && shown.text.contains("recovery key is deleted"),
        "the recovery key was not shown: {shown:?}"
    );
}
