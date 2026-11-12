//! The live check of the `PostToolUse` and `Stop` hooks on Claude Code and Codex, and Codex's
//! `UserPromptSubmit`: how a session is set up with the hook handler, what each case prompts, how
//! long each run of the hook takes, and the checks' verdicts. The sessions run with `claude -p` or
//! `codex exec`, as the `inbox` check's do.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use agent_source::{State, now_ms};
use serde_json::{Value, json};

use crate::{
    Agent, Call, Checks, Event, Kind, Options, QUICK, Run, SERVER, SHELL, Session, Stub,
    busy_details, claude_print, codex_exec, codex_hook, exec_hooks, handed_over, other_details,
    prepare, pretty, shell_quote, waiting_details,
};

/// The hook handler's binary, next to `live-check`.
pub const HOOK_BIN: &str = "agent-hook";

/// The file, in a session's directory, where `live-check timed-hook` records each run of the hook.
pub const HOOK_RUNS: &str = "hook-runs.jsonl";

/// A hook under check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hook {
    PostToolUse,
    Stop,
    /// Codex's only: the handler does not answer it on Claude Code.
    UserPromptSubmit,
}

impl Hook {
    /// The hook of `live-check`'s argument.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::PostToolUse, Self::Stop, Self::UserPromptSubmit]
            .into_iter()
            .find(|hook| hook.name() == name)
    }

    /// The hook's name as `live-check` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::PostToolUse => "post-tool-use",
            Self::Stop => "stop",
            Self::UserPromptSubmit => "user-prompt-submit",
        }
    }

    /// Whether the check runs on `agent`.
    pub fn runs_on(self, agent: Agent) -> bool {
        agent == Agent::Codex || self != Self::UserPromptSubmit
    }

    /// Whether the check has a busy session: `UserPromptSubmit` runs only when a prompt starts a
    /// turn, so a message that arrives during the turn waits for the next prompt, the case of a
    /// message waiting at the start.
    pub fn checks_busy(self) -> bool {
        self != Self::UserPromptSubmit
    }

    /// The hook event's name in the CLI's settings and hook events.
    pub fn event(self) -> &'static str {
        match self {
            Self::PostToolUse => "PostToolUse",
            Self::Stop => "Stop",
            Self::UserPromptSubmit => "UserPromptSubmit",
        }
    }

    /// The session's prompt: `PostToolUse` needs a tool call; `Stop` only the end of a turn and
    /// `UserPromptSubmit` only the prompt.
    pub fn prompt(self, kind: Kind) -> String {
        let token =
            format!("only the check token from the {SERVER} message, or NONE if there is none.");
        match (self, kind) {
            (Self::Stop | Self::UserPromptSubmit, Kind::Idle) => format!("Reply with {token}"),
            (Self::PostToolUse, Kind::Idle) => {
                format!("Run the shell command `{QUICK}`. Then reply with {token}")
            }
            (_, Kind::Busy) => format!("Run the shell command `{SHELL}`. Then reply with {token}"),
        }
    }

    /// The model stub's tool calls, in order, before it replies `DONE`.
    pub fn script(self, kind: Kind) -> &'static [Call] {
        match (self, kind) {
            (Self::Stop | Self::UserPromptSubmit, Kind::Idle) => &[],
            (Self::PostToolUse, Kind::Idle) => &[Call::Quick],
            (_, Kind::Busy) => &[Call::Shell],
        }
    }

    /// The built-in tools of a Claude Code session.
    fn tools(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Self::Stop | Self::UserPromptSubmit, Kind::Idle) => "",
            _ => "Bash",
        }
    }
}

/// Writes the session's settings into `dir` and returns the arguments of a non-interactive `kind`
/// session that runs `handler <source>` as its `hook` hook. Only the settings file and the
/// arguments change Claude Code's config: no project or user settings, no MCP server, no skills,
/// the lowest effort, and no tool but the shell, which may run only the kind's command.
pub fn session(
    dir: &Path,
    handler: &Path,
    source: &Path,
    hook: Hook,
    kind: Kind,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(dir)?;
    let handlers = exec_hooks(handler, &[source]);
    let settings = dir.join("settings.json");
    fs::write(
        &settings,
        pretty(&json!({
            "hooks": { hook.event(): handlers },
            // Without it, Claude Code with `-p` refuses the commands: it cannot ask a person.
            "permissions": { "allow": [format!("Bash({QUICK})"), "Bash(sleep:*)"] },
        })),
    )?;
    Ok(claude_print(
        &hook.prompt(kind),
        &[
            "--include-hook-events",
            "--settings",
            &settings.to_string_lossy(),
        ],
        hook.tools(kind),
        model,
    ))
}

