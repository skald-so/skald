//! Runs the CLI live check of the paths that tell that a Claude Code session waits on its own
//! permission prompt, in a pseudo-terminal, as a person answers the dialog there. The GUI checks
//! are in `permission_gui`; everything else is in the library's `permission` module.

use std::time::{Duration, Instant};

use agent_live::channel::{CHANNEL_BIN, ENTER, Files, extra_environment, stub_environment};
use agent_live::permission::{self, Answer, Candidate, DIALOG, Observed, PermissionCheck, SCRIPT};
use agent_live::{Agent, Options};
use agent_source::now_ms;

use crate::Result;
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
        if ready {
            session.press(permission::prompt().as_bytes())?;
            // The terminal UI takes text typed at once as a paste, and Enter after it as sending it.
            session.pump_for(Duration::from_millis(500))?;
            session.press(ENTER)?;
        } else {
            eprintln!("live-check: session {name}'s channel never got ready");
        }
        if ready {
            (observed.shown_ms, observed.answered_ms) =
                answer_dialog(&mut session, name, DIALOG, answer.keys(), AFTER)?;
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

/// Waits for the session `name`'s `dialog`, holds it [`HOLD`], answers it `keys` and runs the
/// session `after`; when the dialog showed and when the check answered it, or neither if it never
/// showed.
pub(crate) fn answer_dialog(
    live: &mut Live,
    name: &str,
    dialog: &str,
    keys: &[u8],
    after: Duration,
) -> Result<(Option<u64>, Option<u64>)> {
    if !shows(live, dialog, TURN)? {
        eprintln!("live-check: session {name} showed no permission dialog");
        return Ok((None, None));
    }
    let shown = now_ms();
    live.pump_for(HOLD)?;
    live.press(keys)?;
    let answered = now_ms();
    live.pump_for(after)?;
    Ok((Some(shown), Some(answered)))
}

/// Runs the session until its terminal shows `text` or `limit` passes; whether it showed.
pub(crate) fn shows(live: &mut Live, text: &str, limit: Duration) -> Result<bool> {
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
