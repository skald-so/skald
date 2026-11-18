//! The helper (`docs/dev/architecture.md`, "Components"): one process per local state that holds
//! the device's connection ([`crate::mailbox`]) and the local state's lock while it runs, and runs
//! the operations of the commands that change the local state, which reach it over a socket in the
//! state directory.
//!
//! - The first command that connects as the device starts it in the background, detached from the
//!   command's terminal, unless it runs in one-shot mode. Any such command goes through a helper
//!   that runs; one whose helper cannot start runs here, holding the lock itself.
//! - On the socket, the command and the helper first say their versions. A helper older than the
//!   command exits once it has closed its socket, and the command goes on as if none ran. A command
//!   older than the helper fails, naming both versions, and leaves it running; an MCP server's tool
//!   call goes through it ([`route_into`]).
//! - A command hands its operation and working directory over; the helper runs it there, and the
//!   command shows its output and exits with its exit code. A write of the operation's output
//!   returns once the command has written it, or fails as the command's write did, so the
//!   operation marks nothing shown that the command did not show.
//! - A `skald recv --wait` is a session of the helper ([`crate::wait`]), which ends its sessions
//!   when it exits.
//! - The helper exits after an idle time with no command or session, after sign-out and when a
//!   newer command or a sign-in asks it to; a helper that is killed leaves its socket, which the
//!   next helper replaces.
//! - The socket's directory is its owner's only; on Windows it has the access rights it inherits
//!   from the state directory.

use std::cmp::Ordering;
use std::fs;
use std::io::{self, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
pub use std::os::unix::net::UnixStream as Stream;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use serde::{Deserialize, Serialize};
#[cfg(windows)]
pub use uds_windows::UnixStream as Stream;
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

use crate::lock::{self, Lock, Taken};
use crate::output;
use crate::wait::{self, Ran, Session, Sessions};
use crate::{Command, Operation, connection, mailbox, state};

/// How long the helper waits idle, with no command, before it exits.
pub const IDLE: Duration = Duration::from_secs(10 * 60);

/// The setting of the version a run acts as, for tests; otherwise it is the version it was built
/// as. A helper a command starts acts as the command's version.
const VERSION_SETTING: &str = "SKALD_VERSION";

/// The socket's directory in the state directory, and the socket in it.
const DIR: &str = "helper";
const SOCKET: &str = "sock";

/// How often a waiting command or helper looks again, and the helper for a command.
const RETRY: Duration = Duration::from_millis(50);

/// How long the helper waits for what a connection says before it drops it: a command says it
/// at once, and a helper that only looks for another says nothing.
const GREETING: Duration = Duration::from_secs(5);

/// How many times a command looks for a helper again after one it reached went away.
const ROUNDS: usize = 5;

/// The largest message on the socket.
const LIMIT: usize = 64 << 20;

/// What a message on the socket is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A version: the command's first, then the helper's.
    Hello = 1,
    /// An operation for the helper to run ([`Request`]).
    Run = 2,
    /// Asks the helper to exit.
    Exit = 3,
    /// Output of the operation, to standard output or standard error, which the command answers
    /// with `Written` once it has written it, or `Unwritten` with why it could not.
    Out = 4,
    Err = 5,
    /// The operation's exit code, its last message.
    Code = 6,
    /// The helper has closed its socket and exits.
    Bye = 7,
    Written = 8,
    Unwritten = 9,
}

impl Kind {
    fn of(byte: u8) -> Option<Self> {
        [
            Kind::Hello,
            Kind::Run,
            Kind::Exit,
            Kind::Out,
            Kind::Err,
            Kind::Code,
            Kind::Bye,
            Kind::Written,
            Kind::Unwritten,
        ]
        .into_iter()
        .find(|kind| *kind as u8 == byte)
    }
}

/// What a command hands over: its working directory and its operation.
#[derive(Serialize)]
struct Handed<'o> {
    cwd: PathBuf,
    operation: &'o Operation,
}

/// What the helper takes: [`Handed`].
#[derive(Deserialize)]
struct Request {
    cwd: PathBuf,
    operation: Operation,
}

/// Where a command's operation runs.
pub enum Route {
    /// In the command's own process, which holds the lock if there is a state to lock.
    Here(Option<Lock>),
    /// In the helper, which ran it, giving the exit code.
    Helped(ExitCode),
}