/// Returns the arguments of a non-interactive Codex `kind` session in `project` that runs
/// `live-check timed-hook <dir>/hook-runs.jsonl <handler> --codex <source>` as its `hook` hook, on
/// the model stub at `stub` if given; `live_check` is `live-check`'s binary. Only the arguments
/// change Codex's config: as for the `inbox` session, and the hook, which runs without the trust
/// Codex otherwise asks a person for.
#[allow(clippy::too_many_arguments, reason = "the parts of a session")]
pub fn codex_session(
    dir: &Path,
    project: &Path,
    live_check: &Path,
    handler: &Path,
    source: &Path,
    hook: Hook,
    kind: Kind,
    model: Option<&str>,
    stub: Option<&str>,
) -> Vec<String> {
    // Codex runs a command hook with the shell.
    let command = [
        live_check,
        Path::new("timed-hook"),
        &dir.join(HOOK_RUNS),
        handler,
        Path::new("--codex"),
        source,
    ]
    .map(|arg| shell_quote(&arg.to_string_lossy()))
    .join(" ");
    codex_exec(
        project,
        &["--dangerously-bypass-hook-trust"],
        &[codex_hook(hook.event(), &command)],
        model,
        stub,
        &hook.prompt(kind),
    )
}

/// What every session of the check shares.
#[derive(Debug, Clone, Copy)]
pub struct Setup<'a> {
    /// The check's temporary directory.
    pub work: &'a Path,
    pub agent: Agent,
    pub hook: Hook,
    /// The `agent-hook` binary.
    pub handler: &'a Path,
    /// `live-check`'s binary, which times each run of the handler on Codex.
    pub live_check: &'a Path,
    pub model: Option<&'a str>,
    /// `live-check`'s `PATH`, the only variable the CLI on the stub inherits.
    pub path: &'a str,
}

impl Setup<'_> {
    /// Prepares the `kind` session `name` in `work/<name>`: an empty project, the hook's config
    /// and source and, on `stub`, a home of its own.
    pub fn run(&self, name: &str, kind: Kind, stub: Option<&Stub>) -> io::Result<Run> {
        let session =
            |dir: &Path, project: &Path, source: &Path, url: Option<&str>| match self.agent {
                Agent::Claude => session(dir, self.handler, source, self.hook, kind, self.model),
                Agent::Codex => Ok(codex_session(
                    dir,
                    project,
                    self.live_check,
                    self.handler,
                    source,
                    self.hook,
                    kind,
                    self.model,
                    url,
                )),
            };
        prepare(self.work, name, self.agent, self.path, stub, session)
    }
}

/// Runs `command`, a hook handler, with the hook input `input` on its stdin and writes its stdout
/// to `out`; appends the run to `log` as a JSON line: the hook event, when the handler started and
/// ended, in milliseconds since the Unix epoch, its outcome and stdout, and the input. Returns the
/// handler's exit code, 1 if a signal ended it. `live-check timed-hook` runs it for Codex, whose
/// `codex exec` shows no hook events.
pub fn timed(
    log: &Path,
    mut command: Command,
    input: &[u8],
    out: &mut impl Write,
) -> io::Result<i32> {
    let started_ms = now_ms();
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    // A handler that exits without reading its input closes the pipe: its exit code tells.
    let _ = stdin.write_all(input);
    drop(stdin);
    let output = child.wait_with_output()?;
    let ended_ms = now_ms();
    let code = output.status.code().unwrap_or(1);
    let input: Value = serde_json::from_slice(input).unwrap_or(Value::Null);
    let run = json!({
        "hook_event_name": input["hook_event_name"],
        "started_ms": started_ms,
        "ended_ms": ended_ms,
        "outcome": if code == 0 { "success" } else { "error" },
        "stdout": String::from_utf8_lossy(&output.stdout),
        "input": input,
    });
    let mut log = OpenOptions::new().create(true).append(true).open(log)?;
    log.write_all(format!("{run}\n").as_bytes())?;
    out.write_all(&output.stdout)?;
    out.flush()?;
    Ok(code)
}

/// Where `run`'s hook handler records its runs on Codex.
pub fn hook_runs_file(run: &Run) -> PathBuf {
    run.log.with_file_name(HOOK_RUNS)
}

/// One run of the hook under check, as Claude Code's hook events or, on Codex, [`timed`] show it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HookRun {
    /// When the hook started, in milliseconds since the Unix epoch.
    pub started_ms: u64,
    /// When the hook ended.
    pub ended_ms: Option<u64>,
    /// `success`, `error` or `cancelled`.
    pub outcome: String,
    /// What the handler printed.
    pub stdout: String,
}

impl HookRun {
    /// Whether the handler printed an output: it gave the agent messages.
    pub fn gave(&self) -> bool {
        !self.stdout.trim().is_empty()
    }

