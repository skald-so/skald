//! The live check of Claude Code's `PostToolUse` and `Stop` hooks: how a session is set up with the
//! hook handler, what each case prompts, how long each run of the hook takes, and the checks'
//! verdicts. The sessions run with `-p`, as the `inbox` check's do.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agent_source::State;
use serde_json::{Value, json};

use crate::{
    Agent, Call, Event, Options, QUICK, Run, SERVER, SHELL, Session, Stub, handover_delay, report,
    verdict,
};

/// The hook handler's binary, next to `live-check`.
pub const HOOK_BIN: &str = "agent-hook";

/// A hook under check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hook {
    PostToolUse,
    Stop,
}

/// A session's case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// The message waits before the session starts.
    Waiting,
    /// The message comes while the agent runs `SHELL`.
    Busy,
}

impl Hook {
    /// The hook of `live-check`'s argument.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "post-tool-use" => Some(Self::PostToolUse),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }

    /// The hook event's name in Claude Code's settings and hook events.
    pub fn event(self) -> &'static str {
        match self {
            Self::PostToolUse => "PostToolUse",
            Self::Stop => "Stop",
        }
    }

    /// The session's prompt: `PostToolUse` needs a tool call, `Stop` only the end of a turn.
    pub fn prompt(self, case: Case) -> String {
        let token =
            format!("only the check token from the {SERVER} message, or NONE if there is none.");
        match (self, case) {
            (Self::Stop, Case::Waiting) => format!("Reply with {token}"),
            (Self::PostToolUse, Case::Waiting) => {
                format!("Run the shell command `{QUICK}`. Then reply with {token}")
            }
            (_, Case::Busy) => format!("Run the shell command `{SHELL}`. Then reply with {token}"),
        }
    }

    /// The model stub's tool calls, in order, before it replies `DONE`.
    pub fn script(self, case: Case) -> &'static [Call] {
        match (self, case) {
            (Self::Stop, Case::Waiting) => &[],
            (Self::PostToolUse, Case::Waiting) => &[Call::Quick],
            (_, Case::Busy) => &[Call::Shell],
        }
    }

    /// The built-in tools of the session.
    fn tools(self, case: Case) -> &'static str {
        match (self, case) {
            (Self::Stop, Case::Waiting) => "",
            _ => "Bash",
        }
    }
}

/// The `agent-hook` binary next to `live-check`, the binary `exe`.
pub fn hook_handler(exe: &Path) -> Result<PathBuf, String> {
    let handler = exe.with_file_name(HOOK_BIN);
    if handler.exists() {
        Ok(handler)
    } else {
        Err(format!(
            "{} is missing: build the workspace",
            handler.display()
        ))
    }
}

/// Writes the session's settings into `dir` and returns the arguments of a non-interactive `case`
/// session that runs `handler <source>` as its `hook` hook. Only the settings file and the
/// arguments change Claude Code's config: no project or user settings, no MCP server, no skills,
/// the lowest effort, and no tool but the shell, which may run only the case's command.
pub fn session(
    dir: &Path,
    handler: &Path,
    source: &Path,
    hook: Hook,
    case: Case,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(dir)?;
    // The exec form: Claude Code starts the handler with no shell.
    let handlers =
        json!([{ "hooks": [{ "type": "command", "command": handler, "args": [source] }] }]);
    let settings = dir.join("settings.json");
    fs::write(
        &settings,
        pretty(&json!({
            "hooks": { hook.event(): handlers },
            // Without it, Claude Code with `-p` refuses the commands: it cannot ask a person.
            "permissions": { "allow": [format!("Bash({QUICK})"), "Bash(sleep:*)"] },
        })),
    )?;
    let mut args: Vec<String> = [
        "-p",
        &hook.prompt(case),
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-hook-events",
        "--setting-sources",
        "local",
        "--strict-mcp-config",
        "--settings",
        &settings.to_string_lossy(),
        "--disable-slash-commands",
        "--tools",
        hook.tools(case),
        "--effort",
        "low",
        "--no-session-persistence",
    ]
    .map(String::from)
    .into();
    if let Some(model) = model {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    Ok(args)
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("JSON values serialize")
}

/// What every session of the check shares.
#[derive(Debug, Clone, Copy)]
pub struct Setup<'a> {
    /// The check's temporary directory.
    pub work: &'a Path,
    pub hook: Hook,
    /// The `agent-hook` binary.
    pub handler: &'a Path,
    pub model: Option<&'a str>,
    /// `live-check`'s `PATH`, the only variable the CLI on the stub inherits.
    pub path: &'a str,
}

impl Setup<'_> {
    /// Prepares the `case` session `name` in `work/<name>`: an empty project, the hook's settings
    /// and source and, on `stub`, a home of its own.
    pub fn run(&self, name: &str, case: Case, stub: Option<&Stub>) -> io::Result<Run> {
        let dir = self.work.join(name);
        let project = dir.join("project");
        let source = dir.join("source");
        fs::create_dir_all(&project)?;
        let args = session(&dir, self.handler, &source, self.hook, case, self.model)?;
        let env = match stub {
            Some(stub) => {
                Some(Agent::Claude.environment(&dir.join("home"), &stub.url(), self.path)?)
            }
            None => None,
        };
        Ok(Run {
            name: name.to_owned(),
            agent: Agent::Claude,
            project,
            source,
            args,
            env,
            log: dir.join("session.jsonl"),
            stderr: dir.join("session.stderr"),
        })
    }
}

