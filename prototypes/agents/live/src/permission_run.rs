//! Runs the live checks of the paths that tell that a Claude Code session waits on its own
//! permission prompt: on the CLI in a pseudo-terminal, as a person answers the dialog there; on the
//! desktop app and VS Code, `gui-permission` prepares the check for the developer or reviewer who
//! drives the surface, `gui-permission-link` opens the desktop app's `needs-input` link and
//! `gui-permission-result` reads what the session recorded. Everything else is in the library's
//! `permission` module.

use std::path::Path;
use std::time::{Duration, Instant};

use agent_live::channel::{CHANNEL_BIN, ENTER, Files, extra_environment, stub_environment};
use agent_live::gui::{Surface, session_versions};
use agent_live::permission::{self, Answer, Candidate, DIALOG, Observed, PermissionCheck, SCRIPT};
use agent_live::{Agent, Options};
use agent_source::now_ms;

use crate::Result;
use crate::gui_run::{home, installed, open, user_claude};
use crate::pty::{Env, Live, Prepared, TICK, TURN};
use crate::run::{finish, start_with};

/// How long the dialog waits before the check answers it: past the `Notification` hook's six
/// seconds.
const HOLD: Duration = Duration::from_secs(10);

/// How long the session runs once the dialog is answered.
const AFTER: Duration = Duration::from_secs(15);

/// The CLI check: a session whose dialog the check answers yes; with `--all`, also one it answers
/// no.
pub fn permission(options: &Options) -> Result<bool> {
    let (work, channel, path) = start_with(Agent::Claude, CHANNEL_BIN)?;
    let live = std::env::current_exe()?;
    let mut answers = vec![("a", Answer::Yes)];
    if options.all {
        answers.push(("b", Answer::No));
    }
    let mut check = PermissionCheck::default();
    for (name, answer) in answers {
        let files = Files::new(&work.path().join(name));
        let args = permission::session(&files, &live, &channel, options.model)?;
        let stub = options.stub(SCRIPT)?;
        let env = Env::of(
            &files,
            stub.as_ref(),
            &path,
            stub_environment,
            extra_environment(options.real),
        )?;
        let dir = files.dir.clone();
        let mut session =
            Prepared::new(name, Agent::Claude, true, files, args, env, stub)?.start()?;
        let mut observed = Observed::default();
        // A prompt that opens before the channel is ready is never relayed.
        let ready = session.wait(TURN, |_| Observed::read(&dir).channel_ready_ms().is_some())?;
        if !ready {
            eprintln!("live-check: session {name}'s channel never got ready");
        }
        session.press(permission::prompt().as_bytes())?;
        // The terminal UI takes text typed at once as a paste, and Enter after it as sending it.
        session.pump_for(Duration::from_millis(500))?;
        session.press(ENTER)?;
        if shows(&mut session, DIALOG, TURN)? {
            observed.shown_ms = Some(now_ms());
            session.pump_for(HOLD)?;
            session.press(answer.keys())?;
            observed.answered_ms = Some(now_ms());
            session.pump_for(AFTER)?;
        } else {
            eprintln!("live-check: session {name} showed no permission dialog");
        }
        let session = session.end()?;
        observed = Observed {
            shown_ms: observed.shown_ms,
            answered_ms: observed.answered_ms,
            ..Observed::read(&dir)
        };
        let lines = check.session(
            name,
            &observed,
            &Candidate::CLI,
            Some(answer),
            &session.summary(),
        );
        for line in lines {
            println!("{line}");
        }
    }
    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// Runs the session until its terminal shows `text` or `limit` passes; whether it showed.
fn shows(live: &mut Live, text: &str, limit: Duration) -> Result<bool> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if live.showed(text) {
            return Ok(true);
        }
        if !live.pump_for(TICK)? {
            return Ok(live.showed(text));
        }
    }
    Ok(false)
}

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
    println!(
        "{version}; the session ran Claude Code {}",
        sessions.join(", ")
    );
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