    /// Whether the run ended well and printed nothing: on `Stop`, it let the turn end.
    fn quiet(&self) -> bool {
        self.outcome == "success" && !self.gave()
    }
}

/// The runs of the hook under check in a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRuns {
    event: &'static str,
    ids: Vec<String>,
    pub runs: Vec<HookRun>,
}

impl HookRuns {
    pub fn new(hook: Hook) -> Self {
        Self {
            event: hook.event(),
            ids: Vec::new(),
            runs: Vec::new(),
        }
    }

    /// Records the runs of the hook in `text`, the lines `timed` writes.
    pub fn read_log(&mut self, text: &str) {
        for line in text.lines() {
            let Ok(run) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if run["hook_event_name"] != self.event {
                continue;
            }
            let text = |key: &str| run[key].as_str().unwrap_or_default().to_owned();
            self.runs.push(HookRun {
                started_ms: run["started_ms"].as_u64().unwrap_or_default(),
                ended_ms: run["ended_ms"].as_u64(),
                outcome: text("outcome"),
                stdout: text("stdout"),
            });
        }
    }

    /// Records what `event`, seen at `now_ms`, says about the hook's runs.
    pub fn on_event(&mut self, event: &Event, now_ms: u64) {
        match event {
            Event::HookStarted { id, event } if event == self.event => {
                self.ids.push(id.clone());
                self.runs.push(HookRun {
                    started_ms: now_ms,
                    ..HookRun::default()
                });
            }
            // A run's ID tells which hook ended.
            Event::HookEnded {
                id,
                outcome,
                stdout,
            } => {
                if let Some(at) = self.ids.iter().position(|started| started == id) {
                    let run = &mut self.runs[at];
                    run.ended_ms = Some(now_ms);
                    run.outcome.clone_from(outcome);
                    run.stdout.clone_from(stdout);
                }
            }
            _ => {}
        }
    }

    /// Whether exactly one run gave `token` to the agent, and it ended well.
    fn gave_once(&self, token: &str) -> bool {
        let giving: Vec<&HookRun> = self
            .runs
            .iter()
            .filter(|run| run.stdout.contains(token))
            .collect();
        matches!(giving[..], [run] if run.outcome == "success")
    }

    /// Whether the hook ran and its last run printed nothing: on `Stop`, the turn ended.
    fn ended_quietly(&self) -> bool {
        self.runs.last().is_some_and(HookRun::quiet)
    }

    /// The runs and how long each took.
    fn summary(&self) -> String {
        let runs: Vec<String> = self
            .runs
            .iter()
            .map(|run| {
                let took = run.ended_ms.map_or_else(
                    || "unfinished".to_owned(),
                    |ended| format!("{} ms", ended.saturating_sub(run.started_ms)),
                );
                let gave = if run.gave() {
                    "with messages"
                } else {
                    "without"
                };
                format!("{took} {gave}")
            })
            .collect();
        format!(
            "{} ran {} time(s): {}",
            self.event,
            runs.len(),
            runs.join(", ")
        )
    }
}

/// The hook's checks: each returns its line in the output.
#[derive(Debug)]
pub struct HookCheck {
    agent: Agent,
    hook: Hook,
    checks: Checks,
}

impl HookCheck {
    pub fn new(agent: Agent, hook: Hook) -> Self {
        Self {
            agent,
            hook,
            checks: Checks::default(),
        }
    }

    /// Whether the message with `token`, now in `state`, reached the session through one run of
    /// the hook; on `Stop`, the turn then ended.
    fn delivered(
        &self,
        session: &Session,
        runs: &HookRuns,
        token: &str,
        state: Option<State>,
    ) -> bool {
        session.clean()
            && session.got(token)
            && handed_over(state)
            && runs.gave_once(token)
            && (self.hook != Hook::Stop || runs.ended_quietly())
    }

    fn summary(session: &Session, runs: &HookRuns, token: &str) -> String {
        format!(
            "{}, {}{}",
            session.head(),
            runs.summary(),
            session.tail(token)
        )
    }

    /// The session whose message with `token` waited before it started, now in `state`.
    pub fn waiting(
        &mut self,
        session: &Session,
        runs: &HookRuns,
        token: &str,
        state: Option<State>,
    ) -> String {
        let pass = self.delivered(session, runs, token, state);
        let details = waiting_details(
            &Self::summary(session, runs, token),
            state,
            session.started_ms,
        );
        self.checks
            .report("message waiting at the start", pass, &details)
    }