/// The version this run acts as.
pub fn version() -> String {
    std::env::var(VERSION_SETTING)
        .ok()
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned())
}

/// What takes the output of an operation a helper runs, on the command's side: each write, to
/// standard output or standard error, which it gives back whether it wrote.
pub type Sink<'s> = dyn FnMut(output::Stream, &[u8]) -> io::Result<()> + 's;

/// Where `operation`, of a command that changes the state in `dir`, runs: through the helper that
/// runs, or one it starts if `starts` and there is a state; otherwise here, once it holds the lock,
/// waiting for it as [`lock::take`] does, with a missing `dir` created if `create`, unless a helper
/// starts meanwhile. What a helper's run of it shows goes to standard output and standard error.
pub fn route(
    dir: &Path,
    operation: &Operation,
    create: bool,
    starts: bool,
) -> Result<Route, String> {
    route_into(dir, operation, (create, starts, false), &mut shown)
}

/// Writes `bytes` to the standard output or standard error, as `stream` says.
fn shown(stream: output::Stream, bytes: &[u8]) -> io::Result<()> {
    match stream {
        output::Stream::Out => {
            let mut out = io::stdout().lock();
            out.write_all(bytes).and_then(|()| out.flush())
        }
        output::Stream::Err => io::stderr().lock().write_all(bytes),
    }
}

/// As [`route`], what a helper's run of `operation` shows going to `sink`; through a helper newer
/// than this run too if `newer`, as an MCP server's tool call goes ([`crate::mcp`]).
pub fn route_into(
    dir: &Path,
    operation: &Operation,
    (create, starts, newer): (bool, bool, bool),
    sink: &mut Sink,
) -> Result<Route, String> {
    let dir = std::path::absolute(dir)
        .map_err(|err| format!("cannot use the state directory {}: {err}", dir.display()))?;
    let timeout = operation.timeout;
    let mut start = starts && dir.join(state::FILE).is_file();
    let mut started: Option<(Child, Instant)> = None;
    for _ in 0..ROUNDS {
        loop {
            if let Some(stream) = connect(&dir) {
                if let Some((stream, helper)) = greet(stream, timeout)? {
                    match compare(&version(), &helper) {
                        Ordering::Equal => return hand(stream, operation, sink).map(Route::Helped),
                        Ordering::Less if newer => {
                            return hand(stream, operation, sink).map(Route::Helped);
                        }
                        Ordering::Greater => dismiss(stream, timeout),
                        Ordering::Less => return Err(older(&version(), &helper)),
                    }
                }
                break;
            }
            match started.as_mut() {
                Some((child, since)) => {
                    let exited = !matches!(child.try_wait(), Ok(None));
                    if exited || since.elapsed() >= timeout {
                        // It gave way to another helper, or cannot start: the run goes on here.
                        started = None;
                        start = false;
                        break;
                    }
                    thread::sleep(RETRY);
                }
                None if start => match spawn(&dir, timeout) {
                    Ok(child) => started = Some((child, Instant::now())),
                    Err(_) => {
                        start = false;
                        break;
                    }
                },
                None => break,
            }
        }
        if started.is_some() || start {
            continue;
        }
        match lock::take_unless(&dir, create, timeout, || connect(&dir).is_some())? {
            Taken::Lock(lock) => return Ok(Route::Here(lock)),
            Taken::GaveWay => {}
        }
    }
    Err("the helper of the local state keeps going away; run the command again".to_owned())
}

/// Asks the helper of the state in `dir`, if one runs, to exit, so a sign-in takes the lock; fails
/// for a helper newer than this run.
pub fn stop(dir: &Path, timeout: Duration) -> Result<(), String> {
    let Some(stream) = connect(dir) else {
        return Ok(());
    };
    let Some((stream, helper)) = greet(stream, timeout)? else {
        return Ok(());
    };
    if compare(&version(), &helper) == Ordering::Less {
        return Err(older(&version(), &helper));
    }
    dismiss(stream, timeout);
    Ok(())
}

/// Whether a helper of the state in `dir` listens.
pub fn running(dir: &Path) -> bool {
    connect(dir).is_some()
}

/// The helper's socket in the state directory `dir`.
fn socket(dir: &Path) -> PathBuf {
    dir.join(DIR).join(SOCKET)
}

