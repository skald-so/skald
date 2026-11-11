//! The GUI live checks' commands: `gui` prepares a check for the developer or reviewer who drives
//! the surface, `gui-result` takes the token they read. Everything else is in the library's `gui`
//! module.

use std::path::{Path, PathBuf};

use agent_live::gui::{self, Binaries, GuiPath, Installed, Surface};
use agent_live::hook::HOOK_BIN;
use agent_live::{INBOX_BIN, binary, channel::CHANNEL_BIN, token};

use crate::Result;

/// Prepares the GUI check of `path` on `surface` in a new temporary directory, which stays: prints
/// the surface's version, the project to open, the prompt to type and the command to run with the
/// token read from the reply. It never prints the token.
pub fn prepare(path: GuiPath, surface: Surface) -> Result<bool> {
    let installed = Installed {
        applications: PathBuf::from("/Applications"),
        home: std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("HOME is not set")?,
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
    let prepared = gui::prepare(&work, surface, path, &binaries, &version, &token())?;
    println!("{version}");
    println!("project: {}", prepared.project.display());
    println!("prompt: {}", prepared.prompt);
    println!(
        "then: live-check gui-result {} <the token read from the reply>",
        work.display()
    );
    Ok(true)
}

/// Prints the surface's version and the outcome of the GUI check prepared in `work`, given the
/// token read from the reply.
pub fn result(work: &Path, token: &str) -> Result<bool> {
    let outcome = gui::result(work, token)?;
    println!("{}", outcome.version);
    println!("{}", outcome.details);
    println!("{}", outcome.verdict);
    Ok(outcome.pass)
}