    /// The other of two sessions, with no message: the hook ran, gave nothing, and the first
    /// session's `token` did not reach it.
    pub fn other(&mut self, session: &Session, runs: &HookRuns, token: &str) -> String {
        let pass = session.clean()
            && !runs.runs.is_empty()
            && runs.runs.iter().all(HookRun::quiet)
            && !session.got(token);
        let details = other_details(&Self::summary(session, runs, token));
        self.checks
            .report("the other of two sessions", pass, &details)
    }

    /// The busy session and its message with `token`: added at `added_at_ms`, if the agent ran a
    /// shell command, and now in `state`.
    pub fn busy(
        &mut self,
        session: &Session,
        runs: &HookRuns,
        token: &str,
        added_at_ms: Option<u64>,
        state: Option<State>,
    ) -> String {
        let pass = added_at_ms.is_some() && self.delivered(session, runs, token, state);
        let details = busy_details(&Self::summary(session, runs, token), added_at_ms, state);
        self.checks.report("busy session", pass, &details)
    }

    /// Whether every check passed.
    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output.
    pub fn verdict(&self, options: &Options) -> String {
        let path = format!("{} hook", self.hook.event());
        self.checks.verdict(&path, self.agent, options)
    }
}

#[cfg(test)]
mod tests {
    use crate::Usage;
    use crate::tests::{read_json, replied, stubbed};

    use super::*;

    #[test]
    fn parses_the_hooks_and_names_their_events() {
        assert_eq!(Hook::parse("post-tool-use"), Some(Hook::PostToolUse));
        assert_eq!(Hook::parse("stop"), Some(Hook::Stop));
        assert_eq!(Hook::parse("PostToolUse"), None);
        assert_eq!(Hook::parse("pre-tool-use"), None);
        assert_eq!(Hook::PostToolUse.event(), "PostToolUse");
        assert_eq!(Hook::Stop.event(), "Stop");
        assert_eq!(
            Hook::parse("user-prompt-submit"),
            Some(Hook::UserPromptSubmit)
        );
        assert_eq!(Hook::UserPromptSubmit.event(), "UserPromptSubmit");
        for hook in [Hook::PostToolUse, Hook::Stop, Hook::UserPromptSubmit] {
            assert_eq!(Hook::parse(hook.name()), Some(hook));
        }
    }

    #[test]
    fn user_prompt_submit_runs_only_on_codex_and_has_no_busy_session() {
        for hook in [Hook::PostToolUse, Hook::Stop] {
            assert!(hook.runs_on(Agent::Claude));
            assert!(hook.runs_on(Agent::Codex));
            assert!(hook.checks_busy());
        }
        assert!(!Hook::UserPromptSubmit.runs_on(Agent::Claude));
        assert!(Hook::UserPromptSubmit.runs_on(Agent::Codex));
        assert!(!Hook::UserPromptSubmit.checks_busy());
        assert_eq!(
            Hook::UserPromptSubmit.prompt(Kind::Idle),
            Hook::Stop.prompt(Kind::Idle)
        );
        assert_eq!(Hook::UserPromptSubmit.script(Kind::Idle), []);
        assert_eq!(Hook::UserPromptSubmit.tools(Kind::Idle), "");
    }

    #[test]
    fn a_codex_session_runs_the_timed_handler_as_its_only_hook_and_loads_no_user_config() {
        let args = codex_session(
            Path::new("/w/a"),
            Path::new("/w/a/project"),
            Path::new("/bin/live-check"),
            Path::new("/bin/agent-hook"),
            Path::new("/w/a/my source"),
            Hook::Stop,
            Kind::Idle,
            Some("gpt-5.5"),
            None,
        );
        let hook = r#"hooks.Stop=[{hooks=[{type="command",command="'/bin/live-check' 'timed-hook' '/w/a/hook-runs.jsonl' '/bin/agent-hook' '--codex' '/w/a/my source'"}]}]"#;
        assert_eq!(
            args,
            [
                "exec",
                "--json",
                "--skip-git-repo-check",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--disable",
                "plugins",
                "--dangerously-bypass-hook-trust",
                "-C",
                "/w/a/project",
                "-c",
                hook,
                "-c",
                "skills.include_instructions=false",
                "-c",
                r#"model_reasoning_effort="low""#,
                "-m",
                "gpt-5.5",
                &Hook::Stop.prompt(Kind::Idle),
            ]
        );

        let args = codex_session(
            Path::new("/w/c"),
            Path::new("/w/c/project"),
            Path::new("/bin/live-check"),
            Path::new("/bin/agent-hook"),
            Path::new("/w/c/source"),
            Hook::UserPromptSubmit,
            Kind::Busy,
            None,
            Some("http://127.0.0.1:4100"),
        );
        assert!(
            args[12].starts_with("hooks.UserPromptSubmit=[{hooks=[{"),
            "{args:?}"
        );
        assert!(
            args.contains(
                &r#"model_providers.stub.base_url="http://127.0.0.1:4100/v1""#.to_owned()
            ),
            "{args:?}"
        );
        assert_eq!(
            args.last(),
            Some(&Hook::UserPromptSubmit.prompt(Kind::Busy))
        );
    }

