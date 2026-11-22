//! A stand-in `codex` that records each run, the device's environment and the user's config home
//! an executor starts from, and waiting for a run within a limit.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use skald_adapters::executor::Outcome;

use crate::fixtures::{Scratch, plain, token};

/// Variables of the device's environment that Codex needs neither to run nor to sign in: no
/// executor passes them on.
pub const MARKERS: [&str; 4] = [
    "SKALD_E2E_MARKER",
    "GITHUB_TOKEN",
    "ANTHROPIC_API_KEY",
    "SKALD_TOKEN",
];

/// The variables whose presence the stand-in records.
const WATCHED: [&str; 6] = [
    "SKALD_E2E_MARKER",
    "GITHUB_TOKEN",
    "ANTHROPIC_API_KEY",
    "SKALD_TOKEN",
    "PATH",
    "CODEX_HOME",
];

/// How long a run may take: the stand-in answers at once.
const RUN_LIMIT: Duration = Duration::from_secs(60);

/// What the stand-in does once it has recorded its run.
pub enum Script {
    /// Ends its turn well with this final answer, as `codex exec --json` prints it, also in the
    /// file `--output-last-message` names.
    Answer(String),
    /// Fails the turn with this message, as `codex exec --json` prints it, and exits with 1.
    TurnFails(String),
    /// Prints this message on stderr only and exits with 2, as Codex does on a bad argument.
    Exits(String),
    /// Starts a child process, then runs until it is ended; each of the two appends to a file of
    /// its own every 100 ms, `beat` and `child-beat`, and exits once the records are removed.
    Lingers,
}

/// A stand-in `codex` in a scratch directory: a copy of this test binary named `codex`, which runs
/// its [`Script`] when started, and a setup file beside it that names the script and where to
/// record.
pub struct StandIn {
    pub path: PathBuf,
    records: PathBuf,
}

impl StandIn {
    pub fn new(scratch: &Scratch, script: Script) -> Self {
        let records = scratch.dir("stand-in-records");
        let dir = scratch.dir("stand-in");
        let path = dir.join(format!("{NAME}{}", env::consts::EXE_SUFFIX));
        let this = env::current_exe().expect("this test binary");
        fs::copy(this, &path).expect("copy the stand-in codex");
        let (kind, text) = match &script {
            Script::Answer(text) => ("answer", text.as_str()),
            Script::TurnFails(text) => ("turn-fails", text.as_str()),
            Script::Exits(text) => ("exits", text.as_str()),
            Script::Lingers => ("lingers", ""),
        };
        let setup = json!({ "records": records, "script": kind, "text": text });
        fs::write(dir.join(SETUP), setup.to_string()).expect("write the stand-in's setup");
        StandIn { path, records }
    }

    /// Whether any process of it started.
    pub fn started(&self) -> bool {
        self.records.join("invocations").exists()
    }

    /// How many times it started.
    pub fn invocations(&self) -> usize {
        let text = fs::read_to_string(self.records.join("invocations")).unwrap_or_default();
        text.lines().filter(|line| !line.trim().is_empty()).count()
    }

    /// Its arguments, as it got them.
    pub fn args(&self) -> Vec<String> {
        let recorded = fs::read(self.records.join("args")).expect("no run recorded its arguments");
        let text = String::from_utf8_lossy(&recorded);
        let text = text.strip_suffix('\0').unwrap_or(&text);
        if text.is_empty() {
            return Vec::new();
        }
        text.split('\0').map(str::to_owned).collect()
    }

    /// Its arguments with each `--flag=value` split in two and quotes dropped.
    pub fn flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        for arg in self.args() {
            let arg = arg.replace(['"', '\''], "");
            match arg.split_once('=') {
                Some((flag, value)) if flag.starts_with("--") => {
                    flags.push(flag.to_owned());
                    flags.push(value.to_owned());
                }
                _ => flags.push(arg),
            }
        }
        flags
    }

    /// Everything it was given to read: its arguments, then its stdin.
    pub fn input(&self) -> String {
        let stdin = fs::read(self.records.join("stdin")).unwrap_or_default();
        format!(
            "{}\n{}",
            self.args().join("\n"),
            String::from_utf8_lossy(&stdin)
        )
    }

    /// Whether the run got the variable `name`, one of [`WATCHED`].
    pub fn got(&self, name: &str) -> bool {
        assert!(
            WATCHED.contains(&name),
            "the stand-in does not watch {name}"
        );
        self.records.join(format!("got-{name}")).exists()
    }

    /// The `CODEX_HOME` the run got, if any.
    pub fn codex_home(&self) -> Option<PathBuf> {
        let recorded = fs::read_to_string(self.records.join("codex-home")).ok()?;
        let recorded = recorded.trim();
        (!recorded.is_empty()).then(|| PathBuf::from(recorded))
    }

    /// The names of the entries in that `CODEX_HOME` while the run ran, sorted.
    pub fn codex_home_entries(&self) -> Vec<String> {
        let text = fs::read_to_string(self.records.join("codex-home-entries")).unwrap_or_default();
        let mut names: Vec<String> = (text.lines().map(str::trim))
            .filter(|name| !name.is_empty() && *name != "." && *name != "..")
            .map(str::to_owned)
            .collect();
        names.sort();
        names
    }

    /// The `auth.json` in that `CODEX_HOME` while the run ran, if any.
    pub fn codex_home_auth(&self) -> Option<String> {
        let text = fs::read_to_string(self.records.join("codex-home-auth")).ok()?;
        Some(text.trim().to_owned())
    }

    /// The size of the heartbeat file `name` of [`Script::Lingers`], 0 before its first beat.
    pub fn beats(&self, name: &str) -> u64 {
        fs::metadata(self.records.join(name)).map_or(0, |meta| meta.len())
    }
}