/// A connection to the helper of the state in `dir`, if one listens.
fn connect(dir: &Path) -> Option<UnixStream> {
    UnixStream::connect(socket(dir)).ok()
}

/// Says this run's version on `stream` and gives the helper's, waiting for it up to `timeout`
/// while the helper runs another command's operation; `None` if the helper went away.
fn greet(
    mut stream: UnixStream,
    timeout: Duration,
) -> Result<Option<(UnixStream, String)>, String> {
    let said = stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| send(&mut stream, Kind::Hello, version().as_bytes()));
    if said.is_err() {
        return Ok(None);
    }
    match receive(&mut stream) {
        Ok(Some((Kind::Hello, helper))) => Ok(Some((
            stream,
            String::from_utf8_lossy(&helper).into_owned(),
        ))),
        Err(err) if connection::timed_out(&err) => Err(format!(
            "the helper of the local state is busy with another command: gave up waiting for it \
             after {} seconds; nothing changed",
            timeout.as_secs()
        )),
        _ => Ok(None),
    }
}

/// Asks the helper on `stream` to exit, and waits up to `timeout` until it has closed its socket.
fn dismiss(mut stream: UnixStream, timeout: Duration) {
    if send(&mut stream, Kind::Exit, &[]).is_ok() {
        let _ = stream.set_read_timeout(Some(timeout));
        // Its goodbye, or the end of the connection.
        let _ = receive(&mut stream);
    }
}

/// Hands `operation` over on `stream` and gives `sink` what the helper's run of it shows, saying
/// whether each write of it worked; gives its exit code.
fn hand(
    mut stream: UnixStream,
    operation: &Operation,
    sink: &mut Sink,
) -> Result<ExitCode, String> {
    let cwd = std::env::current_dir()
        .map_err(|err| format!("cannot read the working directory: {err}"))?;
    let request = serde_json::to_vec(&Handed { cwd, operation })
        .map_err(|err| format!("cannot hand the command to the helper: {err}"))?;
    let gone = "the helper of the local state exited before the command finished; run the \
                command again";
    stream
        .set_read_timeout(None)
        .and_then(|()| send(&mut stream, Kind::Run, &request))
        .map_err(|_| gone.to_owned())?;
    loop {
        let written = match receive(&mut stream) {
            Ok(Some((Kind::Out, bytes))) => sink(output::Stream::Out, &bytes),
            Ok(Some((Kind::Err, bytes))) => sink(output::Stream::Err, &bytes),
            Ok(Some((Kind::Code, code))) => {
                return Ok(ExitCode::from(code.first().copied().unwrap_or(1)));
            }
            _ => return Err(gone.to_owned()),
        };
        // A helper gone meanwhile shows as the end of the stream.
        let _ = match written {
            Ok(()) => send(&mut stream, Kind::Written, &[]),
            Err(err) => send(&mut stream, Kind::Unwritten, err.to_string().as_bytes()),
        };
    }
}

/// How the version `command` compares with the helper's, `helper`: one that is not a semantic
/// version is older than any other.
fn compare(command: &str, helper: &str) -> Ordering {
    if command == helper {
        return Ordering::Equal;
    }
    match (
        semver::Version::parse(command),
        semver::Version::parse(helper),
    ) {
        (Ok(command), Ok(helper)) => command.cmp(&helper),
        (Ok(_), Err(_)) => Ordering::Greater,
        _ => Ordering::Less,
    }
}

/// Why a command of the version `command` does not use the helper of the newer version `helper`:
/// to run it again, when the `skald` on disk is not older than the helper, as when the binary was
/// replaced while the command ran; otherwise, to update that `skald`.
fn older(command: &str, helper: &str) -> String {
    let said = format!(
        "this skald {command} is older than the helper of the local state, skald {helper}, which \
         keeps running"
    );
    match on_disk() {
        Some(disk) if compare(&disk, helper) != Ordering::Less => {
            format!("{said}; skald {disk} is installed now: run the command again")
        }
        Some(disk) => format!("{said}; the installed skald {disk} is older too: update skald"),
        None => format!("{said}; update skald"),
    }
}

