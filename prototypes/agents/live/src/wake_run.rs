//! Runs the live checks of the paths that wake an idle Claude Code session: the inbox socket, an
//! `asyncRewake` hook and a mod. Everything that runs no CLI is in the library's `wake` module.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use agent_live::channel::{Files, PtySession, Sent, extra_environment, stub_environment};
use agent_live::hook::HOOK_BIN;
use agent_live::wake::{self, Bypass, Wake, WakeBinaries, WakeCheck, held_notice};
use agent_live::{Agent, Call, INBOX_BIN, Kind, Options, binary, channel, model_use, token};
use agent_socket::{FROM, Receipts, hand_over};
use agent_source::Source;

use crate::Result;
use crate::pty::{Added, Env, Live, Prepared, turn_ended};
use crate::run::{finish, read_lines, side_by_side, start_with};

/// The longest wait for a turn or for the session to register.
const TURN: Duration = Duration::from_secs(120);

/// The longest wait for the loop's scheduled prompt to get the message.
const LOOP: Duration = Duration::from_secs(200);

/// How long a held message is watched before it goes again with the token.
const HELD: Duration = Duration::from_secs(5);

/// The mod's plugin directory, in the repository.
fn plugin() -> Result<PathBuf> {
    Ok(fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../mod"),
    )?)
}

/// The stub's script of a `kind` session of `wake`: the busy one runs the shell command; the loop
/// schedules its prompt.
fn script(wake: Wake, kind: Kind) -> &'static [Call] {
    match (wake, kind) {
        (Wake::Loop, _) => &[Call::Cron],
        (_, Kind::Idle) => &[],
        (_, Kind::Busy) => &[Call::Shell],
    }
}

