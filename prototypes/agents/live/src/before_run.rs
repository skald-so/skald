//! Runs the Claude Code live checks before implementation: a signal and a request from the
//! person's own agent over the cross-session socket, and the executor sessions. Everything that
//! runs no CLI is in the library's `before` module.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use agent_inbox::CLAIMS;
use agent_live::before::{
    self, Before, BeforeCheck, Folder, FullRun, Loaded, Mode, PARENT_VAR, Reached, ReadOnlyRun,
    SIGNAL,
};
use agent_live::channel::{Files, extra_environment, stub_environment};
use agent_live::hook::HOOK_BIN;
use agent_live::{Agent, Call, INBOX_BIN, Options, Run, Session, Stub, binary, token};
use agent_source::{Source, State};
use serde_json::json;
use tempfile::TempDir;

use crate::Result;
use crate::pty::{Env, Prepared, TURN, turn_ended};
use crate::run::{finish, run_session, start, start_with};

/// Runs the check `check` with `options`; whether it passed.
pub fn run(check: Before, options: &Options) -> Result<bool> {
    match check {
        Before::Signal | Before::OwnText => socket(check, options),
        Before::ReadOnly => read_only(options),
        Before::Full => full(options),
        Before::Clean => clean(options),
    }
}

/// The signal's script on the stub: `inbox` and `claim` once the signal starts a turn; the shell
/// command the first task asks for; `inbox` and `claim` again once the second signal comes.
const SIGNAL_SCRIPT: &[Call] = &[
    Call::Inbox,
    Call::Claim,
    Call::Shell,
    Call::Inbox,
    Call::Claim,
];

/// The socket checks: a session that registers its socket; for the signal, the signal once it
/// waits idle and again while it runs the shell command the first task asks for; for the own
/// text, the own agent's request.
fn socket(check: Before, options: &Options) -> Result<bool> {
    let (work, hook, path) = start_with(Agent::Claude, HOOK_BIN)?;
    let inbox = binary(&std::env::current_exe()?, INBOX_BIN)?;
    let files = Files::new(&work.path().join("a"));
    let claims = files.dir.join("claims");
    let args = before::socket_session(&files, check, &hook, &inbox, &claims, options.model)?;
    let own = token();
    let script = match check {
        Before::Signal => SIGNAL_SCRIPT,
        _ => Call::script(vec![Call::tool(
            "Bash",
            &json!({ "command": format!("touch {own}.txt") }),
        )]),
    };
    let stub = options.stub(script)?;
    let env = Env::of(
        &files,
        stub.as_ref(),
        &path,
        stub_environment,
        extra_environment(options.real),
    )?;
    let project = files.project.clone();
    let mut live =
        Prepared::new("a", Agent::Claude, false, files.clone(), args, env, stub)?.start()?;
    let registration = files.source.join("session.json");
    let registered = live.wait(TURN, |_| registration.exists())?;
    let mut result = BeforeCheck::new(check);

    if check == Before::OwnText {
        let made = before::own_file(&project, &own);
        if registered {
            send(
                live.source(),
                &before::own_request(&own),
                agent_socket::FROM,
            )?;
            live.wait(TURN, |s| made.exists() && turn_ended(s))?;
        }
        let session = live.end()?;
        println!("{}", result.own_text(made.exists(), &session.summary()));
    } else {
        // Idle: the signal comes once the session registered.
        let idle = if registered {
            let added = live.add(&token())?;
            live.watch(added.token());
            send(live.source(), SIGNAL, &before::signal_from(1))?;
            live.wait(TURN, |s| s.shell_started() || turn_ended(s))?;
            Some(added)
        } else {
            None
        };
        // Busy: the signal comes while the shell command the first task asks for runs.
        let busy = if live.session().shell_started() {
            let added = live.add(&token())?;
            live.watch(added.token());
            send(live.source(), SIGNAL, &before::signal_from(2))?;
            live.wait(TURN, |s| s.got(added.token()) && turn_ended(s))?;
            Some(added)
        } else {
            None
        };
        let dropped = live.showed(before::DROPPED);
        let session = live.end()?;
        let signalled = |n: usize| match &session.stub {
            Some(stub) => before::signals_seen(stub) >= n,
            None => !dropped,
        };
        let tools = before::tools_started(&session);
        let (idle_tools, busy_tools) = before::idle_and_busy(&tools);
        let claimed = fs::read_to_string(claims.join(CLAIMS))
            .unwrap_or_default()
            .lines()
            .count();
        let details = format!("claim calls: {claimed}; {}", session.summary());
        let case = |added: &Option<agent_live::channel::Sent>| {
            added.as_ref().map_or((false, false), |sent| {
                (
                    matches!(sent.state, Some(State::HandedOver { .. })),
                    session.got(&sent.token),
                )
            })
        };
        let idle = idle.as_ref().map(|added| added.now()).transpose()?;
        let busy = busy.as_ref().map(|added| added.now()).transpose()?;
        let (handed, reached) = case(&idle);
        println!(
            "{}",
            result.signal(
                "idle session",
                idle.as_ref().map(|_| idle_tools.as_slice()),
                signalled(1),
                handed,
                reached,
                &details
            )
        );
        let (handed, reached) = case(&busy);
        println!(
            "{}",
            result.signal(
                "busy session",
                busy.as_ref().and(busy_tools.as_deref()),
                signalled(2),
                handed,
                reached,
                &details
            )
        );
    }
    let verdict = result.verdict(options);
    Ok(finish(work, options, &verdict, result.pass()))
}