/// One run of the hook under check, as Claude Code's hook events show it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HookRun {
    /// When Claude Code said it started the hook, in milliseconds since the Unix epoch.
    pub started_ms: u64,
    /// When it said the hook ended.
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
                ..
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

    /// The runs and how long each took, as Claude Code saw it.
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

fn handed_over(state: Option<State>) -> bool {
    matches!(state, Some(State::HandedOver { .. }))
}

/// The hook's checks: each returns its line in the output.
#[derive(Debug)]
pub struct HookCheck {
    hook: Hook,
    passed: Vec<bool>,
}

impl HookCheck {
    pub fn new(hook: Hook) -> Self {
        Self {
            hook,
            passed: Vec::new(),
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
        let details = format!(
            "{}; handed over {} after the session started",
            Self::summary(session, runs, token),
            handover_delay(state, session.started_ms)
        );
        self.report("message waiting at the start", pass, &details)
    }

    /// The other of two sessions, with no message: the hook ran, gave nothing, and the first
    /// session's `token` did not reach it.
    pub fn other(&mut self, session: &Session, runs: &HookRuns, token: &str) -> String {
        let pass = session.clean()
            && !runs.runs.is_empty()
            && runs.runs.iter().all(HookRun::quiet)
            && !session.got(token);
        let details = format!(
            "{}; the message is the first session's",
            Self::summary(session, runs, token)
        );
        self.report("the other of two sessions", pass, &details)
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
        let message = match added_at_ms {
            Some(added_at_ms) => format!(
                "added while the shell command ran, handed over {} later",
                handover_delay(state, added_at_ms)
            ),
            None => "the agent ran no shell command".to_owned(),
        };
        let details = format!("{}; {message}", Self::summary(session, runs, token));
        self.report("busy session", pass, &details)
    }

    /// Whether every check passed.
    pub fn pass(&self) -> bool {
        self.passed.iter().all(|pass| *pass)
    }

    /// The last line of the output.
    pub fn verdict(&self, options: &Options) -> String {
        format!(
            "{} hook on claude with {}: {}",
            self.hook.event(),
            options.model_name(),
            verdict(self.pass())
        )
    }

    fn report(&mut self, check: &str, pass: bool, details: &str) -> String {
        self.passed.push(pass);
        report(check, pass, details)
    }
}

#[cfg(test)]
mod tests {
    use crate::{DONE, StubLog, Usage};

    use super::*;

    #[test]
    fn parses_the_hooks_and_names_their_events() {
        assert_eq!(Hook::parse("post-tool-use"), Some(Hook::PostToolUse));
        assert_eq!(Hook::parse("stop"), Some(Hook::Stop));
        assert_eq!(Hook::parse("PostToolUse"), None);
        assert_eq!(Hook::parse("pre-tool-use"), None);
        assert_eq!(Hook::PostToolUse.event(), "PostToolUse");
        assert_eq!(Hook::Stop.event(), "Stop");
    }

    #[test]
    fn prompts_and_scripts_each_case() {
        let reply = "reply with only the check token from the skald message, or NONE if there is \
                     none.";
        assert_eq!(
            Hook::Stop.prompt(Case::Waiting),
            "Reply with only the check token from the skald message, or NONE if there is none."
        );
        assert_eq!(
            Hook::PostToolUse.prompt(Case::Waiting),
            format!("Run the shell command `true`. Then {reply}")
        );
        for hook in [Hook::PostToolUse, Hook::Stop] {
            assert_eq!(
                hook.prompt(Case::Busy),
                format!("Run the shell command `sleep 20`. Then {reply}")
            );
            assert_eq!(hook.script(Case::Busy), [Call::Shell]);
            assert_eq!(hook.tools(Case::Busy), "Bash");
        }
        assert_eq!(Hook::Stop.script(Case::Waiting), []);
        assert_eq!(Hook::Stop.tools(Case::Waiting), "");
        assert_eq!(Hook::PostToolUse.script(Case::Waiting), [Call::Quick]);
        assert_eq!(Hook::PostToolUse.tools(Case::Waiting), "Bash");
    }

