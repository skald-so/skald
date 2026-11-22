//! The stand-in `codex` command: a copy of this test binary named `codex` in a directory of its own
//! ([`StandIn`]). Started as a [`CodexProcess`], it plays a Codex process: it runs each command the
//! test gives it as its own child, as Codex runs a session's hooks, until the test ends it
//! ([`CodexApp`]). Run otherwise, as the helper runs `codex queue`, it records the run in its
//! directory, its arguments, its settings, its input and when it started, and exits with success,
//! or, once the test made it fail, with failure; it queues nothing and reaches no network.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use skald_proto::MessageId;

use crate::cloud::WAIT;
use crate::codex::{
    CODEX_HOME, Codex, CodexProcess, Event, QUIET, Thread, assert_bare, given, gives_by, play,
    plays,
};
use crate::hooks::Hooked;

/// The stand-in's file name.
const NAME: &str = "codex";

/// The file in the stand-in's directory that holds its runs, one JSON object a line.
const RUNS: &str = "runs.jsonl";

/// The file in the stand-in's directory whose presence makes its runs fail.
const FAIL: &str = "fail";

/// How long a recorded run waits for its input to end: a run whose input stays open records none.
const INPUT_WAIT: Duration = Duration::from_millis(500);

/// How soon an idle session gets its `codex queue` once its message reached the helper. The tests
/// count from when the send returned, once the server stored the message.
pub const WAKE: Duration = Duration::from_secs(1);

/// What this run of the test binary does, if it is the stand-in: plays a Codex process when started
/// as one, records itself otherwise. `None` for a run as the tests' binary.
pub fn acts() -> Option<ExitCode> {
    let exe = std::env::current_exe().ok()?;
    if exe.file_stem() != Some(OsStr::new(NAME)) {
        return None;
    }
    if plays() {
        play();
        return Some(ExitCode::SUCCESS);
    }
    Some(record(&exe))
}