/// The value of `-c approval_policy`, `--sandbox` and the like in `flags`: each value that follows
/// `flag`.
pub fn values_of(flags: &[String], flag: &str) -> Vec<String> {
    let follows = flags.windows(2).filter(|pair| pair[0] == flag);
    follows.map(|pair| pair[1].clone()).collect()
}

/// The stand-in's file name.
const NAME: &str = "codex";

/// The file beside the stand-in that names its records directory and its script.
const SETUP: &str = "stand-in.json";

/// The argument that starts the stand-in as the child of [`Script::Lingers`].
const CHILD: &str = "--skald-stand-in-child";

/// What this run of the test binary does, if it is the stand-in: records its run and plays its
/// script. `None` for a run as the tests' binary.
pub fn acts() -> Option<ExitCode> {
    let exe = env::current_exe().ok()?;
    if exe.file_stem() != Some(OsStr::new(NAME)) {
        return None;
    }
    let setup = fs::read(exe.with_file_name(SETUP)).expect("the stand-in's setup");
    let setup: Value = serde_json::from_slice(&setup).expect("the stand-in's setup as JSON");
    let field = |name: &str| setup[name].as_str().expect("a setup field").to_owned();
    let records = PathBuf::from(field("records"));
    let args: Vec<String> = (env::args_os().skip(1))
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if args == [CHILD] {
        beat(&records, "child-beat");
        return Some(ExitCode::SUCCESS);
    }
    record(&records, &args);
    let (kind, text) = (field("script"), field("text"));
    let mut stdout = io::stdout().lock();
    let mut say = |line: &str| writeln!(stdout, "{line}").expect("write the stand-in's output");
    say(r#"{"type":"thread.started","thread_id":"019a0000-0000-7000-8000-000000000000"}"#);
    say(r#"{"type":"turn.started"}"#);
    match kind.as_str() {
        "answer" => {
            say(&agent_message(&text));
            say(TURN_COMPLETED);
            if let Some(out) = last_message_file(&args) {
                fs::write(out, &text).expect("write the last message");
            }
            Some(ExitCode::SUCCESS)
        }
        "turn-fails" => {
            let (error, failed) = turn_failed(&text);
            say(&error);
            say(&failed);
            eprintln!("Error: {text}");
            Some(ExitCode::from(1))
        }
        "exits" => {
            eprintln!("error: {text}");
            Some(ExitCode::from(2))
        }
        _ => {
            #[expect(
                clippy::zombie_processes,
                reason = "stopping the run ends the child with the stand-in"
            )]
            let _child = Command::new(&exe)
                .arg(CHILD)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start the stand-in's child");
            beat(&records, "beat");
            Some(ExitCode::SUCCESS)
        }
    }
}

/// Records a run with `args` in `records`: that it ran, its arguments, which of the [`WATCHED`]
/// variables it got, its `CODEX_HOME` with the names of the entries in it and its `auth.json`, and
/// its stdin unless that is a terminal.
fn record(records: &Path, args: &[String]) {
    let write = |name: &str, contents: &[u8]| {
        fs::write(records.join(name), contents).expect("record the run");
    };
    let mut invocations = (OpenOptions::new().create(true).append(true))
        .open(records.join("invocations"))
        .expect("record the run");
    invocations.write_all(b"run\n").expect("record the run");
    let joined: String = args.iter().map(|arg| format!("{arg}\0")).collect();
    write("args", joined.as_bytes());
    for name in WATCHED {
        if env::var_os(name).is_some() {
            write(&format!("got-{name}"), b"");
        }
    }
    let codex_home = env::var_os("CODEX_HOME").unwrap_or_default();
    write("codex-home", codex_home.to_string_lossy().as_bytes());
    let codex_home = Path::new(&codex_home);
    if !codex_home.as_os_str().is_empty() && codex_home.is_dir() {
        let names: Vec<String> = (fs::read_dir(codex_home).expect("list CODEX_HOME"))
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        write("codex-home-entries", names.join("\n").as_bytes());
        if let Ok(auth) = fs::read(codex_home.join("auth.json")) {
            write("codex-home-auth", &auth);
        }
    }
    if !io::stdin().is_terminal() {
        let mut stdin = Vec::new();
        io::stdin()
            .read_to_end(&mut stdin)
            .expect("read the stand-in's stdin");
        write("stdin", &stdin);
    }
}