/// The version of the `skald` installed where this run's binary is, as it says.
fn on_disk() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    // Linux names a replaced binary so.
    let exe = match exe
        .to_str()
        .and_then(|path| path.strip_suffix(" (deleted)"))
    {
        Some(path) => PathBuf::from(path),
        None => exe,
    };
    let said = std::process::Command::new(exe)
        .arg("--version")
        .env_remove(VERSION_SETTING)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(said.stdout).ok()?;
    let version = text.split_whitespace().nth(1)?;
    semver::Version::parse(version).ok()?;
    Some(version.to_owned())
}

/// Starts the helper of the state in `dir`, detached from this run's terminal and outputs, as this
/// run's version, waiting for the lock up to `timeout`.
fn spawn(dir: &Path, timeout: Duration) -> io::Result<Child> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("--state-dir")
        .arg(dir)
        .arg("--timeout")
        .arg(timeout.as_secs().max(1).to_string());
    if let Some(retry) = wait::retry_set() {
        command
            .arg("--websocket-retry")
            .arg(retry.as_secs().to_string());
    }
    command
        .arg("helper")
        .env(VERSION_SETTING, version())
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe; it only starts a session without a terminal.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        windows::keep_own_handles();
    }
    command.spawn()
}

/// Runs the helper of the state in `dir`, exiting after `idle` with no command; waits for the lock
/// up to `timeout`, and exits at once when another helper of the state answers.
pub fn serve(dir: &Path, idle: Duration, timeout: Duration) -> Result<(), String> {
    let dir = std::path::absolute(dir)
        .map_err(|err| format!("cannot use the state directory {}: {err}", dir.display()))?;
    let place = dir.join(DIR);
    owners_only(&place).map_err(|err| {
        format!(
            "cannot use the helper's directory {}: {err}",
            place.display()
        )
    })?;
    let _lock = match lock::take_unless(&dir, false, timeout, || connect(&dir).is_some())? {
        Taken::Lock(Some(lock)) => lock,
        Taken::Lock(None) => {
            return Err(format!(
                "no local state in {}: sign in first",
                dir.display()
            ));
        }
        Taken::GaveWay => return Ok(()),
    };
    let path = socket(&dir);
    // A socket a killed helper left.
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path)
        .map_err(|err| format!("cannot listen on {}: {err}", path.display()))?;
    let bound = Bound(path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bound.0, fs::Permissions::from_mode(0o600))
            .map_err(|err| format!("cannot protect {}: {err}", bound.0.display()))?;
    }
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("cannot listen on {}: {err}", bound.0.display()))?;
    let _ = std::env::set_current_dir(&dir);
    let mut last = Instant::now();
    let mut sessions = Sessions::default();
    let parting = loop {
        match listener.accept() {
            Ok((stream, _)) => match serve_one(stream, &dir, &mut sessions) {
                Served::Nothing => {}
                Served::Ran => {
                    last = Instant::now();
                    sessions.ran();
                }
                Served::Joined => last = Instant::now(),
                Served::Exit(stream) => break stream,
            },
            // None waits, or a failure to accept one, which the next look may not meet.
            Err(_) => {
                match sessions.look(&dir) {
                    Some(true) => last = Instant::now(),
                    Some(false) if last.elapsed() >= idle => break None,
                    Some(false) => {}
                    None => break None,
                }
                thread::sleep(RETRY);
            }
        }
    };
    // Its sessions end as a dead helper's do.
    drop(sessions);
    drop(listener);
    drop(bound);
    if let Some(mut stream) = parting {
        let _ = send(&mut stream, Kind::Bye, &[]);
    }
    mailbox::close_kept();
    Ok(())
}

/// The socket the helper bound, removed on drop.
struct Bound(PathBuf);