    #[test]
    fn prepares_a_codex_session_in_its_own_directory() {
        let work = tempfile::tempdir().unwrap();
        let setup = Setup {
            work: work.path(),
            agent: Agent::Codex,
            hook: Hook::PostToolUse,
            handler: Path::new("/bin/agent-hook"),
            live_check: Path::new("/bin/live-check"),
            model: None,
            path: "/bin",
        };
        let stub = Stub::start(&[]).unwrap();
        let dir = work.path().join("a");
        let run = setup.run("a", Kind::Idle, Some(&stub)).unwrap();
        assert_eq!(run.agent, Agent::Codex);
        assert_eq!(
            run.args,
            codex_session(
                &dir,
                &dir.join("project"),
                Path::new("/bin/live-check"),
                Path::new("/bin/agent-hook"),
                &dir.join("source"),
                Hook::PostToolUse,
                Kind::Idle,
                None,
                Some(&stub.url()),
            )
        );
        assert_eq!(
            run.env,
            Some(
                Agent::Codex
                    .environment(&dir.join("home"), &stub.url(), "/bin")
                    .unwrap()
            )
        );
        assert_eq!(hook_runs_file(&run), dir.join(HOOK_RUNS));
        assert_eq!(
            fs::read_dir(&run.project).unwrap().count(),
            0,
            "the project is empty"
        );
    }

