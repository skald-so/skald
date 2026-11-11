//! Runs the Channels live check: Claude Code in a pseudo-terminal, which it needs to load a
//! channel. Everything that runs no CLI is in the library's `channel` module.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use agent_live::channel::{
    CHANNEL_BIN, Case, ChannelCheck, ChannelSession, Files, HookEvent, IN_A_ROW, LISTENING, Screen,
    Sent, add_message, extra_environment, hook_events, session, stub_environment,
};
use agent_live::{Agent, Options, Stub, binary, token};
use agent_source::{Source, now_ms};

use crate::Result;
use crate::run::{finish, side_by_side, start};

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
        || -> Result<(ChannelSession, Vec<Added>)> {
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
            move || -> Result<(ChannelSession, Added)> {
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

/// Whether the session's last turn ended: the agent waits for a prompt.
fn turn_ended(session: &ChannelSession) -> bool {
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
        let env = match &stub {
            Some(stub) => Env::Only(stub_environment(
                &files.dir.join("home"),
                &stub.url(),
                self.path,
            )?),
            None => Env::Add(extra_environment(true)),
        };
        let source = Source::open(&files.source)?;
        Ok(Prepared {
            name: name.to_owned(),
            files,
            args,
            env,
            stub,
            source,
        })
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
enum Env {
    Only(Vec<(String, String)>),
    Add(Vec<(String, String)>),
}

struct Prepared {
    name: String,
    files: Files,
    args: Vec<String>,
    env: Env,
    stub: Option<Stub>,
    source: Source,
}

/// A check's message and where it is.
#[derive(Debug, Clone)]
struct Added {
    sent: Sent,
    dir: PathBuf,
    msg_id: String,
}

impl Added {
    /// Adds a check's message with `token` to `source`, in `dir`.
    fn new(source: &Source, dir: &Path, token: &str) -> Result<Self> {
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

    fn token(&self) -> &str {
        &self.sent.token
    }

    /// The message with its state now.
    fn now(&self) -> Result<Sent> {
        let source = Source::open(&self.dir)?;
        Ok(Sent {
            state: source.state(&self.msg_id)?,
            ..self.sent.clone()
        })
    }
}

impl Prepared {
    /// Adds a check's message with `token` to the session's source.
    fn add(&self, token: &str) -> Result<Added> {
        Added::new(&self.source, &self.files.source, token)
    }

    /// Starts Claude Code in a pseudo-terminal.
    fn start(self) -> Result<Live> {
        let (master, slave) = open_pty()?;
        let mut command = Agent::Claude.command();
        match &self.env {
            Env::Only(env) => {
                command.env_clear().envs(env.iter().map(|(n, v)| (n, v)));
            }
            Env::Add(env) => {
                command.envs(env.iter().map(|(n, v)| (n, v)));
            }
        }
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
        Ok(Live {
            log: File::create(&self.files.screen)?,
            writer: File::from(master),
            session: ChannelSession::new(now_ms()),
            screen: Screen::default(),
            tokens: Vec::new(),
            child,
            output,
            prepared: self,
        })
    }
}

/// A running session.
struct Live {
    prepared: Prepared,
    child: Child,
    writer: File,
    output: Receiver<Vec<u8>>,
    screen: Screen,
    log: File,
    session: ChannelSession,
    /// The tokens whose arrival at the model is timed.
    tokens: Vec<String>,
}

impl Live {
    fn watch(&mut self, token: &str) {
        self.tokens.push(token.to_owned());
    }

    /// Adds a check's message with `token` to the session's source.
    fn add(&self, token: &str) -> Result<Added> {
        self.prepared.add(token)
    }

    /// Runs the session until `done` holds or `limit` passes; whether `done` held.
    fn wait(&mut self, limit: Duration, done: impl Fn(&ChannelSession) -> bool) -> Result<bool> {
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
        if !self.session.channel_listening {
            self.session.channel_listening = self.screen.showed(LISTENING);
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
    fn end(mut self) -> Result<ChannelSession> {
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