impl Drop for Bound {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// What came of a connection to the helper.
enum Served {
    /// Nothing ran: a helper looking for another, or a command that went away.
    Nothing,
    /// An operation ran.
    Ran,
    /// A session joined, its first run done.
    Joined,
    /// The helper exits; it says goodbye on the stream, if any, once its socket is closed.
    Exit(Option<UnixStream>),
}

/// Serves the command on `stream`, by the state in `dir`; a `skald recv --wait` joins `sessions`.
fn serve_one(mut stream: UnixStream, dir: &Path, sessions: &mut Sessions) -> Served {
    let ready = stream
        .set_nonblocking(false)
        .and_then(|()| stream.set_read_timeout(Some(GREETING)));
    if ready.is_err() {
        return Served::Nothing;
    }
    let Ok(Some((Kind::Hello, _))) = receive(&mut stream) else {
        return Served::Nothing;
    };
    if send(&mut stream, Kind::Hello, version().as_bytes()).is_err() {
        return Served::Nothing;
    }
    // A command of another version says why it does not run its operation here.
    match receive(&mut stream) {
        Ok(Some((Kind::Run, request))) => run(stream, dir, &request, sessions),
        Ok(Some((Kind::Exit, _))) => Served::Exit(Some(stream)),
        _ => Served::Nothing,
    }
}

/// Runs the operation of `request` for the command on `stream`, in its working directory, sending
/// its output and then its exit code; a `skald recv --wait` joins `sessions` instead. The helper
/// exits after sign-out, and after an operation that panicked.
fn run(stream: UnixStream, dir: &Path, request: &[u8], sessions: &mut Sessions) -> Served {
    let request: Request = match serde_json::from_slice(request) {
        Ok(request) => request,
        Err(err) => {
            let mut answer = stream;
            let said = format!("skald: the helper cannot read the command: {err}\n");
            let _ = send(&mut answer, Kind::Err, said.as_bytes());
            let _ = send(&mut answer, Kind::Code, &[1]);
            return Served::Ran;
        }
    };
    let mut operation = request.operation;
    operation.state_dir = Some(dir.to_path_buf());
    if operation.command.waits() {
        let session = Session {
            stream,
            cwd: request.cwd,
            operation,
        };
        return if sessions.start(session, dir) {
            Served::Joined
        } else {
            Served::Exit(None)
        };
    }
    let signs_out = matches!(operation.command, Command::SignOut);
    let code = match on_behalf(&stream, dir, &request.cwd, || {
        crate::perform(operation).map(|()| true)
    }) {
        Some(Outcome::Done(code)) => code,
        Some(Outcome::Waits) => unreachable!("only a session waits"),
        None => return Served::Nothing,
    };
    let mut answer = stream;
    let _ = send(&mut answer, Kind::Code, &[code]);
    if signs_out || code == PANICKED {
        Served::Exit(None)
    } else {
        Served::Ran
    }
}

/// Runs `work` for `session`'s operation, its output going to the session's command, with the
/// state in `dir`; ends the session with its exit code once `work` gave what the session waits
/// for, or failed.
pub fn run_for(
    session: Session,
    dir: &Path,
    work: impl FnOnce(&Operation) -> Result<bool, String>,
) -> Ran {
    let Session {
        stream,
        cwd,
        operation,
    } = session;
    let outcome = on_behalf(&stream, dir, &cwd, || work(&operation));
    let code = match outcome {
        Some(Outcome::Waits) => {
            return Ran::Waits(Box::new(Session {
                stream,
                cwd,
                operation,
            }));
        }
        Some(Outcome::Done(code)) => code,
        None => return Ran::Ended,
    };
    let mut answer = stream;
    let _ = send(&mut answer, Kind::Code, &[code]);
    if code == PANICKED {
        Ran::Panicked
    } else {
        Ran::Ended
    }
}

/// The exit code of an operation that panicked.
const PANICKED: u8 = 101;

/// What came of an operation the helper ran.
enum Outcome {
    /// It ended with this exit code.
    Done(u8),
    /// It waits on: a session's run that showed nothing it waits for.
    Waits,
}

/// Runs `work` in `cwd` for the command on `stream`, sending its output, then returns to `dir`:
/// `work` gives whether the operation is done, or fails with its error, which the command shows.
/// `None` if the command cannot be answered.
fn on_behalf(
    stream: &UnixStream,
    dir: &Path,
    cwd: &Path,
    work: impl FnOnce() -> Result<bool, String>,
) -> Option<Outcome> {
    // The command answers each write once its own output takes it, however long that is.
    let mut answer = stream
        .set_read_timeout(None)
        .and_then(|()| stream.try_clone())
        .ok()?;
    let mut sink = stream.try_clone().ok()?;
    let ran = output::into(
        move |stream, bytes| {
            let kind = match stream {
                output::Stream::Out => Kind::Out,
                output::Stream::Err => Kind::Err,
            };
            send(&mut sink, kind, bytes)?;
            match receive(&mut sink)? {
                Some((Kind::Written, _)) => Ok(()),
                Some((Kind::Unwritten, why)) => {
                    Err(io::Error::other(String::from_utf8_lossy(&why).into_owned()))
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the command went away",
                )),
            }
        },
        || {
            panic::catch_unwind(AssertUnwindSafe(|| {
                std::env::set_current_dir(cwd)
                    .map_err(|err| {
                        format!("cannot use the working directory {}: {err}", cwd.display())
                    })
                    .and_then(|()| work())
                    .inspect_err(|err| eprintln!("skald: {err}"))
            }))
        },
    );
    let _ = std::env::set_current_dir(dir);
    Some(match ran {
        Ok(Ok(true)) => Outcome::Done(0),
        Ok(Ok(false)) => Outcome::Waits,
        Ok(Err(_)) => Outcome::Done(1),
        Err(panicked) => {
            let why = panicked
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panicked.downcast_ref::<&str>().copied())
                .unwrap_or("a panic");
            let said = format!("skald: the helper failed: {why}\n");
            let _ = send(&mut answer, Kind::Err, said.as_bytes());
            Outcome::Done(PANICKED)
        }
    })
}

