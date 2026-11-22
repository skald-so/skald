//! A stand-in `codex` that records each run, the device's environment and the user's config home
//! an executor starts from, and waiting for a run within a limit.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

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

/// A stand-in `codex` in a scratch directory.
pub struct StandIn {
    pub path: PathBuf,
    records: PathBuf,
}

impl StandIn {
    pub fn new(scratch: &Scratch, script: Script) -> Self {
        let records = scratch.dir("stand-in-records");
        let dir = scratch.dir("stand-in");
        let path = if cfg!(windows) {
            write_windows(&dir, &records, &script)
        } else {
            write_unix(&dir, &records, &script)
        };
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

    /// Its arguments with each `--flag=value` split in two and quotes dropped, as Windows' batch
    /// files may keep or drop them.
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

/// The stand-in as a POSIX shell script that uses only the shell's builtins to record, so it
/// records whatever `PATH` it gets.
fn write_unix(dir: &Path, records: &Path, script: &Script) -> PathBuf {
    let rec = records.to_string_lossy().into_owned();
    let mut sh = String::from("#!/bin/sh\n");
    sh += &format!("rec='{rec}'\n");
    sh += "printf 'run\\n' >> \"$rec/invocations\"\n";
    sh += "for arg in \"$@\"; do printf '%s\\0' \"$arg\"; done > \"$rec/args\"\n";
    sh += "pwd -P > \"$rec/cwd\"\n";
    for name in WATCHED {
        sh += &format!("[ -n \"${{{name}+set}}\" ] && : > \"$rec/got-{name}\"\n");
    }
    sh += "printf '%s' \"${CODEX_HOME-}\" > \"$rec/codex-home\"\n";
    sh += "if [ -n \"${CODEX_HOME-}\" ] && [ -d \"$CODEX_HOME\" ]; then\n";
    sh += "  for entry in \"$CODEX_HOME\"/* \"$CODEX_HOME\"/.*; do\n";
    sh += "    [ -e \"$entry\" ] || [ -L \"$entry\" ] || continue\n";
    sh += "    printf '%s\\n' \"${entry##*/}\"\n";
    sh += "  done > \"$rec/codex-home-entries\"\n";
    sh += "  if [ -f \"$CODEX_HOME/auth.json\" ]; then\n";
    sh += "    while IFS= read -r line || [ -n \"$line\" ]; do printf '%s\\n' \"$line\"; done \\\n";
    sh += "      < \"$CODEX_HOME/auth.json\" > \"$rec/codex-home-auth\"\n";
    sh += "  fi\n";
    sh += "fi\n";
    sh += "if [ ! -t 0 ]; then\n";
    sh += "  while IFS= read -r line || [ -n \"$line\" ]; do printf '%s\\n' \"$line\"; done > \"$rec/stdin\"\n";
    sh += "fi\n";
    sh += "out=''\nprev=''\n";
    sh += "for arg in \"$@\"; do\n";
    sh += "  case \"$prev\" in -o|--output-last-message) out=$arg;; esac\n";
    sh += "  case \"$arg\" in --output-last-message=*) out=${arg#*=};; esac\n";
    sh += "  prev=$arg\n";
    sh += "done\n";
    sh += "printf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"019a0000-0000-7000-8000-000000000000\"}'\n";
    sh += "printf '%s\\n' '{\"type\":\"turn.started\"}'\n";
    match script {
        Script::Answer(answer) => {
            let item = agent_message(answer);
            sh += &format!("printf '%s\\n' '{item}'\n");
            sh += &format!("printf '%s\\n' '{TURN_COMPLETED}'\n");
            sh += &format!("[ -n \"$out\" ] && printf '%s' '{answer}' > \"$out\"\n");
            sh += "exit 0\n";
        }
        Script::TurnFails(message) => {
            let (error, failed) = turn_failed(message);
            sh += &format!("printf '%s\\n' '{error}' '{failed}'\n");
            sh += &format!("printf 'Error: %s\\n' '{message}' >&2\n");
            sh += "exit 1\n";
        }
        Script::Exits(message) => {
            sh += &format!("printf 'error: %s\\n' '{message}' >&2\n");
            sh += "exit 2\n";
        }
        Script::Lingers => {
            sh += "( while [ -d \"$rec\" ]; do printf x >> \"$rec/child-beat\"; sleep 0.1; done ) \\\n";
            sh += "  < /dev/null > /dev/null 2>&1 &\n";
            sh += "while [ -d \"$rec\" ]; do printf x >> \"$rec/beat\"; sleep 0.1; done\n";
        }
    }
    let path = dir.join("codex");
    write_executable(&path, &sh);
    path
}

/// What the Windows stand-in's batch file passes for a last argument `-`.
const DASH: &str = "--skald-stand-in-dash";

/// The stand-in as a batch file that runs a PowerShell script. PowerShell's `-File` takes a lone
/// `-`, the last argument of a session, for a parameter with no name and refuses it: the batch
/// file passes [`DASH`] for it, and the script turns that back into `-`.
fn write_windows(dir: &Path, records: &Path, script: &Script) -> PathBuf {
    let rec = records.to_string_lossy().replace('\'', "''");
    let mut ps = String::from("$ErrorActionPreference = 'Continue'\r\n");
    ps += &format!(
        "$argv = @($args | ForEach-Object {{ if (\"$_\" -eq '{DASH}') {{ '-' }} else {{ \"$_\" }} }})\r\n"
    );
    ps += &format!("$rec = '{rec}'\r\n");
    ps += "Add-Content -LiteralPath \"$rec\\invocations\" -Value 'run'\r\n";
    ps += "[IO.File]::WriteAllText(\"$rec\\args\", ($argv -join [char]0))\r\n";
    ps += "[IO.File]::WriteAllText(\"$rec\\cwd\", (Get-Location).Path)\r\n";
    for name in WATCHED {
        ps += &format!(
            "if (Test-Path 'env:{name}') {{ [IO.File]::WriteAllText(\"$rec\\got-{name}\", '') }}\r\n"
        );
    }
    ps += "[IO.File]::WriteAllText(\"$rec\\codex-home\", [string]$env:CODEX_HOME)\r\n";
    ps += "if ($env:CODEX_HOME -and (Test-Path -LiteralPath $env:CODEX_HOME -PathType Container)) {\r\n";
    ps += "  $names = @(Get-ChildItem -Force -LiteralPath $env:CODEX_HOME | ForEach-Object { $_.Name })\r\n";
    ps += "  [IO.File]::WriteAllText(\"$rec\\codex-home-entries\", ($names -join \"`n\"))\r\n";
    ps += "  $auth = Join-Path $env:CODEX_HOME 'auth.json'\r\n";
    ps += "  if (Test-Path -LiteralPath $auth -PathType Leaf) {\r\n";
    ps +=
        "    [IO.File]::WriteAllText(\"$rec\\codex-home-auth\", [IO.File]::ReadAllText($auth))\r\n";
    ps += "  }\r\n";
    ps += "}\r\n";
    ps += "if ([Console]::IsInputRedirected) {\r\n";
    ps += "  [IO.File]::WriteAllText(\"$rec\\stdin\", [Console]::In.ReadToEnd())\r\n";
    ps += "}\r\n";
    ps += "$out = $null\r\n";
    ps += "for ($i = 0; $i -lt $argv.Count; $i++) {\r\n";
    ps += "  if (($argv[$i] -eq '-o' -or $argv[$i] -eq '--output-last-message') -and $i + 1 -lt $argv.Count) { $out = $argv[$i + 1] }\r\n";
    ps += "  if ($argv[$i] -like '--output-last-message=*') { $out = $argv[$i].Substring(24) }\r\n";
    ps += "}\r\n";
    let line = |text: &str| {
        format!(
            "[Console]::Out.WriteLine('{}')\r\n",
            text.replace('\'', "''")
        )
    };
    ps += &line(r#"{"type":"thread.started","thread_id":"019a0000-0000-7000-8000-000000000000"}"#);
    ps += &line(r#"{"type":"turn.started"}"#);
    match script {
        Script::Answer(answer) => {
            ps += &line(&agent_message(answer));
            ps += &line(TURN_COMPLETED);
            ps += &format!("if ($out) {{ [IO.File]::WriteAllText($out, '{answer}') }}\r\n");
            ps += "exit 0\r\n";
        }
        Script::TurnFails(message) => {
            let (error, failed) = turn_failed(message);
            ps += &line(&error);
            ps += &line(&failed);
            ps += &format!("[Console]::Error.WriteLine('Error: {message}')\r\n");
            ps += "exit 1\r\n";
        }
        Script::Exits(message) => {
            ps += &format!("[Console]::Error.WriteLine('error: {message}')\r\n");
            ps += "exit 2\r\n";
        }
        Script::Lingers => {
            ps += "$child = \"while (Test-Path -LiteralPath '$rec') { Add-Content -LiteralPath '$rec\\child-beat' -Value x -NoNewline; Start-Sleep -Milliseconds 100 }\"\r\n";
            ps += "Start-Process -NoNewWindow -FilePath powershell -ArgumentList '-NoProfile', '-NonInteractive', '-Command', $child\r\n";
            ps += "while (Test-Path -LiteralPath $rec) { Add-Content -LiteralPath \"$rec\\beat\" -Value x -NoNewline; Start-Sleep -Milliseconds 100 }\r\n";
        }
    }
    fs::write(dir.join("codex.ps1"), ps).expect("write the stand-in's script");
    let script = dir.join("codex.ps1").to_string_lossy().into_owned();
    let cmd = format!(
        "@echo off\r\nsetlocal\r\nset \"args=%*\"\r\n\
         if \"%args:~-2%\"==\" -\" set \"args=%args:~0,-2% {DASH}\"\r\n\
         powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{script}\" %args%\r\n\
         exit /b %ERRORLEVEL%\r\n"
    );
    let path = dir.join("codex.cmd");
    write_executable(&path, &cmd);
    path
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

/// Writes the script `text` to `path`, which anyone may run on Unix.
fn write_executable(path: &Path, text: &str) {
    fs::write(path, text).expect("write the stand-in");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(path).expect("the stand-in").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("let the stand-in run");
    }
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