    #[test]
    fn finds_the_hook_handler_next_to_live_check() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("live-check");
        let handler = dir.path().join("agent-hook");
        assert_eq!(
            hook_handler(&exe),
            Err(format!(
                "{} is missing: build the workspace",
                handler.display()
            ))
        );
        fs::write(&handler, "").unwrap();
        assert_eq!(hook_handler(&exe), Ok(handler));
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
            Case::Waiting,
            Some("haiku"),
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "-p",
                &Hook::Stop.prompt(Case::Waiting),
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-hook-events",
                "--setting-sources",
                "local",
                "--strict-mcp-config",
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
        let written: Value = serde_json::from_str(&fs::read_to_string(settings).unwrap()).unwrap();
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
            Case::Busy,
            None,
        )
        .unwrap();
        assert_eq!(busy[1], Hook::PostToolUse.prompt(Case::Busy));
        assert_eq!(busy[12..14], ["--tools", "Bash"]);
        assert_eq!(busy.len(), 17, "{busy:?}");
        let written: Value = serde_json::from_str(&fs::read_to_string(settings).unwrap()).unwrap();
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
            hook: Hook::PostToolUse,
            handler: Path::new("/bin/agent-hook"),
            model: Some("m"),
            path: "/bin",
        };
        let dir = work.path().join("a");
        let run = setup.run("a", Case::Busy, None).unwrap();
        let (project, source) = (dir.join("project"), dir.join("source"));
        assert!(project.is_dir());
        assert_eq!(
            fs::read_dir(&project).unwrap().count(),
            0,
            "the project is empty"
        );
        assert_eq!(
            run,
            Run {
                name: "a".to_owned(),
                agent: Agent::Claude,
                args: session(
                    &dir,
                    Path::new("/bin/agent-hook"),
                    &source,
                    Hook::PostToolUse,
                    Case::Busy,
                    Some("m")
                )
                .unwrap(),
                env: None,
                project,
                source,
                log: dir.join("session.jsonl"),
                stderr: dir.join("session.stderr"),
            }
        );

        let stub = Stub::start(&[]).unwrap();
        let run = setup.run("b", Case::Waiting, Some(&stub)).unwrap();
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

    fn ended(id: &str, event: &str, outcome: &str, stdout: &str) -> Event {
        Event::HookEnded {
            id: id.to_owned(),
            event: event.to_owned(),
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
        runs.on_event(&ended("p1", "PostToolUse", "success", "x"), 1_002);
        runs.on_event(&Event::ShellStarted, 1_003);
        runs.on_event(&ended("h1", "Stop", "success", "{\"tok-1\"}\n"), 1_012);
        runs.on_event(&started("h2", "Stop"), 2_000);
        runs.on_event(&ended("h3", "Stop", "success", "unknown run"), 2_005);
        runs.on_event(&started("h4", "Stop"), 3_000);
        runs.on_event(&ended("h2", "Stop", "error", ""), 2_007);
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
            runs.on_event(&ended(&id, hook.event(), outcome, stdout), at * 100 + 5);
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

    /// A session on the real model that replied `reply`.
    fn replied(reply: &str) -> Session {
        let mut session = Session::new(1_000);
        session.record(&Event::Reply(reply.to_owned()));
        session
    }

    /// A session on the stub whose requests hold `token`, if given.
    fn stubbed(token: Option<&str>, credentials: &[&str]) -> Session {
        let mut session = replied(DONE);
        session.stub = Some(StubLog {
            requests: vec![format!("{{\"text\":\"{}\"}}", token.unwrap_or("none"))],
            credentials: credentials.iter().map(|name| (*name).to_owned()).collect(),
        });
        session
    }

    fn handed() -> Option<State> {
        Some(State::HandedOver { at_ms: 1_900 })
    }

    #[test]
    fn checks_the_message_waiting_at_the_start() {
        let mut check = HookCheck::new(Hook::Stop);
        let stop = runs(Hook::Stop, &[("success", "tok-1"), ("success", "")]);
        assert_eq!(
            check.waiting(&stubbed(Some("tok-1"), &[]), &stop, "tok-1", handed()),
            r#"  message waiting at the start: pass — reply "Done.", Stop ran 2 time(s): 5 ms with messages, 5 ms without, requests to the stub: 1, the token reached it; handed over 0.9 s after the session started"#
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
            let mut check = HookCheck::new(Hook::Stop);
            let line = check.waiting(&session, runs, "tok-1", state);
            assert!(line.contains(": fail — "), "{line}");
            assert!(!check.pass());
        }

        // PostToolUse needs no quiet run after the one that gave the message.
        let post = runs(Hook::PostToolUse, &[("success", "tok-1")]);
        let mut check = HookCheck::new(Hook::PostToolUse);
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
        let mut check = HookCheck::new(Hook::PostToolUse);
        assert_eq!(
            check.other(&stubbed(None, &[]), &quiet, "tok-1"),
            r#"  the other of two sessions: pass — reply "Done.", PostToolUse ran 1 time(s): 5 ms without, requests to the stub: 1, the token did not reach it; the message is the first session's"#
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
            let mut check = HookCheck::new(Hook::PostToolUse);
            let line = check.other(&session, &runs, "tok-1");
            assert!(line.contains(": fail — "), "{line}");
            assert!(!check.pass());
        }
    }

    #[test]
    fn checks_the_busy_session() {
        let stop = runs(Hook::Stop, &[("success", "tok-1"), ("success", "")]);
        let mut check = HookCheck::new(Hook::Stop);
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
        let mut check = HookCheck::new(Hook::Stop);
        let line = check.busy(&replied("tok-1"), &stop, "tok-1", Some(1_500), None);
        assert!(line.contains(": fail — "), "{line}");
        assert!(!check.pass());
    }
}
