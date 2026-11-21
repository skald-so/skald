//! Runs the Codex live checks before implementation: a signal over `codex queue` and the executor
//! sessions. Everything that runs no CLI is in the library's `codex_before` module.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use agent_live::before::{
    Before, Folder, LocalServer, Mode, SIGNAL, claim_calls, executor_details,
};
use agent_live::channel::Files;
use agent_live::codex_before::{self, CodexCheck, Executor, Reached, Setup, Signalled};
use agent_live::{Agent, INBOX_BIN, Kind, Options, Run, Session, Stub, token};
use agent_source::State;
use tempfile::TempDir;

use crate::Result;
use crate::pty::{Env, Prepared, TURN, turn_ended};
use crate::run::{finish, run_session, start, start_with};

/// Runs the check `check` with `options`; whether it passed.
pub fn run(check: Before, options: &Options) -> Result<bool> {
    match check {
        Before::Signal => signal(options),
        Before::ReadOnly => executor(Mode::ReadOnly, options),
        Before::Full => executor(Mode::Full, options),
        Before::Clean => clean(options),
        Before::OwnText => Err("own-text runs only on Claude Code".into()),
    }
}

/// The signal: a terminal UI session with `inbox` and the stub `claim`; once its first turn ends,
/// the signal goes over `codex queue`. With `--all`, also a busy session, whose signal goes once
/// the agent starts the shell command of its first turn.
fn signal(options: &Options) -> Result<bool> {
    let (work, inbox, path) = start_with(Agent::Codex, INBOX_BIN)?;
    let mut check = CodexCheck::new(Before::Signal);
    let cases = [
        ("a", Kind::Idle, "idle session"),
        ("c", Kind::Busy, "busy session"),
    ];
    for (name, kind, case) in cases.into_iter().take(if options.all { 2 } else { 1 }) {
        let files = Files::new(&work.path().join(name));
        let claims = files.dir.join("claims");
        let (script, cue) = codex_before::signal_script(kind);
        let stub = options.cued_stub(script, Some(cue))?;
        let url = stub.as_ref().map(Stub::url);
        let args = codex_before::signal_session(
            &files,
            kind,
            &inbox,
            &claims,
            options.model,
            url.as_deref(),
        )?;
        let env = match &stub {
            Some(stub) => Env::Only(agent_live::queue::stub_environment(
                &files.dir.join("home"),
                &stub.url(),
                &path,
            )?),
            None => Env::Add(signed_in(&files.dir.join("home/.codex"))?),
        };
        let mut live = Prepared::new(name, Agent::Codex, false, files, args, env, stub)?.start()?;
        let ready = match kind {
            Kind::Idle => live.wait(TURN, turn_ended)?,
            Kind::Busy => {
                live.wait(TURN, |s| s.shell_started() || turn_ended(s))?
                    && live.session().shell_started()
            }
        };
        let sent = match (ready, live.thread()) {
            (true, Some(thread)) => {
                let added = live.add(&token())?;
                live.watch(added.token());
                let queued = live.codex_queue(&thread, SIGNAL)?;
                if queued {
                    let handed = || {
                        added
                            .now()
                            .is_ok_and(|sent| matches!(sent.state, Some(State::HandedOver { .. })))
                    };
                    live.wait(TURN, |s| {
                        handed()
                            && claim_calls(&claims) > 0
                            && s.got(added.token())
                            && turn_ended(s)
                    })?;
                }
                Some((added, queued))
            }
            _ => None,
        };
        let session = live.end()?;
        let seen = sent
            .map(|(added, queued)| -> Result<Signalled> {
                Ok(Signalled {
                    queued,
                    reached: codex_before::signal_reached(session.stub.as_ref(), queued),
                    handed_over: matches!(added.now()?.state, Some(State::HandedOver { .. })),
                    claims: claim_calls(&claims),
                    token: session.got(added.token()),
                })
            })
            .transpose()?;
        println!("{}", check.signal(case, seen.as_ref(), &session.summary()));
    }
    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// Signs Codex in, in the config home `config`, with the key in [`codex_before::REAL_KEY`] on
/// stdin, and returns the variables its sessions get on top of `live-check`'s own.
fn signed_in(config: &Path) -> Result<Vec<(String, String)>> {
    let key = std::env::var(codex_before::REAL_KEY).map_err(|_| {
        format!(
            "--real signs Codex in with {}, which is not set",
            codex_before::REAL_KEY
        )
    })?;
    fs::create_dir_all(config)?;
    let env = codex_before::real_environment(config);
    let mut login = Agent::Codex
        .command()
        .envs(env.iter().map(|(name, value)| (name, value)))
        .args(codex_before::LOGIN)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    login
        .stdin
        .take()
        .ok_or("codex login has no stdin")?
        .write_all(key.as_bytes())?;
    if !login.wait()?.success() {
        return Err("codex login --with-api-key failed".into());
    }
    Ok(env)
}

/// Runs the executor session `name` in `folder` with `args` on `stub`, with the stub's environment
/// in a home of its own and, for the control, `parent`; what it did.
fn execute(
    work: &TempDir,
    name: &str,
    folder: &Path,
    args: Vec<String>,
    stub: &Stub,
    env: impl FnOnce(Vec<(String, String)>) -> Vec<(String, String)>,
) -> Result<Session> {
    let dir = work.path().join(name);
    let stub_env = Agent::Codex.environment(
        &dir.join("home"),
        &stub.url(),
        &std::env::var("PATH").unwrap_or_default(),
    )?;
    let run = Run {
        name: name.to_owned(),
        agent: Agent::Codex,
        project: folder.to_owned(),
        source: dir.join("source"),
        args,
        env: Some(env(stub_env)),
        log: dir.join("session.jsonl"),
        stderr: dir.join("session.stderr"),
    };
    run_session(&run, Some(stub), |_| {})
}

/// The model stub of an executor check, which runs only on it.
fn executor_stub(options: &Options, script: &'static [agent_live::Call]) -> Result<Stub> {
    Ok(options
        .stub(script)?
        .ok_or("the executor checks run only on the model stub")?)
}

/// An executor in `mode`, with the clean flags: its turn tries to read a file in its folder, write
/// one in it and one outside it, run a command, reach the check's server and a public host, and
/// read a file outside the folder.
fn executor(mode: Mode, options: &Options) -> Result<bool> {
    let work = start(Agent::Codex)?;
    let name = codex_before::sandbox(mode);
    let server = LocalServer::start()?;
    let executor = Executor::make(&work.path().join(name), &server.url)?;
    let stub = executor_stub(options, executor.script())?;
    let folder = &executor.folder.folder;
    let args = codex_before::executor_args(
        mode,
        folder,
        &executor.prompt(),
        Setup::Sandbox,
        options.model,
        Some(&stub.url()),
    );
    let session = execute(&work, name, folder, args, &stub, |env| env)?;
    let done = executor.outcomes(session.stub.as_ref(), server.requests());
    let mut check = CodexCheck::new(match mode {
        Mode::ReadOnly => Before::ReadOnly,
        Mode::Full => Before::Full,
    });
    println!(
        "{}",
        check.attempts(mode, &done, &executor_details(&session))
    );
    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// The clean executor: with the sign-in, an MCP server, a hook, a plugin with a skill and a hook,
/// a rule, a skill and `AGENTS.md` planted in its config home, it makes the file the rule forbids
/// and prints a variable. A control session without the clean flags, and with the variable, comes
/// first, then one with only the flags, then the clean one, which also has a config home of its
/// own that holds only the sign-in.
fn clean(options: &Options) -> Result<bool> {
    let work = start(Agent::Codex)?;
    let parent = token();
    let mut check = CodexCheck::new(Before::Clean);
    for setup in Setup::CLEAN {
        let name = setup.name();
        let dir = work.path().join(name);
        let folder = Folder::make(&dir)?;
        let config = dir.join("home/.codex");
        let planted = codex_before::plant(&config, &dir.join("marks"), &dir.join("marketplace"))?;
        let own = dir.join("own/.codex");
        codex_before::sign_in_only(&config, &own)?;
        let stub = executor_stub(options, codex_before::clean_script(&planted))?;
        let args = codex_before::executor_args(
            Mode::Full,
            &folder.folder,
            &codex_before::clean_prompt(&planted),
            setup,
            options.model,
            Some(&stub.url()),
        );
        let session = execute(&work, name, &folder.folder, args, &stub, |env| {
            codex_before::executor_environment(env, setup, &parent, &own)
        })?;
        let reached = Reached::of(&planted, session.stub.as_ref(), &parent);
        println!(
            "{}",
            check.session(setup, &reached, &executor_details(&session))
        );
    }
    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}
