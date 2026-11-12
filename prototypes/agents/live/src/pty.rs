//! Runs the live checks whose sessions run in a pseudo-terminal: Channels, as Claude Code loads a
//! channel only in an interactive session, and `codex queue`, as a queued message reaches only a
//! session that keeps running after its turn. The app-server check runs the terminal UI of a
//! session a person works in with it. Everything that runs no CLI is in the library's `channel` and
//! `queue` modules.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use agent_live::channel::{
    CHANNEL_BIN, Case, ChannelCheck, Files, HookEvent, IN_A_ROW, LISTENING, PtySession, Screen,
    Sent, TERM, add_message, extra_environment, hook_events, session, stub_environment,
};
use agent_live::queue::{self, QUEUE_BIN, QueueCheck, thread};
use agent_live::{Agent, Kind, Options, Stub, binary, token};
use agent_source::{Source, now_ms};

use crate::Result;
use crate::run::{finish, side_by_side, start, start_with};

/// The longest wait for a turn.
const TURN: Duration = Duration::from_secs(120);

/// How long the session without the channel flag runs.
const QUIET: Duration = Duration::from_secs(20);

/// How long a read of the terminal waits.
const TICK: Duration = Duration::from_millis(100);

/// The Channels path's live check: an idle session; with `--all`, also messages in a row, the
/// other of two idle sessions, a busy session and a session without the channel flag.
pub fn channel(options: &Options) -> Result<bool> {
    let work = start(Agent::Claude)?;
    let prepare = Prepare {
        work: work.path(),
        channel: &binary(&std::env::current_exe()?, CHANNEL_BIN)?,
        options,
        path: &std::env::var("PATH").unwrap_or_default(),
    };
    let mut check = ChannelCheck::default();

    // Idle: a message waits for session a; with --all, session b, with its own server and source,
    // runs at the same time, and a gets messages in a row after its first turn.
    let (a, added_a) = prepare.waiting("a", Case::Idle)?;
    let b = if options.all {
        Some(prepare.waiting("b", Case::Idle)?)
    } else {
        None
    };
    let (session_a, session_b) = side_by_side(
        || -> Result<(PtySession, Vec<Added>)> {
            let mut a = a.start()?;
            a.watch(added_a.token());
            a.wait(TURN, |s| s.got(added_a.token()) && turn_ended(s))?;
            let mut row = Vec::new();
            if options.all {
                for _ in 0..IN_A_ROW {
                    let added = a.add(&token())?;
                    a.watch(added.token());
                    row.push(added);
                }
                a.wait(TURN, |s| {
                    row.iter().all(|added| s.got(added.token())) && turn_ended(s)
                })?;
            }
            Ok((a.end()?, row))
        },
        b.map(|(b, added_b)| {
            move || -> Result<(PtySession, Added)> {
                let mut b = b.start()?;
                b.watch(added_b.token());
                b.wait(TURN, |s| s.got(added_b.token()) && turn_ended(s))?;
                Ok((b.end()?, added_b))
            }
        }),
    );
    let (session_a, row) = session_a?;
    println!("{}", check.idle(&session_a, &added_a.now()?));
    if options.all {
        let row = row.iter().map(Added::now).collect::<Result<Vec<_>>>()?;
        println!("{}", check.in_a_row(&session_a, &row));
    }
    if let Some(session_b) = session_b {
        let (session_b, added_b) = session_b?;
        println!(
            "{}",
            check.other(&session_b, &added_b.now()?, &session_a, added_a.token())
        );
    }

    if options.all {
        // Busy: the message is added when the agent starts the shell command.
        let mut c = prepare.prepared("c", Case::Busy)?.start()?;
        c.wait(TURN, |s| s.shell_started() || turn_ended(s))?;
        let added_c = if c.session.shell_started() {
            let added = c.add(&token())?;
            c.watch(added.token());
            // It may reach the model in a later turn.
            c.wait(TURN, |s| s.got(added.token()) && turn_ended(s))?;
            Some(added)
        } else {
            None
        };
        let session_c = c.end()?;
        let sent_c = added_c.as_ref().map(Added::now).transpose()?;
        println!("{}", check.busy(&session_c, sent_c.as_ref()));

        // Without the channel flag, the server's events go nowhere.
        let (d, added_d) = prepare.waiting("d", Case::Unloaded)?;
        let mut d = d.start()?;
        d.watch(added_d.token());
        d.wait(QUIET, |_| false)?;
        let session_d = d.end()?;
        println!("{}", check.unloaded(&session_d, &added_d.now()?));
    }

    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// The `codex queue` path's live check: an idle session, once its first turn ends; with `--all`,
/// also the other of two idle sessions and a busy session.
pub fn queue(options: &Options) -> Result<bool> {
    let (work, agent_queue, path) = start_with(Agent::Codex, QUEUE_BIN)?;
    let prepare = |name: &str, kind: Kind| -> Result<Prepared> {
        let files = Files::new(&work.path().join(name));
        fs::create_dir_all(&files.project)?;
        let stub = options.stub(queue::script(kind))?;
        let url = stub.as_ref().map(Stub::url);
        let args = queue::session(&files, kind, options.model, url.as_deref());
        let term = vec![("TERM".to_owned(), TERM.to_owned())];
        let env = Env::of(&files, stub.as_ref(), &path, queue::stub_environment, term)?;
        Prepared::new(name, Agent::Codex, false, files, args, env, stub)
    };
    let mut check = QueueCheck::default();

    // Idle: once session a's first turn ends, a message is queued for it; with --all, session b
    // runs at the same time and gets nothing.
    let a = prepare("a", Kind::Idle)?;
    let b = if options.all {
        Some(prepare("b", Kind::Idle)?)
    } else {
        None
    };
    let (session_a, session_b) = side_by_side(
        || -> Result<(PtySession, Option<Added>)> {
            let mut a = a.start()?;
            a.wait(TURN, turn_ended)?;
            let added = a.queue(&agent_queue)?;
            if let Some(added) = &added {
                a.watch(added.token());
                a.wait(TURN, |s| answered(s, added.token()))?;
            }
            Ok((a.end()?, added))
        },
        b.map(|b| {
            move || -> Result<PtySession> {
                let mut b = b.start()?;
                b.wait(TURN, turn_ended)?;
                b.wait(QUIET, |_| false)?;
                b.end()
            }
        }),
    );
    let (session_a, added_a) = session_a?;
    let sent_a = added_a.as_ref().map(Added::now).transpose()?;
    println!("{}", check.idle(&session_a, sent_a.as_ref()));
    if let Some(session_b) = session_b {
        let token_a = added_a.as_ref().map_or("", Added::token);
        println!("{}", check.other(&session_b?, token_a));
    }

    // Busy: the message is queued when the agent starts the shell command.
    if options.all {
        let mut c = prepare("c", Kind::Busy)?.start()?;
        c.wait(TURN, |s| s.shell_started() || turn_ended(s))?;
        let added = if c.session.shell_started() {
            c.queue(&agent_queue)?
        } else {
            None
        };
        if let Some(added) = &added {
            c.watch(added.token());
            c.wait(TURN, |s| answered(s, added.token()))?;
        }
        let session_c = c.end()?;
        let sent_c = added.as_ref().map(Added::now).transpose()?;
        println!("{}", check.busy(&session_c, sent_c.as_ref()));
    }

    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// Whether `token` reached the model and the turn it came in ended: a queued message comes in a
/// turn after the first.
fn answered(session: &PtySession, token: &str) -> bool {
    session.got(token) && session.replies().len() >= 2 && turn_ended(session)
}

/// Whether the session's last turn ended: the agent waits for a prompt.
pub(crate) fn turn_ended(session: &PtySession) -> bool {
    matches!(session.hooks.last(), Some(HookEvent::Stopped(_)))
}

/// What every session of the check shares.
struct Prepare<'a> {
    work: &'a Path,
    channel: &'a Path,
    options: &'a Options<'a>,
    path: &'a str,
}

impl Prepare<'_> {
    /// The `case` session `name`, ready to start, on its own stub unless the model is real.
    fn prepared(&self, name: &str, case: Case) -> Result<Prepared> {
        let files = Files::new(&self.work.join(name));
        let args = session(&files, self.channel, case, self.options.model)?;
        let stub = self.options.stub(case.script())?;
        let env = Env::of(
            &files,
            stub.as_ref(),
            self.path,
            stub_environment,
            extra_environment(true),
        )?;
        Prepared::new(name, Agent::Claude, true, files, args, env, stub)
    }

    /// The `case` session `name` with a check's message waiting.
    fn waiting(&self, name: &str, case: Case) -> Result<(Prepared, Added)> {
        let prepared = self.prepared(name, case)?;
        let added = prepared.add(&token())?;
        Ok((prepared, added))
    }
}

