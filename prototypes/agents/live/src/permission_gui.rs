//! The GUI live checks of the paths that tell that a Claude Code session waits on its own
//! permission prompt, on the desktop app and VS Code: `gui-permission` prepares the check for the
//! developer or reviewer who drives the surface, `gui-permission-link` opens the desktop app's
//! `needs-input` link and `gui-permission-result` reads what the session recorded.

use std::path::Path;

use agent_live::gui::{Surface, session_versions};
use agent_live::permission::{self, Candidate, Observed, PermissionCheck};

use crate::Result;
use crate::gui_run::{home, installed, open, user_claude};

/// Prepares the GUI check on `surface` in a new temporary directory, which stays: prints the
/// surface's version, the project to open, the prompt to type and the commands to run next.
pub fn gui(surface: Surface) -> Result<bool> {
    let version = surface.version(&installed()?)?;
    let work = tempfile::Builder::new()
        .prefix("skald-live-")
        .tempdir()?
        .keep();
    let live = std::env::current_exe()?;
    let project = permission::prepare_gui(&work, surface, &version, &live, &user_claude(&home()?))?;
    println!("{version}");
    println!("project: {}", project.display());
    println!("prompt: {}", permission::prompt());
    if surface == Surface::ClaudeDesktop {
        println!(
            "while the prompt waits: live-check gui-permission-link {}",
            work.display()
        );
    }
    println!(
        "once it is answered: live-check gui-permission-result {}",
        work.display()
    );
    Ok(true)
}

/// Opens the desktop app's `needs-input` link to the session of the GUI check in `work`, and prints
/// it.
pub fn gui_link(work: &Path) -> Result<bool> {
    open(&permission::needs_input(work, &home()?)?)
}

/// Prints the version of the surface and of the session, and what the session of the GUI check in
/// `work` recorded.
pub fn gui_result(work: &Path) -> Result<bool> {
    let (surface, version) = permission::gui_check(work)?;
    let sessions = session_versions(&home()?, &work.join("project"))?;
    if sessions.is_empty() {
        println!("{version}, no session transcript");
    } else {
        println!(
            "{version}; the session ran Claude Code {}",
            sessions.join(", ")
        );
    }
    let mut check = PermissionCheck::default();
    let lines = check.session(
        surface.name(),
        &Observed::read(work),
        &Candidate::GUI,
        None,
        "",
    );
    for line in lines {
        println!("{line}");
    }
    let verdict = if check.pass() { "pass" } else { "fail" };
    println!("permission prompts on {}: {verdict}", surface.name());
    Ok(check.pass())
}