/// Records this run in [`RUNS`] beside `exe`; fails if [`FAIL`] is there.
fn record(exe: &Path) -> ExitCode {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    let (send, input) = mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let _ = io::stdin().read_to_string(&mut text);
        let _ = send.send(text);
    });
    let input = input.recv_timeout(INPUT_WAIT).ok();
    let dir = exe.parent().unwrap_or(Path::new("."));
    let failing = dir.join(FAIL).exists();
    let settings: Map<String, Value> = std::env::vars_os()
        .map(|(name, value)| (lossy(&name), json!(lossy(&value))))
        .collect();
    let line = json!({
        "args": std::env::args().skip(1).collect::<Vec<_>>(),
        "settings": settings,
        "input": input,
        "exe": lossy(exe.as_os_str()),
        "started_ms": u64::try_from(started).unwrap_or(u64::MAX),
        "failed": failing,
    });
    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(RUNS))
        .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()));
    if written.is_err() || failing {
        eprintln!("codex: error: the stand-in fails, as the test asked");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn lossy(text: &OsStr) -> String {
    text.to_string_lossy().into_owned()
}

/// A stand-in `codex` in a directory of its own.
pub struct StandIn {
    exe: PathBuf,
}

impl StandIn {
    /// Copies this test binary to `dir`, which it makes, as `codex`.
    pub fn new(dir: &Path) -> Self {
        fs::create_dir_all(dir).expect("make the stand-in's directory");
        let exe = dir.join(format!("{NAME}{}", std::env::consts::EXE_SUFFIX));
        let this = std::env::current_exe().expect("this test binary");
        fs::copy(this, &exe).expect("copy the stand-in codex");
        StandIn { exe }
    }

    /// The stand-in for the `codex` command Ada's machine finds on its search path: the machine's
    /// `bin/`, which these tests' runs search alone.
    pub fn on_path(codex: &Codex) -> Self {
        StandIn::new(&codex.machine().root().join("bin"))
    }

    /// A stand-in as the `codex` of the app `app` on Ada's machine.
    pub fn app(codex: &Codex, app: &str) -> Self {
        StandIn::new(&codex.machine().root().join("apps").join(app))
    }

    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// From now on, its runs fail.
    pub fn fail(&self) {
        let dir = self.exe.parent().expect("the stand-in's directory");
        fs::write(dir.join(FAIL), "").expect("make the stand-in fail");
    }

    /// Its runs so far, in order.
    pub fn runs(&self) -> Vec<Ran> {
        let dir = self.exe.parent().expect("the stand-in's directory");
        let text = fs::read_to_string(dir.join(RUNS)).unwrap_or_default();
        (text.lines())
            .map(|line| Ran(serde_json::from_str(line).expect("a recorded run")))
            .collect()
    }

    /// Waits for up to [`WAIT`] until it has run `count` times in all; fails if it ran more or
    /// fewer. Gives its runs.
    #[track_caller]
    pub fn await_runs(&self, count: usize) -> Vec<Ran> {
        let deadline = Instant::now() + WAIT;
        loop {
            let runs = self.runs();
            assert!(
                runs.len() <= count,
                "{} runs, not {count}: {runs:#?}",
                runs.len()
            );
            if runs.len() == count {
                return runs;
            }
            assert!(
                Instant::now() < deadline,
                "{} runs within {WAIT:?}, not {count}: {runs:#?}",
                runs.len()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits for up to [`WAIT`] until it has run; fails if it did not. Gives its runs.
    #[track_caller]
    pub fn await_some(&self) -> Vec<Ran> {
        let deadline = Instant::now() + WAIT;
        loop {
            let runs = self.runs();
            if !runs.is_empty() {
                return runs;
            }
            assert!(Instant::now() < deadline, "no run within {WAIT:?}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Fails unless it has run `count` times in all.
    #[track_caller]
    pub fn assert_runs(&self, count: usize, what: &str) {
        let runs = self.runs();
        assert_eq!(runs.len(), count, "{what}: {runs:#?}");
    }

    /// Starts it as a Codex process.
    pub fn start(&self) -> CodexApp {
        CodexApp(CodexProcess::start_as(&self.exe))
    }
}

/// Waits [`QUIET`], then fails unless each of `stand_ins` has run as many times as it gives.
#[track_caller]
pub fn assert_quiet(stand_ins: &[(&StandIn, usize, &str)]) {
    thread::sleep(QUIET);
    for (stand_in, count, what) in stand_ins {
        stand_in.assert_runs(*count, what);
    }
}

/// A run of the stand-in, as it recorded it.
#[derive(Debug, Clone)]
pub struct Ran(Value);

impl Ran {
    /// Its arguments.
    pub fn args(&self) -> Vec<String> {
        (self.0["args"].as_array().into_iter().flatten())
            .map(|arg| arg.as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// The value of its option `flag`, given as `<flag> <value>` or `<flag>=<value>`.
    pub fn option(&self, flag: &str) -> Option<String> {
        let args = self.args();
        let joined = format!("{flag}=");
        (args.iter().enumerate()).find_map(|(at, arg)| {
            if arg == flag {
                args.get(at + 1).cloned()
            } else {
                arg.strip_prefix(&joined).map(str::to_owned)
            }
        })
    }

    /// Its setting `name`, if set.
    pub fn setting(&self, name: &str) -> Option<String> {
        self.0["settings"][name].as_str().map(str::to_owned)
    }

    /// The names and values of its settings.
    pub fn settings(&self) -> Vec<(String, String)> {
        (self.0["settings"].as_object().into_iter().flatten())
            .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_owned()))
            .collect()
    }

    /// Its input, if it ended within [`INPUT_WAIT`].
    pub fn input(&self) -> Option<String> {
        self.0["input"].as_str().map(str::to_owned)
    }

    /// The executable that ran.
    pub fn exe(&self) -> PathBuf {
        PathBuf::from(self.0["exe"].as_str().unwrap_or_default())
    }

    /// When it started.
    pub fn started(&self) -> SystemTime {
        let ms = self.0["started_ms"].as_u64().unwrap_or_default();
        UNIX_EPOCH + Duration::from_millis(ms)
    }

    /// Whether it failed, as the test asked.
    pub fn failed(&self) -> bool {
        self.0["failed"].as_bool().unwrap_or_default()
    }
}

/// The same file, however its path was written.
fn same_file(one: &Path, other: &Path) -> bool {
    match (fs::canonicalize(one), fs::canonicalize(other)) {
        (Ok(one), Ok(other)) => one == other,
        _ => one == other,
    }
}

/// Fails unless `ran` is `codex queue` for `of`'s thread, run by `codex`, the `codex` of the process
/// that runs `of`, with `of`'s config home in [`CODEX_HOME`], and its message is the signal alone,
/// holding none of `texts`, `msg_ids` and `names` ([`assert_bare`]); and unless no other argument,
/// no setting and not its input holds any of `texts`, nor its first 40 characters. Gives the
/// message.
#[track_caller]
pub fn assert_queued(
    ran: &Ran,
    (of, codex): (&Thread, &StandIn),
    texts: &[&str],
    msg_ids: &[MessageId],
    names: &[&str],
) -> String {
    let args = ran.args();
    assert_eq!(
        args.first().map(String::as_str),
        Some("queue"),
        "not codex queue: {args:?}"
    );
    assert_eq!(
        ran.option("--thread"),
        Some(of.id.clone()),
        "another thread: {args:?}"
    );
    assert!(
        same_file(&ran.exe(), codex.exe()),
        "run by {:?}, not by its process's codex {:?}",
        ran.exe(),
        codex.exe()
    );
    let home = ran
        .setting(CODEX_HOME)
        .unwrap_or_else(|| panic!("no {CODEX_HOME}"));
    assert!(
        same_file(Path::new(&home), &of.home),
        "{CODEX_HOME} is {home:?}, not the session's {:?}",
        of.home
    );
    let message = (ran.option("--message")).unwrap_or_else(|| panic!("no --message: {args:?}"));
    assert_bare(&message, texts, msg_ids, names);
    let heads: Vec<String> = (texts.iter())
        .map(|text| text.chars().take(40).collect())
        .collect();
    let holds = |value: &str| heads.iter().any(|head| value.contains(head.as_str()));
    for arg in &args {
        assert!(
            *arg == message || !holds(arg),
            "an argument holds a message's text"
        );
    }
    for (name, value) in ran.settings() {
        assert!(!holds(&value), "the setting {name} holds a message's text");
    }
    let input = ran.input().unwrap_or_default();
    assert!(!holds(&input), "its input holds a message's text");
    message
}

/// Fails unless `ran` started within [`WAKE`] of `sent`, when its message's send returned.
#[track_caller]
pub fn assert_soon(ran: &Ran, sent: SystemTime) {
    let after = ran.started().duration_since(sent).unwrap_or_default();
    assert!(
        after <= WAKE,
        "codex queue ran {after:?} after the message came, not within {WAKE:?}"
    );
}

/// When now is, to compare with when a run of the stand-in started.
pub fn now() -> SystemTime {
    SystemTime::now()
}

/// A Codex session in `agent`'s folder, `--agent <agent>`, with a config home of its own,
/// `homes/<home>/.codex` under Ada's machine's directory.
pub fn session_of(codex: &Codex, agent: &str, home: &str) -> Thread {
    let mut thread = codex.thread_of(agent);
    thread.home = (codex.machine().root().join("homes").join(home)).join(".codex");
    fs::create_dir_all(&thread.home).expect("make the session's config home");
    thread
}

/// The stand-in playing a Codex process, which runs its sessions' hooks as its children; killed
/// on drop.
pub struct CodexApp(CodexProcess);

impl CodexApp {
    /// `of`'s hook of `event`, run by this process.
    #[track_caller]
    pub fn hook(&mut self, codex: &Codex, of: &Thread, event: Event) -> Hooked {
        codex.hook_by(&mut self.0, of, event)
    }

    /// `of` starts in this process: its `SessionStart` must print nothing. The session is idle.
    #[track_caller]
    pub fn starts(&mut self, codex: &Codex, of: &Thread) {
        self.hook(codex, of, Event::SessionStart)
            .assert_silent("SessionStart");
    }

    /// Fails unless `of`'s hook of `event` prints nothing.
    #[track_caller]
    pub fn silent(&mut self, codex: &Codex, of: &Thread, event: Event, what: &str) {
        let hooked = self.hook(codex, of, event);
        hooked.assert_silent(&format!("{what}: {}", event.name()));
    }

    /// Runs `of`'s hook of `event` every half second until it gives the session something, for up
    /// to [`WAIT`]; each run before that must print nothing. Gives what it gave.
    #[track_caller]
    pub fn gives(&mut self, codex: &Codex, of: &Thread, event: Event) -> String {
        gives_by(event, || self.hook(codex, of, event))
    }

    /// A turn of `of` runs, as one a queued signal starts: its `UserPromptSubmit`, then its `Stop`,
    /// and if that continues the turn, the continued turn's `Stop`, which must let it end. Each
    /// exits with success and gives the session the signal or nothing. The session is idle after.
    #[track_caller]
    pub fn turn(&mut self, codex: &Codex, of: &Thread) {
        let started = self.hook(codex, of, Event::UserPromptSubmit);
        if started.stdout.is_empty() {
            started.assert_silent("the turn's UserPromptSubmit");
        } else {
            assert_bare(&given(&started, Event::UserPromptSubmit), &[], &[], &[]);
        }
        let stop = self.hook(codex, of, Event::Stop);
        if stop.stdout.is_empty() {
            stop.assert_silent("the turn's Stop");
        } else {
            assert_bare(&given(&stop, Event::Stop), &[], &[], &[]);
            self.silent(codex, of, Event::StopActive, "the continued turn's Stop");
        }
    }

    /// Ends the process, as Codex exits, and waits until it no longer runs.
    pub fn end(self) {
        self.0.end();
    }
}

/// web's session, `--agent web` in its folder, starts in a Codex app of its own, `cli`, with a
/// config home of its own: it is idle. Gives the app's stand-in `codex`, the session and the app.
pub fn web_in_app(codex: &Codex) -> (StandIn, Thread, CodexApp) {
    let stand_in = StandIn::app(codex, "cli");
    let web = session_of(codex, crate::solo::WEB, "web");
    let mut app = stand_in.start();
    app.starts(codex, &web);
    (stand_in, web, app)
}