/// Creates `dir`, if missing, readable only by its owner on Unix; on Windows it has the access
/// rights it inherits from its parent.
fn owners_only(dir: &Path) -> io::Result<()> {
    state::create_dir(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Sends a message of `kind` with `payload` on `stream`: its kind, its length in 4 bytes, big-endian,
/// and the payload.
fn send(stream: &mut impl Write, kind: Kind, payload: &[u8]) -> io::Result<()> {
    let length = u32::try_from(payload.len())
        .ok()
        .filter(|length| *length as usize <= LIMIT)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a message too large"))?;
    let mut message = Vec::with_capacity(5 + payload.len());
    message.push(kind as u8);
    message.extend_from_slice(&length.to_be_bytes());
    message.extend_from_slice(payload);
    stream.write_all(&message)?;
    stream.flush()
}

/// The next message on `stream`: `None` if it ends first.
fn receive(stream: &mut impl Read) -> io::Result<Option<(Kind, Vec<u8>)>> {
    let mut head = [0; 5];
    match stream.read_exact(&mut head) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }
    let kind = Kind::of(head[0])
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "an unknown message"))?;
    let length = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if length > LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a message too large",
        ));
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload)?;
    Ok(Some((kind, payload)))
}

/// Serde for protocol versions as `<oldest>-<latest>`, which they parse from.
pub mod versions {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};
    use skald_proto::VersionRange;

    pub fn serialize<S: Serializer>(
        range: &VersionRange,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("{}-{}", range.oldest(), range.latest()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<VersionRange, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{
        HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    /// Keeps this run's standard handles from a process it starts, so a helper does not hold the
    /// outputs of the command that started it open.
    pub fn keep_own_handles() {
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: the process's own standard handles; a missing one is skipped.
            unsafe {
                let handle = GetStdHandle(which);
                if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                    SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message goes over as its kind, length and payload, and the end of a stream before one
    /// gives none.
    #[test]
    fn messages_go_over_whole() {
        let mut wire = Vec::new();
        send(&mut wire, Kind::Out, b"line\n").unwrap();
        send(&mut wire, Kind::Code, &[1]).unwrap();
        let mut read = wire.as_slice();
        assert_eq!(
            receive(&mut read).unwrap(),
            Some((Kind::Out, b"line\n".to_vec()))
        );
        assert_eq!(receive(&mut read).unwrap(), Some((Kind::Code, vec![1])));
        assert_eq!(receive(&mut read).unwrap(), None);
        assert!(receive(&mut [99, 0, 0, 0, 0].as_slice()).is_err());
    }

    /// Versions compare as semantic versions; one that is not is older than any that is.
    #[test]
    fn versions_compare_as_semantic_versions() {
        assert_eq!(compare("0.2.0", "0.2.0"), Ordering::Equal);
        assert_eq!(compare("0.10.0", "0.9.1"), Ordering::Greater);
        assert_eq!(compare("0.1.0", "0.1.1"), Ordering::Less);
        assert_eq!(compare("0.1.0", "dev"), Ordering::Greater);
        assert_eq!(compare("dev", "0.1.0"), Ordering::Less);
    }
}