/// Writes `text` from `from` into the session `source` registered, over its socket, with its
/// token.
fn send(source: &Source, text: &str, from: &str) -> Result<()> {
    let registration = agent_socket::registered(source)?;
    let socket = registration
        .socket
        .as_deref()
        .ok_or("the registered session has no messaging socket")?;
    let mut stream = UnixStream::connect(socket)?;
    stream.write_all(before::socket_lines(&registration, text, from).as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    Ok(())
}

/// A local HTTP server that counts the requests it gets.
struct LocalServer {
    url: String,
    requests: Arc<AtomicUsize>,
}

impl LocalServer {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let url = format!("http://{}/", listener.local_addr()?);
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&requests);
        thread::spawn(move || {
            for mut stream in listener.incoming().map_while(std::result::Result::ok) {
                counted.fetch_add(1, Ordering::SeqCst);
                let _ = stream.read(&mut [0; 4096]);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            }
        });
        Ok(Self { url, requests })
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// Runs the executor session `name` in `folder` with `args`, on `stub` if given, in a clean
/// environment, with auto-memory off if `clean`; what it did and its stream-json output.
fn execute(
    work: &TempDir,
    name: &str,
    folder: &Path,
    args: Vec<String>,
    stub: Option<&Stub>,
    clean: bool,
) -> Result<(Session, String)> {
    let dir = work.path().join(name);
    let stub_env = stub
        .map(|stub| {
            Agent::Claude.environment(
                &dir.join("home"),
                &stub.url(),
                &std::env::var("PATH").unwrap_or_default(),
            )
        })
        .transpose()?;
    let run = Run {
        name: name.to_owned(),
        agent: Agent::Claude,
        project: folder.to_owned(),
        source: dir.join("source"),
        args,
        env: Some(before::executor_environment(
            std::env::vars(),
            stub_env,
            clean,
        )),
        log: dir.join("session.jsonl"),
        stderr: dir.join("session.stderr"),
    };
    let session = run_session(&run, stub, |_| {})?;
    let log = fs::read_to_string(&run.log)?;
    Ok((session, log))
}

/// Whether `token` reached the model of a session: in a request to its stub, or in its output,
/// which holds the tool results and replies.
fn reached(session: &Session, log: &str, token: &str) -> bool {
    session.stub.as_ref().is_some_and(|stub| stub.saw(token)) || log.contains(token)
}

/// The read-only executor: told to write a file, run a command, fetch from a local server, and
/// read a file outside its folder, directly and through a symlink in it.
fn read_only(options: &Options) -> Result<bool> {
    let work = start(Agent::Claude)?;
    let folder = Folder::make(&work.path().join("read-only"))?;
    let server = LocalServer::start()?;
    let stub = options.stub(folder.script(Mode::ReadOnly, &server.url))?;
    let prompt = folder.prompt(Mode::ReadOnly, &server.url);
    let args = before::executor_args(Mode::ReadOnly, &prompt, true, options.model);
    let (session, log) = execute(
        &work,
        "read-only",
        &folder.folder,
        args,
        stub.as_ref(),
        true,
    )?;
    let run = ReadOnlyRun {
        wrote: folder.written().exists(),
        ran: folder.ran().exists(),
        fetched: server.requests(),
        secret: reached(&session, &log, &folder.secret),
        linked: reached(&session, &log, &folder.linked),
        tools: before::tool_calls(&log),
    };
    let mut check = BeforeCheck::new(Before::ReadOnly);
    println!(
        "{}",
        check.read_only(&run, &before::executor_details(&session))
    );
    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// The full executor: told to write a file and run a command in its folder.
fn full(options: &Options) -> Result<bool> {
    let work = start(Agent::Claude)?;
    let folder = Folder::make(&work.path().join("full"))?;
    let stub = options.stub(folder.script(Mode::Full, ""))?;
    let prompt = folder.prompt(Mode::Full, "");
    let args = before::executor_args(Mode::Full, &prompt, true, options.model);
    let (session, log) = execute(&work, "full", &folder.folder, args, stub.as_ref(), true)?;
    let run = FullRun {
        wrote: fs::read_to_string(folder.written()).is_ok_and(|text| text.trim() == "written"),
        ran: folder.ran().exists(),
        tools: before::tool_calls(&log),
    };
    let mut check = BeforeCheck::new(Before::Full);
    println!("{}", check.full(&run, &before::executor_details(&session)));
    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// The clean executor: with [`PARENT_VAR`] set in this process's environment and, on the stub,
/// an MCP server, hooks, a plugin, a skill, instructions and the project's memory planted in its
/// config home, it runs a command that prints the variable. On the stub, a control session
/// without the clean flags first shows that what is planted loads.
fn clean(options: &Options) -> Result<bool> {
    let parent = token();
    // SAFETY: no other thread of this process runs yet.
    unsafe { std::env::set_var(PARENT_VAR, &parent) };
    let work = start(Agent::Claude)?;
    let command = format!("echo \"parent: ${{{PARENT_VAR}:-unset}}\"");
    let prompt = format!("Run the shell command `{command}` and reply with its output.");
    let script = Call::script(vec![Call::tool("Bash", &json!({ "command": command }))]);
    let mut check = BeforeCheck::new(Before::Clean);
    let sessions: &[(&str, bool)] = if options.real {
        &[("clean", true)]
    } else {
        &[("control", false), ("clean", true)]
    };
    for &(name, clean) in sessions {
        let dir = work.path().join(name);
        let folder = Folder::make(&dir)?;
        let stub = options.stub(script)?;
        let planted = match &stub {
            Some(_) => Some(before::plant(
                &dir.join("home/.claude"),
                &folder.folder,
                &dir.join("marks"),
            )?),
            None => None,
        };
        let args = before::executor_args(Mode::Full, &prompt, clean, options.model);
        let (session, log) = execute(&work, name, &folder.folder, args, stub.as_ref(), clean)?;
        let loaded = Loaded::read(&log);
        let reached = Reached::of(planted.as_ref(), session.stub.as_ref(), &log, &parent);
        let details = before::executor_details(&session);
        println!(
            "{}",
            if clean {
                check.clean(&reached, &loaded, &details)
            } else {
                check.control(&reached, &loaded, &details)
            }
        );
    }
    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}