/// The file `--output-last-message` or `-o` in `args` names, if any.
fn last_message_file(args: &[String]) -> Option<&str> {
    let mut out = None;
    for (at, arg) in args.iter().enumerate() {
        if arg == "-o" || arg == "--output-last-message" {
            out = args.get(at + 1).map(String::as_str).or(out);
        }
        if let Some(file) = arg.strip_prefix("--output-last-message=") {
            out = Some(file);
        }
    }
    out
}

/// Appends to the file `name` in `records` every 100 ms while `records` is there.
fn beat(records: &Path, name: &str) {
    while records.is_dir() {
        let beat = (OpenOptions::new().create(true).append(true)).open(records.join(name));
        let _ = beat.and_then(|mut file| file.write_all(b"x"));
        thread::sleep(Duration::from_millis(100));
    }
}

/// The event of `codex exec --json` that gives the agent's message `text`.
fn agent_message(text: &str) -> String {
    format!(
        r#"{{"type":"item.completed","item":{{"id":"item_0","type":"agent_message","text":"{text}"}}}}"#
    )
}

/// The event of `codex exec --json` that ends a turn well.
const TURN_COMPLETED: &str = r#"{"type":"turn.completed","usage":{"input_tokens":120,"cached_input_tokens":0,"output_tokens":8}}"#;

/// The events of `codex exec --json` of a turn that failed with `message`.
fn turn_failed(message: &str) -> (String, String) {
    (
        format!(r#"{{"type":"error","message":"{message}"}}"#),
        format!(r#"{{"type":"turn.failed","error":{{"message":"{message}"}}}}"#),
    )
}

/// The user's Codex config home, planted in `home`: the sign-in in `auth.json`, and an MCP server,
/// a hook, an enabled plugin with a hook and a skill, a rule, a skill and an instruction file,
/// none of which an executor may load.
pub struct ConfigHome {
    pub path: PathBuf,
    /// What `auth.json` holds.
    pub auth: String,
}

impl ConfigHome {
    pub fn plant(home: &Path) -> Self {
        fs::create_dir_all(home).expect("make the config home");
        let auth = format!(
            r#"{{"auth_mode":"apikey","OPENAI_API_KEY":"{}"}}"#,
            token("placeholder-key")
        );
        let files = [
            ("auth.json", auth.clone()),
            (
                "config.toml",
                "[mcp_servers.planted]\ncommand = \"planted-mcp\"\n\n[plugins.\"planted@local\"]\nenabled = true\n".to_owned(),
            ),
            (
                "hooks.json",
                r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"planted-hook"}]}]}}"#.to_owned(),
            ),
            ("AGENTS.md", format!("Say {}.\n", token("instructions"))),
            ("rules/planted.rules", "prefix_rule(pattern = [\"touch\"], decision = \"forbidden\")\n".to_owned()),
            ("skills/planted/SKILL.md", "---\nname: planted\ndescription: planted\n---\nPlanted.\n".to_owned()),
            (
                "plugins/planted/.codex-plugin/plugin.json",
                r#"{"name":"planted","version":"1.0.0"}"#.to_owned(),
            ),
        ];
        for (name, contents) in files {
            let path = home.join(name);
            fs::create_dir_all(path.parent().expect("a parent")).expect("make a directory");
            fs::write(path, contents).expect("plant a file");
        }
        ConfigHome {
            path: plain(home),
            auth,
        }
    }
}

/// The variables of this process that the device's environment keeps, so programs run.
const RUNNING: [&str; 9] = [
    "PATH",
    "PATHEXT",
    "COMSPEC",
    "SYSTEMDRIVE",
    "SYSTEMROOT",
    "WINDIR",
    "TEMP",
    "TMP",
    "TMPDIR",
];

/// The device's environment an executor starts from: what this process has to run programs, the
/// `home`, `CODEX_HOME` if given, and each of the [`MARKERS`] set to a token.
pub fn device_environment(home: &Path, codex_home: Option<&Path>) -> Vec<(OsString, OsString)> {
    let mut vars = Vec::new();
    for (name, value) in env::vars_os() {
        let running = name.to_str().map(str::to_ascii_uppercase);
        if running.is_some_and(|name| RUNNING.contains(&name.as_str())) {
            vars.push((name, value));
        }
    }
    vars.push(("HOME".into(), home.as_os_str().to_owned()));
    if cfg!(windows) {
        vars.push(("USERPROFILE".into(), home.as_os_str().to_owned()));
    }
    if let Some(codex_home) = codex_home {
        vars.push(("CODEX_HOME".into(), codex_home.as_os_str().to_owned()));
    }
    for name in MARKERS {
        vars.push((name.into(), token(name).into()));
    }
    vars
}

/// Waits on a thread of its own for `run` to end by `wait`; its outcome. Fails unless it ends
/// within [`RUN_LIMIT`].
#[track_caller]
pub fn finish<R, F>(run: R, wait: F) -> Outcome
where
    R: Send + 'static,
    F: FnOnce(R) -> Outcome + Send + 'static,
{
    let (done, ended) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(wait(run));
    });
    (ended.recv_timeout(RUN_LIMIT))
        .unwrap_or_else(|_| panic!("the run did not end within {RUN_LIMIT:?}"))
}