    /// A command that runs `script` with the shell and nothing of the test's environment.
    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]).env_clear();
        command
    }

    #[test]
    fn times_a_handler_and_records_its_run() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join(HOOK_RUNS);
        let input = br#"{"hook_event_name":"Stop","session_id":"s1"}"#;
        let mut out = Vec::new();
        let before = now_ms();
        let code = timed(
            &log,
            sh("cat >/dev/null; printf '{\"x\":1}'"),
            input,
            &mut out,
        )
        .unwrap();
        assert_eq!(code, 0);
        assert_eq!(out, br#"{"x":1}"#);
        // A failing handler that reads nothing; its exit code passes on.
        let mut out = Vec::new();
        assert_eq!(timed(&log, sh("exit 3"), b"not JSON", &mut out).unwrap(), 3);
        assert!(out.is_empty());
        let killed = timed(&log, sh("kill -9 $$"), input, &mut Vec::new()).unwrap();
        assert_eq!(killed, 1);

        let text = fs::read_to_string(&log).unwrap();
        let runs: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(runs.len(), 3);
        let first = &runs[0];
        assert_eq!(first["hook_event_name"], "Stop");
        assert_eq!(first["outcome"], "success");
        assert_eq!(first["stdout"], r#"{"x":1}"#);
        assert_eq!(first["input"]["session_id"], "s1");
        let (started, ended) = (
            first["started_ms"].as_u64().unwrap(),
            first["ended_ms"].as_u64().unwrap(),
        );
        assert!(before <= started && started <= ended && ended <= now_ms());
        assert_eq!(runs[1]["outcome"], "error");
        assert_eq!(runs[1]["input"], Value::Null);
        assert_eq!(runs[2]["outcome"], "error");

        let mut stop = HookRuns::new(Hook::Stop);
        stop.read_log(&text);
        assert_eq!(stop.runs.len(), 2, "the run without an event is not Stop's");
        assert_eq!(
            stop.runs[0],
            HookRun {
                started_ms: started,
                ended_ms: Some(ended),
                outcome: "success".to_owned(),
                stdout: r#"{"x":1}"#.to_owned(),
            }
        );
        assert!(
            !timed(
                &dir.path().join("none/log"),
                sh("true"),
                input,
                &mut Vec::new()
            )
            .is_ok()
        );
        assert!(timed(&log, Command::new("/nonexistent"), input, &mut Vec::new()).is_err());
    }

    #[test]
    fn reads_the_runs_of_the_hook_under_check_from_the_log() {
        let text = [
            r#"{"hook_event_name":"PostToolUse","started_ms":1000,"ended_ms":1004,"outcome":"success","stdout":"tok-1"}"#,
            r#"{"hook_event_name":"Stop","started_ms":2000,"ended_ms":2003,"outcome":"success","stdout":""}"#,
            "not JSON",
            r#"{"hook_event_name":"PostToolUse","started_ms":3000,"outcome":"error"}"#,
        ]
        .join("\n");
        let mut runs = HookRuns::new(Hook::PostToolUse);
        runs.read_log(&text);
        assert_eq!(
            runs.runs,
            [
                HookRun {
                    started_ms: 1_000,
                    ended_ms: Some(1_004),
                    outcome: "success".to_owned(),
                    stdout: "tok-1".to_owned(),
                },
                HookRun {
                    started_ms: 3_000,
                    ended_ms: None,
                    outcome: "error".to_owned(),
                    stdout: String::new(),
                },
            ]
        );
        assert_eq!(
            runs.summary(),
            "PostToolUse ran 2 time(s): 4 ms with messages, unfinished without"
        );
    }

    #[test]
    fn names_the_agent_in_the_verdict() {
        let check = HookCheck::new(Agent::Codex, Hook::UserPromptSubmit);
        assert_eq!(
            check.verdict(&Options::default()),
            "UserPromptSubmit hook on codex with the model stub: pass"
        );
    }

    #[test]
    fn prompts_and_scripts_each_case() {
        let reply = "reply with only the check token from the skald message, or NONE if there is \
                     none.";
        assert_eq!(
            Hook::Stop.prompt(Kind::Idle),
            "Reply with only the check token from the skald message, or NONE if there is none."
        );
        assert_eq!(
            Hook::PostToolUse.prompt(Kind::Idle),
            format!("Run the shell command `true`. Then {reply}")
        );
        for hook in [Hook::PostToolUse, Hook::Stop] {
            assert_eq!(
                hook.prompt(Kind::Busy),
                format!("Run the shell command `sleep 20`. Then {reply}")
            );
            assert_eq!(hook.script(Kind::Busy), [Call::Shell]);
            assert_eq!(hook.tools(Kind::Busy), "Bash");
        }
        assert_eq!(Hook::Stop.script(Kind::Idle), []);
        assert_eq!(Hook::Stop.tools(Kind::Idle), "");
        assert_eq!(Hook::PostToolUse.script(Kind::Idle), [Call::Quick]);
        assert_eq!(Hook::PostToolUse.tools(Kind::Idle), "Bash");
    }

    #[test]
    fn a_session_runs_the_handler_as_its_only_hook_and_loads_only_its_own_config() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let settings = settings.to_str().unwrap();
        let args = session(
            dir.path(),
            Path::new("/bin/agent-hook"),
            Path::new("/tmp/my source"),
            Hook::Stop,
            Kind::Idle,
            Some("haiku"),
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "-p",
                &Hook::Stop.prompt(Kind::Idle),
                "--output-format",
                "stream-json",
                "--verbose",
                "--setting-sources",
                "local",
                "--strict-mcp-config",
                "--include-hook-events",
                "--settings",
                settings,
                "--disable-slash-commands",
                "--tools",
                "",
                "--effort",
                "low",
                "--no-session-persistence",
                "--model",
                "haiku",
            ]
        );
        let written = read_json(settings);
        assert_eq!(
            written,
            json!({
                "hooks": { "Stop": [{ "hooks": [{
                    "type": "command", "command": "/bin/agent-hook", "args": ["/tmp/my source"],
                }] }] },
                "permissions": { "allow": ["Bash(true)", "Bash(sleep:*)"] },
            })
        );

        let busy = session(
            dir.path(),
            Path::new("/bin/agent-hook"),
            Path::new("/s"),
            Hook::PostToolUse,
            Kind::Busy,
            None,
        )
        .unwrap();
        assert_eq!(busy[1], Hook::PostToolUse.prompt(Kind::Busy));
        assert_eq!(busy[12..14], ["--tools", "Bash"]);
        assert_eq!(busy.len(), 17, "{busy:?}");
        let written = read_json(settings);
        assert_eq!(
            written["hooks"],
            json!({ "PostToolUse": [{ "hooks": [{
                "type": "command", "command": "/bin/agent-hook", "args": ["/s"],
            }] }] })
        );
    }

    #[test]
    fn prepares_a_session_in_its_own_directory() {
        let work = tempfile::tempdir().unwrap();
        let setup = Setup {
            work: work.path(),
            agent: Agent::Claude,
            hook: Hook::PostToolUse,
            handler: Path::new("/bin/agent-hook"),
            live_check: Path::new("/bin/live-check"),
            model: Some("m"),
            path: "/bin",
        };
        let dir = work.path().join("a");
        let run = setup.run("a", Kind::Busy, None).unwrap();
        assert_eq!(run.agent, Agent::Claude);
        assert_eq!(run.project, dir.join("project"));
        assert_eq!(
            fs::read_dir(&run.project).unwrap().count(),
            0,
            "the project is empty"
        );
        assert_eq!(
            run.args,
            session(
                &dir,
                Path::new("/bin/agent-hook"),
                &dir.join("source"),
                Hook::PostToolUse,
                Kind::Busy,
                Some("m")
            )
            .unwrap()
        );
        assert_eq!(run.env, None);

        let stub = Stub::start(&[]).unwrap();
        let run = setup.run("b", Kind::Idle, Some(&stub)).unwrap();
        assert_eq!(
            run.env,
            Some(
                Agent::Claude
                    .environment(&work.path().join("b/home"), &stub.url(), "/bin")
                    .unwrap()
            )
        );
    }

    fn started(id: &str, event: &str) -> Event {
        Event::HookStarted {
            id: id.to_owned(),
            event: event.to_owned(),
        }
    }

    fn ended(id: &str, outcome: &str, stdout: &str) -> Event {
        Event::HookEnded {
            id: id.to_owned(),
            outcome: outcome.to_owned(),
            stdout: stdout.to_owned(),
        }
    }

    #[test]
    fn times_each_run_of_the_hook_under_check() {
        let mut runs = HookRuns::new(Hook::Stop);
        runs.on_event(&started("h1", "Stop"), 1_000);
        // Other hooks and events do not count.
        runs.on_event(&started("p1", "PostToolUse"), 1_001);
        runs.on_event(&ended("p1", "success", "x"), 1_002);
        runs.on_event(&Event::ShellStarted, 1_003);
        runs.on_event(&ended("h1", "success", "{\"tok-1\"}\n"), 1_012);
        runs.on_event(&started("h2", "Stop"), 2_000);
        runs.on_event(&ended("h3", "success", "unknown run"), 2_005);
        runs.on_event(&started("h4", "Stop"), 3_000);
        runs.on_event(&ended("h2", "error", ""), 2_007);
        assert_eq!(
            runs.runs,
            [
                HookRun {
                    started_ms: 1_000,
                    ended_ms: Some(1_012),
                    outcome: "success".to_owned(),
                    stdout: "{\"tok-1\"}\n".to_owned(),
                },
                HookRun {
                    started_ms: 2_000,
                    ended_ms: Some(2_007),
                    outcome: "error".to_owned(),
                    stdout: String::new(),
                },
                HookRun {
                    started_ms: 3_000,
                    ..HookRun::default()
                },
            ]
        );
        assert_eq!(
            runs.summary(),
            "Stop ran 3 time(s): 12 ms with messages, 7 ms without, unfinished without"
        );
        assert!(runs.gave_once("tok-1"));
        assert!(!runs.gave_once("tok-2"));
        assert!(!runs.ended_quietly(), "the last run did not end");
    }

    /// Runs of `hook`, each `(outcome, stdout)`.
    fn runs(hook: Hook, outputs: &[(&str, &str)]) -> HookRuns {
        let mut runs = HookRuns::new(hook);
        for (at, (outcome, stdout)) in (0u64..).zip(outputs) {
            let id = at.to_string();
            runs.on_event(&started(&id, hook.event()), at * 100);
            runs.on_event(&ended(&id, outcome, stdout), at * 100 + 5);
        }
        runs
    }

    #[test]
    fn a_message_is_given_once_and_a_quiet_last_run_ends_the_turn() {
        let gave = |outputs: &[(&str, &str)]| runs(Hook::Stop, outputs).gave_once("tok-1");
        assert!(gave(&[("success", "tok-1"), ("success", "")]));
        assert!(!gave(&[("success", "tok-1"), ("success", "tok-1")]));
        assert!(!gave(&[("error", "tok-1")]));
        assert!(!gave(&[("success", "")]));
        let quiet = |outputs: &[(&str, &str)]| runs(Hook::Stop, outputs).ended_quietly();
        assert!(quiet(&[("success", "tok-1"), ("success", " \n")]));
        assert!(!quiet(&[("success", ""), ("success", "tok-1")]));
        assert!(!quiet(&[("cancelled", "")]));
        assert!(!quiet(&[]));
    }

    fn handed() -> Option<State> {
        Some(State::HandedOver { at_ms: 1_900 })
    }

    #[test]
    fn checks_the_message_waiting_at_the_start() {
        let mut check = HookCheck::new(Agent::Claude, Hook::Stop);
        let stop = runs(Hook::Stop, &[("success", "tok-1"), ("success", "")]);
        assert_eq!(
            check.waiting(&stubbed(Some("tok-1"), &[]), &stop, "tok-1", handed()),
            r#"  message waiting at the start: pass — reply "Done.", Stop ran 2 time(s): 5 ms with messages, 5 ms without, requests to the stub: 2, the token reached it; handed over 0.9 s after the session started"#
        );
        assert!(check.pass());
        assert_eq!(
            check.verdict(&Options::default()),
            "Stop hook on claude with the model stub: pass"
        );

        let mut session = replied("The token is tok-1.");
        session.sign_in = Some("none".to_owned());
        session.record(&Event::Used(Usage {
            input: 10,
            cached: 4,
            output: 2,
        }));
        assert_eq!(
            check.waiting(&session, &stop, "tok-1", handed()),
            r#"  message waiting at the start: pass — reply "The token is tok-1.", apiKeySource none, Stop ran 2 time(s): 5 ms with messages, 5 ms without, tokens: input 10 (cached 4), output 2; handed over 0.9 s after the session started"#
        );

        let given = runs(Hook::Stop, &[("success", "tok-1")]);
        for (session, runs, state) in [
            // The token did not reach the model.
            (stubbed(None, &[]), &stop, handed()),
            (replied("NONE"), &stop, handed()),
            // Not handed over.
            (stubbed(Some("tok-1"), &[]), &stop, Some(State::Taken)),
            // A credential other than the placeholder.
            (stubbed(Some("tok-1"), &["x-api-key"]), &stop, handed()),
            // Stop never let the turn end.
            (stubbed(Some("tok-1"), &[]), &given, handed()),
        ] {
            let mut check = HookCheck::new(Agent::Claude, Hook::Stop);
            let line = check.waiting(&session, runs, "tok-1", state);
            assert!(line.contains(": fail — "), "{line}");
            assert!(!check.pass());
        }

        // PostToolUse needs no quiet run after the one that gave the message.
        let post = runs(Hook::PostToolUse, &[("success", "tok-1")]);
        let mut check = HookCheck::new(Agent::Claude, Hook::PostToolUse);
        let line = check.waiting(&stubbed(Some("tok-1"), &[]), &post, "tok-1", handed());
        assert!(line.contains(": pass — "), "{line}");
        let real = Options {
            real: true,
            ..Options::default()
        };
        assert_eq!(
            check.verdict(&real),
            "PostToolUse hook on claude with the real model: pass"
        );
        let twice = runs(
            Hook::PostToolUse,
            &[("success", "tok-1"), ("success", "tok-1")],
        );
        let line = check.waiting(&stubbed(Some("tok-1"), &[]), &twice, "tok-1", handed());
        assert!(line.contains(": fail — "), "{line}");
        assert_eq!(
            check.verdict(&real),
            "PostToolUse hook on claude with the real model: fail"
        );
    }

    #[test]
    fn checks_the_other_of_two_sessions() {
        let quiet = runs(Hook::PostToolUse, &[("success", "")]);
        let mut check = HookCheck::new(Agent::Claude, Hook::PostToolUse);
        assert_eq!(
            check.other(&stubbed(None, &[]), &quiet, "tok-1"),
            r#"  the other of two sessions: pass — reply "Done.", PostToolUse ran 1 time(s): 5 ms without, requests to the stub: 2, the token did not reach it; the message is the first session's"#
        );
        assert!(check.pass());
        for (session, runs) in [
            (stubbed(Some("tok-1"), &[]), quiet.clone()),
            (replied("tok-1"), quiet.clone()),
            (stubbed(None, &["authorization"]), quiet.clone()),
            (stubbed(None, &[]), runs(Hook::PostToolUse, &[])),
            (
                stubbed(None, &[]),
                runs(Hook::PostToolUse, &[("success", "tok-2")]),
            ),
            (
                stubbed(None, &[]),
                runs(Hook::PostToolUse, &[("error", "")]),
            ),
        ] {
            let mut check = HookCheck::new(Agent::Claude, Hook::PostToolUse);
            let line = check.other(&session, &runs, "tok-1");
            assert!(line.contains(": fail — "), "{line}");
            assert!(!check.pass());
        }
    }

    #[test]
    fn checks_the_busy_session() {
        let stop = runs(Hook::Stop, &[("success", "tok-1"), ("success", "")]);
        let mut check = HookCheck::new(Agent::Claude, Hook::Stop);
        assert_eq!(
            check.busy(&replied("tok-1"), &stop, "tok-1", Some(1_500), handed()),
            r#"  busy session: pass — reply "tok-1", Stop ran 2 time(s): 5 ms with messages, 5 ms without; added while the shell command ran, handed over 0.4 s later"#
        );
        assert!(check.pass());
        assert_eq!(
            check.busy(&replied("NONE"), &stop, "tok-1", None, None),
            r#"  busy session: fail — reply "NONE", Stop ran 2 time(s): 5 ms with messages, 5 ms without; the agent ran no shell command"#
        );
        assert!(!check.pass());
        let mut check = HookCheck::new(Agent::Claude, Hook::Stop);
        let line = check.busy(&replied("tok-1"), &stop, "tok-1", Some(1_500), None);
        assert!(line.contains(": fail — "), "{line}");
        assert!(!check.pass());
    }
}