/// The CLI's environment: only these variables, or the check's own, but for what
/// [`Agent::command`] drops, and these.
pub(crate) enum Env {
    Only(Vec<(String, String)>),
    Add(Vec<(String, String)>),
}

impl Env {
    /// The environment of the session in `files`: on `stub`, only the variables `on_stub` gives
    /// for a home of its own, the stub's address and `path`; on the real model, the check's own and
    /// `real`.
    pub(crate) fn of(
        files: &Files,
        stub: Option<&Stub>,
        path: &str,
        on_stub: impl FnOnce(&Path, &str, &str) -> io::Result<Vec<(String, String)>>,
        real: Vec<(String, String)>,
    ) -> io::Result<Self> {
        Ok(match stub {
            Some(stub) => Env::Only(on_stub(&files.dir.join("home"), &stub.url(), path)?),
            None => Env::Add(real),
        })
    }

    /// Gives `command` this environment.
    pub(crate) fn apply(&self, command: &mut Command) {
        match self {
            Env::Only(env) => {
                command.env_clear().envs(env.iter().map(|(n, v)| (n, v)));
            }
            Env::Add(env) => {
                command.envs(env.iter().map(|(n, v)| (n, v)));
            }
        }
    }
}

pub(crate) struct Prepared {
    name: String,
    agent: Agent,
    /// Whether the session belongs to the Channels check: the screen tells whether its channel
    /// loaded.
    channel: bool,
    files: Files,
    args: Vec<String>,
    env: Env,
    stub: Option<Stub>,
    source: Source,
}

