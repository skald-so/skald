//! The GUI live checks' commands: `gui` prepares a check for the developer or reviewer who drives
//! the surface, `gui-add` adds the busy case's message, `gui-result` takes the token they read.
//! Everything else is in the library's `gui` module.

use std::path::{Path, PathBuf};

use agent_live::gui::{self, Binaries, GuiPath, Installed, Surface};
use agent_live::hook::HOOK_BIN;
use agent_live::{INBOX_BIN, binary, channel::CHANNEL_BIN, token};

use crate::Result;

/// Prepares the GUI check of `path` on `surface` in a new temporary directory, which stays: prints
/// the surface's version, the project to open, the prompt to type and the commands to run next. It
/// never prints the token. In the `busy` case the message waits for `gui-add`.
pub fn prepare(path: GuiPath, surface: Surface, busy: bool) -> Result<bool> {
    let installed = Installed {
        applications: PathBuf::from("/Applications"),
        home: home()?,
    };
    let version = surface.version(&installed)?;
    let exe = std::env::current_exe()?;
    let binaries = Binaries {
        inbox: binary(&exe, INBOX_BIN)?,
        channel: binary(&exe, CHANNEL_BIN)?,
        hook: binary(&exe, HOOK_BIN)?,
    };
    let work = tempfile::Builder::new()
        .prefix("skald-live-")
        .tempdir()?
        .keep();
    let prepared = gui::prepare(&work, surface, path, &binaries, &version, &token(), busy)?;
    println!("{version}");
    println!("project: {}", prepared.project.display());
    println!("prompt: {}", prepared.prompt);
    if busy {
        println!(
            "once the shell command runs: live-check gui-add {}",
            work.display()
        );
    }
    println!(
        "then: live-check gui-result {} <the token read from the reply>",
        work.display()
    );
    Ok(true)
}

/// Prints the surface's version and the outcome of the GUI check prepared in `work`, given the
/// token read from the reply.
pub fn result(work: &Path, token: &str) -> Result<bool> {
    let outcome = gui::result(work, token, &home()?)?;
    println!("{}", outcome.version);
    println!("{}", outcome.details);
    println!("{}", outcome.verdict);
    Ok(outcome.pass)
}

/// Adds the message of the GUI check prepared in `work` and prints its `msg_id`, not its token.
pub fn add(work: &Path) -> Result<bool> {
    println!("{}", gui::add(work)?.msg_id);
    Ok(true)
}

/// The user's home: VS Code's extensions and Claude Code's transcripts.
fn home() -> Result<PathBuf> {
    Ok(std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?)
}