/// A wake path's live check: an idle session, then a second message to it; with `--all`, also
/// the other of two idle sessions, a busy session and, for the socket, a bypass session.
pub fn wake(wake: Wake, options: &Options) -> Result<bool> {
    let (work, hook, path) = start_with(Agent::Claude, HOOK_BIN)?;
    let bins = WakeBinaries {
        hook,
        plugin: plugin()?,
        inbox: binary(&std::env::current_exe()?, INBOX_BIN)?,
    };
    let prepare = |name: &str, kind: Kind| -> Result<Prepared> {
        let files = Files::new(&work.path().join(name));
        let args = wake::session(&files, wake, kind, &bins, options.model)?;
        let stub = options.stub(script(wake, kind))?;
        let env = Env::of(
            &files,
            stub.as_ref(),
            &path,
            stub_environment,
            extra_environment(options.real),
        )?;
        Prepared::new(name, Agent::Claude, false, files, args, env, stub)
    };
    let mut check = WakeCheck::new(wake);

    // Idle: once session a registered, a message comes, then another once that turn ended; with
    // --all, session b runs at the same time and gets nothing.
    let b = (options.all && wake != Wake::Loop)
        .then(|| prepare("b", Kind::Idle))
        .transpose()?;
    let a = prepare("a", Kind::Idle)?;
    let (session_a, session_b) = side_by_side(
        || -> Result<(PtySession, Option<Added>, Option<Added>)> {
            let mut a = a.start()?;
            let first = arrive(&mut a, wake, Kind::Idle)?;
            let second = match &first {
                Some(_) if options.all && wake != Wake::Loop => arrive(&mut a, wake, Kind::Idle)?,
                _ => None,
            };
            Ok((a.end()?, first, second))
        },
        b.map(|b| {
            move || -> Result<(PtySession, bool)> {
                let mut b = b.start()?;
                let registered = registered(&mut b)?;
                b.pump_for(Duration::from_secs(20))?;
                Ok((b.end()?, registered))
            }
        }),
    );
    let (session_a, first, second) = session_a?;
    let sent = |added: &Option<Added>| added.as_ref().map(Added::now).transpose();
    let first_sent = sent(&first)?;
    println!(
        "{}",
        check.received("idle session", &session_a, first_sent.as_ref(), false, "")
    );
    if options.all && wake != Wake::Loop {
        let second = sent(&second)?;
        println!(
            "{}",
            check.received(
                "next message, after the first turn",
                &session_a,
                second.as_ref(),
                false,
                ""
            )
        );
    }
    if let Some(session_b) = session_b {
        let (session_b, registered) = session_b?;
        let token_a = first_sent
            .as_ref()
            .map_or("", |sent: &Sent| sent.token.as_str());
        println!("{}", check.other(&session_b, registered, token_a));
    }

    if options.all && wake != Wake::Loop {
        // Busy: the message comes once the agent starts the shell command.
        let mut c = prepare("c", Kind::Busy)?.start()?;
        let added = arrive(&mut c, wake, Kind::Busy)?;
        let badge = c.showed("1 Skald message(s) waiting");
        let session_c = c.end()?;
        let extra = if wake == Wake::Mod {
            format!(
                ", the badge {}",
                if badge { "showed" } else { "did not show" }
            )
        } else {
            String::new()
        };
        let sent_c = sent(&added)?;
        println!(
            "{}",
            check.received("busy session", &session_c, sent_c.as_ref(), true, &extra)
        );
        if wake == Wake::Socket {
            println!(
                "{}",
                check.bypass(&bypass(work.path(), &bins, options, &path)?)
            );
        }
    }

    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// Waits until the session's `SessionStart` hook registered it; whether it did.
fn registered(live: &mut Live) -> Result<bool> {
    let registration = live.source().dir().join("session.json");
    live.wait(TURN, |_| registration.exists())
}

/// Makes a check's message arrive in the session by `wake`: once the session registered and, in
/// the busy case, the agent started the shell command, adds it; the socket path then hands it over
/// through the socket. Then runs the session until the token reached the model and the turn
/// ended. `None` if the session never got that far.
fn arrive(live: &mut Live, wake: Wake, kind: Kind) -> Result<Option<Added>> {
    if !registered(live)? {
        return Ok(None);
    }
    if kind == Kind::Busy && !live.wait(TURN, PtySession::shell_started)? {
        return Ok(None);
    }
    // The loop's first turn schedules the prompt; the message comes after it.
    if wake == Wake::Loop && !live.wait(TURN, turn_ended)? {
        return Ok(None);
    }
    let turns = live.session().replies().len();
    let added = live.add(&token())?;
    if wake == Wake::Socket {
        hand_over(live.source(), true, FROM)?;
    }
    live.watch(added.token());
    // A scheduled prompt runs within about a minute and a half.
    let limit = if wake == Wake::Loop { LOOP } else { TURN };
    live.wait(limit, |s| {
        s.got(added.token()) && turn_ended(s) && s.replies().len() > turns
    })?;
    Ok(Some(added))
}

/// The socket's bypass session: a message without the token, from a reply address, then the same
/// message with the token.
fn bypass(work: &Path, bins: &WakeBinaries, options: &Options, path: &str) -> Result<Bypass> {
    let files = Files::new(&work.join("s"));
    let args = wake::bypass_session(&files, bins, options.model)?;
    let stub = options.stub(&[])?;
    let env = Env::of(
        &files,
        stub.as_ref(),
        path,
        |home, url, path| Agent::Claude.environment(home, url, path),
        Vec::new(),
    )?;
    let mut command = Agent::Claude.command();
    env.apply(&mut command);
    let mut child = command
        .args(&args)
        .current_dir(&files.project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(fs::File::create(files.dir.join("session.stderr"))?)
        .spawn()?;
    let mut output = Output {
        lines: read_lines(child.stdout.take().expect("stdout is piped")),
        log: fs::File::create(files.dir.join("session.jsonl"))?,
        replies: String::new(),
        notice: false,
    };
    let source = Source::open(&files.source)?;
    let mut outcome = Bypass::default();
    let registration = files.source.join("session.json");
    output.read(Instant::now() + TURN, |_| registration.exists())?;
    if let Some(socket) = source.registration()?.and_then(|r| r.socket) {
        let added = channel::add_message(&source, &token())?;
        let receipts = Receipts::bind(Path::new(&socket))?;
        hand_over(&source, false, &receipts.address())?;
        receipts.collect(&source, Instant::now() + HELD, |_| {})?;
        outcome.receipt = source
            .records(&added.msg_id)?
            .first()
            .and_then(|record| record.signal.strip_prefix("socket-").map(String::from));
        let token = token_of(&added.text);
        let reached = |replies: &str| match &stub {
            Some(stub) => stub.log().saw(&token),
            None => replies.contains(&token),
        };
        output.read(Instant::now() + Duration::from_secs(1), |_| false)?;
        outcome.reached_held = reached(&output.replies);
        source.requeue(std::slice::from_ref(&added.msg_id))?;
        hand_over(&source, true, FROM)?;
        outcome.reached_with_token = output.read(Instant::now() + TURN, reached)?;
    }
    outcome.notice = output.notice;
    drop(child.stdin.take());
    let ended = Instant::now() + Duration::from_secs(10);
    while child.try_wait()?.is_none() && Instant::now() < ended {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    child.wait()?;
    let log = stub
        .as_ref()
        .map(|stub| stub.saved_log(&files.dir.join("stub.json")))
        .transpose()?;
    outcome.model_use = model_use(log.as_ref(), None);
    Ok(outcome)
}

/// A stream-json session's output as it is read: logged, with the replies and whether it told
/// the host that it held a message.
struct Output {
    lines: std::sync::mpsc::Receiver<(String, u64)>,
    log: fs::File,
    replies: String,
    notice: bool,
}

impl Output {
    /// Reads lines until `done` holds for the replies so far, or until `until`, or the end of the
    /// output; whether `done` held.
    fn read(&mut self, until: Instant, done: impl Fn(&str) -> bool) -> Result<bool> {
        loop {
            if done(&self.replies) {
                return Ok(true);
            }
            let left = until.saturating_duration_since(Instant::now());
            match self
                .lines
                .recv_timeout(left.min(Duration::from_millis(100)))
            {
                Ok((line, _)) => {
                    writeln!(self.log, "{line}")?;
                    self.notice |= held_notice(&line);
                    for event in Agent::Claude.events(&line) {
                        if let agent_live::Event::Reply(text) = event {
                            self.replies.push_str(&text);
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) if Instant::now() < until => {}
                Err(_) => return Ok(false),
            }
        }
    }
}

/// The check token in a check message's `text`.
fn token_of(text: &str) -> String {
    text.split_whitespace()
        .find(|word| word.starts_with("tok-"))
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_token_of_a_check_message() {
        assert_eq!(
            token_of("The check token is tok-14a6985c7745. Reply with only the check token."),
            "tok-14a6985c7745"
        );
        assert_eq!(token_of("none"), "");
    }

    #[test]
    fn finds_the_mod_in_the_repository() {
        assert!(
            plugin()
                .unwrap()
                .join(".claude-plugin/plugin.json")
                .is_file()
        );
        assert_eq!(script(Wake::Socket, Kind::Idle), []);
        assert_eq!(script(Wake::Mod, Kind::Busy), [Call::Shell]);
        assert_eq!(script(Wake::Loop, Kind::Idle), [Call::Cron]);
    }
}