/// A check's message and where it is.
#[derive(Debug, Clone)]
pub(crate) struct Added {
    sent: Sent,
    dir: PathBuf,
    msg_id: String,
}

impl Added {
    /// Adds a check's message with `token` to `source`, in `dir`.
    pub(crate) fn new(source: &Source, dir: &Path, token: &str) -> Result<Self> {
        let message = add_message(source, token)?;
        Ok(Self {
            sent: Sent {
                token: token.to_owned(),
                added_at_ms: message.added_at_ms,
                state: None,
            },
            dir: dir.to_owned(),
            msg_id: message.msg_id,
        })
    }

    pub(crate) fn token(&self) -> &str {
        &self.sent.token
    }

    /// The message with its state now.
    pub(crate) fn now(&self) -> Result<Sent> {
        let source = Source::open(&self.dir)?;
        Ok(Sent {
            state: source.state(&self.msg_id)?,
            ..self.sent.clone()
        })
    }
}

impl Prepared {
    /// The session `name` of `agent` in `files`, ready to start with `args` in `env`, on `stub`
    /// unless the model is real; `channel` if it belongs to the Channels check.
    pub(crate) fn new(
        name: &str,
        agent: Agent,
        channel: bool,
        files: Files,
        args: Vec<String>,
        env: Env,
        stub: Option<Stub>,
    ) -> Result<Self> {
        let source = Source::open(&files.source)?;
        Ok(Self {
            name: name.to_owned(),
            agent,
            channel,
            files,
            args,
            env,
            stub,
            source,
        })
    }

    /// Adds a check's message with `token` to the session's source.
    fn add(&self, token: &str) -> Result<Added> {
        Added::new(&self.source, &self.files.source, token)
    }

