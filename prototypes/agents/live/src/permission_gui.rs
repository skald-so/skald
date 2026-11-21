//! The GUI live checks of the paths that tell that a session waits on its own permission prompt,
//! for the developer or reviewer who drives the surface. On the Claude desktop app and Claude Code
//! for VS Code: `gui-permission` prepares the check, `gui-permission-link` opens the desktop app's
//! `needs-input` link and `gui-permission-result` reads what the session recorded. On the Codex
//! desktop app and Codex for VS Code, `gui permission-hook` prepares the check of the
//! `PermissionRequest` hook and `gui-result` without a token reads its hook runs.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agent_live::codex_permission;
use agent_live::gui::{Surface, session_versions};
use agent_live::limits::mcp_servers;
use agent_live::permission::{self, Candidate, Observed, PermissionCheck};

use crate::Result;
use crate::gui_run::{home, installed, open, user_claude};
use crate::subscription::user_codex_config;

/// Prepares a GUI check on `surface` with `write`, given the check's new directory in `base`, which
/// stays, the surface's version line and `live-check`: prints the version, the project `write`
/// returns and `prompt`; returns the directory.
fn prepare(
    surface: Surface,
    base: &Path,
    prompt: &str,
    write: impl FnOnce(&Path, &str, &Path) -> io::Result<PathBuf>,
) -> Result<PathBuf> {
    let version = surface.version(&installed()?)?;
    fs::create_dir_all(base)?;
    let work = tempfile::Builder::new()
        .prefix("skald-live-")
        .tempdir_in(base)?
        .keep();
    let project = write(&work, &version, &std::env::current_exe()?)?;
    println!("{version}");
    println!("project: {}", project.display());
    println!("prompt: {prompt}");
    Ok(work)
}

/// Prepares the GUI check on the Claude `surface`: prints the surface's version, the project to
/// open, the prompt to type and the commands to run next.
pub fn gui(surface: Surface) -> Result<bool> {
    let user = user_claude(&home()?);
    let work = prepare(
        surface,
        &std::env::temp_dir(),
        &permission::prompt(),
        |work, version, live| permission::prepare_gui(work, surface, version, live, &user),
    )?;
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

/// Prepares the GUI check of the `PermissionRequest` hook on the Codex `surface` in
/// `~/Library/Caches`: out of `$TMPDIR` and `/tmp`, which the `workspace-write` sandbox may let the
/// agent write. Prints the surface's version, the project to open, the prompt to type and the
/// command to run next.
pub fn codex_gui(surface: Surface) -> Result<bool> {
    let user = mcp_servers(&user_codex_config());
    let work = prepare(
        surface,
        &home()?.join("Library/Caches"),
        &codex_permission::gui_prompt(),
        |work, version, live| codex_permission::prepare_gui(work, surface, version, live, &user),
    )?;
    println!(
        "once it is answered: live-check gui-result {}",
        work.display()
    );
    Ok(true)
}

/// Prints the output of the Codex GUI check in `work`: the surface's version, what its hook runs
/// tell and the verdict.
pub fn codex_gui_result(work: &Path) -> Result<bool> {
    let (lines, pass) = codex_permission::gui_result(work)?;
    for line in lines {
        println!("{line}");
    }
    Ok(pass)
}