    /// Starts the CLI in a pseudo-terminal.
    pub(crate) fn start(self) -> Result<Live> {
        let (master, slave) = open_pty()?;
        let mut command = self.agent.command();
        self.env.apply(&mut command);
        command
            .args(&self.args)
            .current_dir(&self.files.project)
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        // SAFETY: setsid and ioctl are async-signal-safe; the child makes the terminal its own.
        unsafe {
            command.pre_exec(|| {
                #[allow(
                    clippy::useless_conversion,
                    reason = "TIOCSCTTY is a u32 on macOS and the u64 ioctl takes on Linux"
                )]
                let request = libc::TIOCSCTTY.into();
                if libc::setsid() < 0 || libc::ioctl(0, request, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        drop(command);
        let mut reader = File::from(master.try_clone()?);
        let (chunks, output) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 || chunks.send(buffer[..read].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut session = PtySession::new(now_ms());
        session.channel_listening = self.channel.then_some(false);
        Ok(Live {
            log: File::create(&self.files.screen)?,
            writer: File::from(master),
            session,
            screen: Screen::default(),
            tokens: Vec::new(),
            child,
            output,
            prepared: self,
        })
    }
}

/// A running session.
pub(crate) struct Live {
    prepared: Prepared,
    child: Child,
    writer: File,
    output: Receiver<Vec<u8>>,
    screen: Screen,
    log: File,
    session: PtySession,
    /// The tokens whose arrival at the model is timed.
    tokens: Vec<String>,
}

impl Live {
    pub(crate) fn watch(&mut self, token: &str) {
        self.tokens.push(token.to_owned());
    }

    /// Adds a check's message with `token` to the session's source.
    pub(crate) fn add(&self, token: &str) -> Result<Added> {
        self.prepared.add(token)
    }

    /// What the session did so far.
    pub(crate) fn session(&self) -> &PtySession {
        &self.session
    }

    /// The session's source.
    pub(crate) fn source(&self) -> &Source {
        &self.prepared.source
    }

    /// Adds a check's message and queues it for the session with `agent_queue`, in the session's
    /// environment, so that `codex queue` finds the session's config home. `None` if no hook told
    /// the session's thread yet.
    fn queue(&self, agent_queue: &Path) -> Result<Option<Added>> {
        let hooks = fs::read_to_string(&self.prepared.files.hooks).unwrap_or_default();
        let Some(thread) = thread(&hooks) else {
            eprintln!(
                "live-check: session {} has no thread yet; see {}",
                self.prepared.name,
                self.prepared.files.screen.display()
            );
            return Ok(None);
        };
        let added = self.add(&token())?;
        let mut command = self.prepared.agent.command_for(agent_queue);
        self.prepared.env.apply(&mut command);
        let output = command
            .arg(&self.prepared.files.source)
            .arg(&thread)
            .stdin(Stdio::null())
            .output()?;
        if !output.status.success() {
            eprintln!(
                "live-check: session {}: {}",
                self.prepared.name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(Some(added))
    }

    /// Runs the session until `done` holds or `limit` passes; whether `done` held.
    pub(crate) fn wait(
        &mut self,
        limit: Duration,
        done: impl Fn(&PtySession) -> bool,
    ) -> Result<bool> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            let alive = self.pump()?;
            if done(&self.session) {
                return Ok(true);
            }
            if !alive {
                eprintln!(
                    "live-check: session {} ended early; see {}",
                    self.prepared.name,
                    self.prepared.files.screen.display()
                );
                return Ok(false);
            }
        }
        // The quiet session waits on purpose.
        if limit != QUIET {
            eprintln!(
                "live-check: session {} timed out after {} s",
                self.prepared.name,
                limit.as_secs()
            );
        }
        Ok(false)
    }

    /// Reads the terminal for `limit`, or until the CLI exits; whether it still runs.
    pub(crate) fn pump_for(&mut self, limit: Duration) -> Result<bool> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if !self.pump()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Whether the terminal showed `text`, white space aside.
    pub(crate) fn showed(&self, text: &str) -> bool {
        self.screen.showed(text)
    }

    /// Reads the terminal for up to a tick, answers its dialogs and reads what the session did;
    /// whether the CLI still runs.
    fn pump(&mut self) -> Result<bool> {
        let mut alive = true;
        match self.output.recv_timeout(TICK) {
            Ok(chunk) => {
                self.log.write_all(&chunk)?;
                if let Some((_, keys)) = self.screen.feed(&chunk) {
                    // The terminal UI reads a key after it draws the dialog.
                    thread::sleep(Duration::from_millis(300));
                    self.writer.write_all(keys)?;
                }
                while let Ok(chunk) = self.output.try_recv() {
                    self.log.write_all(&chunk)?;
                    if let Some((_, keys)) = self.screen.feed(&chunk) {
                        thread::sleep(Duration::from_millis(300));
                        self.writer.write_all(keys)?;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => alive = false,
        }
        self.refresh()?;
        Ok(alive && self.child.try_wait()?.is_none())
    }

    fn refresh(&mut self) -> Result<()> {
        if self.session.channel_listening == Some(false) {
            self.session.channel_listening = Some(self.screen.showed(LISTENING));
        }
        if let Ok(text) = fs::read_to_string(&self.prepared.files.hooks) {
            self.session.hooks = hook_events(&text);
        }
        self.session.stub = self.prepared.stub.as_ref().map(Stub::log);
        let tokens: Vec<&str> = self.tokens.iter().map(String::as_str).collect();
        self.session.note(&tokens, now_ms());
        Ok(())
    }

    /// Ends the session as a person does, with Ctrl-C twice, or kills it.
    pub(crate) fn end(mut self) -> Result<PtySession> {
        for _ in 0..2 {
            let _ = self.writer.write_all(b"\x03");
            let pause = Instant::now() + Duration::from_millis(500);
            while Instant::now() < pause {
                self.pump()?;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.child.try_wait()?.is_none() {
            if Instant::now() > deadline {
                self.child.kill()?;
                self.child.wait()?;
                break;
            }
            self.pump()?;
        }
        self.refresh()?;
        if let Some(stub) = &self.session.stub {
            stub.save(&self.prepared.files.dir.join("stub.json"))?;
        }
        // Codex's terminal UI prints the tokens it used when it exits.
        let screen = fs::read(&self.prepared.files.screen)?;
        self.session.usage = queue::usage(&String::from_utf8_lossy(&screen));
        Ok(self.session)
    }
}

/// A pseudo-terminal of 200 columns and 50 rows: its controlling and its terminal side.
fn open_pty() -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut master, mut slave) = (0, 0);
    let mut size = libc::winsize {
        ws_row: 50,
        ws_col: 200,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: the pointers are valid for the call; openpty fills in two new descriptors.
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            // A pointer, not a reference: openpty takes `*mut` on macOS and `*const` on Linux.
            &raw mut size,
        )
    };
    if opened != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty succeeded, so both descriptors are open and owned by nothing else.
    Ok(unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) })
}
