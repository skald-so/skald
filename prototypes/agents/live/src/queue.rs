//! The live check of `codex queue` with the `agent-queue` prototype: how an interactive Codex
//! session is set up, how the check learns its thread, and the checks' verdicts. A queued message
//! reaches only a session that keeps running after its turn, so the binary runs Codex's terminal UI
//! in a pseudo-terminal, as the Channels check runs Claude Code.

use std::io;
use std::path::Path;

use serde_json::{Value, json};

use crate::channel::{Files, PtySession, Sent, TERM};
use crate::{Agent, Call, Checks, Kind, Options, SHELL, Usage, codex_config, codex_hook};

/// The prototype's binary, next to `live-check`.
pub const QUEUE_BIN: &str = "agent-queue";

/// The session's prompt: it starts the first turn, which creates the session's thread.
pub fn prompt(kind: Kind) -> String {
    match kind {
        Kind::Idle => "Reply with only OK.".to_owned(),
        Kind::Busy => format!("Run the shell command `{SHELL}`. Then reply with only OK."),
    }
}

/// The model stub's tool calls in the first turn, before it replies `DONE`; a queued message's
/// turn, after them, gets `DONE` at once.
pub fn script(kind: Kind) -> &'static [Call] {
    match kind {
        Kind::Idle => &[],
        Kind::Busy => &[Call::Shell],
    }
}

/// The arguments of an interactive Codex `kind` session in `files.project`, on the model stub at
/// `stub` if given. Only the arguments change Codex's config: `Stop` and `PreToolUse` hooks that
/// append their input to `files.hooks`, run without the trust Codex otherwise asks a person for;
/// the project trusted; no plugins, update check, notification program or skills' instructions; no
/// approval asked; the lowest effort; and its own app-server, not the shared background one. The
/// terminal UI has no flag that ignores the user's config.
pub fn session(files: &Files, kind: Kind, model: Option<&str>, stub: Option<&str>) -> Vec<String> {
    let project = files.project.to_string_lossy().into_owned();
    let mut args: Vec<String> = [
        "--no-daemon",
        "--disable",
        "plugins",
        "--dangerously-bypass-hook-trust",
        "--ask-for-approval",
        "never",
        "-C",
        &project,
    ]
    .map(String::from)
    .to_vec();
    let mut config = vec![
        codex_hook("Stop", &files.record()),
        codex_hook("PreToolUse", &files.record()),
        format!(
            "projects={{{}={{trust_level=\"trusted\"}}}}",
            json!(project)
        ),
        "check_for_update_on_startup=false".to_owned(),
        "notify=[]".to_owned(),
    ];
    config.extend(codex_config(stub));
    for value in config {
        args.extend(["-c".to_owned(), value]);
    }
    if let Some(model) = model {
        args.extend(["-m".to_owned(), model.to_owned()]);
    }
    args.push(prompt(kind));
    args
}

/// The terminal UI's whole environment on the model stub at `stub`: that of the `inbox` check and
/// the terminal.
pub fn stub_environment(home: &Path, stub: &str, path: &str) -> io::Result<Vec<(String, String)>> {
    let mut env = Agent::Codex.environment(home, stub, path)?;
    env.push(("TERM".to_owned(), TERM.to_owned()));
    Ok(env)
}

/// The session's thread, which `codex queue` takes: the `session_id` of the first hook input in the
/// hooks file's `text`.
pub fn thread(text: &str) -> Option<String> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .map_while(Result::ok)
        .find_map(|input| input["session_id"].as_str().map(String::from))
}

/// The tokens a session used, from the line Codex's terminal UI prints in `screen`, its output,
/// when it exits: `Token usage: total=13,336 input=13,316 (+ 12,672 cached) output=20`. The input
/// holds the cached tokens: the total is the input and the output.
pub fn usage(screen: &str) -> Option<Usage> {
    let line = screen[screen.rfind("Token usage:")?..].lines().next()?;
    let words: Vec<&str> = line.split_whitespace().collect();
    let number = |word: &str| word.replace(',', "").parse::<u64>().ok();
    let after = |key: &str| {
        words
            .iter()
            .find_map(|word| word.strip_prefix(key).and_then(number))
    };
    let cached = words
        .windows(2)
        .find(|pair| pair[1].starts_with("cached"))
        .and_then(|pair| number(pair[0]))
        .unwrap_or(0);
    Some(Usage {
        input: after("input=")?,
        cached,
        output: after("output=")?,
    })
}

/// The `codex queue` path's checks: each returns its line in the output.
#[derive(Debug, Default)]
pub struct QueueCheck {
    checks: Checks,
}

impl QueueCheck {
    /// The idle session, after its first turn, and the message queued for it.
    pub fn idle(&mut self, session: &PtySession, sent: Option<&Sent>) -> String {
        self.received("idle session", session, sent)
    }

    /// The other of two idle sessions, which must not get the first session's message with
    /// `token`.
    pub fn other(&mut self, session: &PtySession, token: &str) -> String {
        let pass = session.clean() && !session.replies().is_empty() && !session.got(token);
        let details = format!(
            "{}; the message is the first session's, its token {} the session",
            session.summary(),
            if session.got(token) {
                "reached"
            } else {
                "did not reach"
            }
        );
        self.checks
            .report("the other of two idle sessions", pass, &details)
    }

    /// The busy session and the message queued while its first turn ran the shell command, if it
    /// did.
    pub fn busy(&mut self, session: &PtySession, sent: Option<&Sent>) -> String {
        self.received("busy session", session, sent)
    }

    /// The `check` of a session and its message, if one was queued: the message is handed over and
    /// its token reached the model. The details tell how many turns ended: a queued message comes in
    /// a turn of its own.
    fn received(&mut self, check: &str, session: &PtySession, sent: Option<&Sent>) -> String {
        let Some(sent) = sent else {
            return self.checks.none(check, session, "no message was queued");
        };
        let details = sent.arrival(session, "queued");
        self.checks.report(check, sent.arrived(session), &details)
    }

    /// Whether every check passed.
    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output.
    pub fn verdict(&self, options: &Options) -> String {
        self.checks.verdict("codex queue", Agent::Codex, options)
    }
}

#[cfg(test)]
mod tests {
    use agent_source::State;

    use crate::channel::tests::{on_stub, real};
    use crate::{StubLog, shell_quote};

    use super::*;

    #[test]
    fn prompts_and_scripts_each_case() {
        assert_eq!(prompt(Kind::Idle), "Reply with only OK.");
        assert_eq!(
            prompt(Kind::Busy),
            "Run the shell command `sleep 20`. Then reply with only OK."
        );
        assert_eq!(script(Kind::Idle), []);
        assert_eq!(script(Kind::Busy), [Call::Shell]);
    }

    #[test]
    fn a_session_records_its_hooks_and_loads_as_little_as_the_terminal_ui_allows() {
        // A `'` in the path must not end the recording command's quoted word.
        let files = Files::new(Path::new("/w/it's"));
        let record = format!("cat >> {}", shell_quote("/w/it's/hooks.json"));
        let hook = format!(
            r#"[{{hooks=[{{type="command",command={}}}]}}]"#,
            json!(record)
        );
        let args = session(&files, Kind::Busy, Some("gpt-5.5"), None);
        assert_eq!(
            args,
            [
                "--no-daemon",
                "--disable",
                "plugins",
                "--dangerously-bypass-hook-trust",
                "--ask-for-approval",
                "never",
                "-C",
                "/w/it's/project",
                "-c",
                &format!("hooks.Stop={hook}"),
                "-c",
                &format!("hooks.PreToolUse={hook}"),
                "-c",
                r#"projects={"/w/it's/project"={trust_level="trusted"}}"#,
                "-c",
                "check_for_update_on_startup=false",
                "-c",
                "notify=[]",
                "-c",
                "skills.include_instructions=false",
                "-c",
                r#"model_reasoning_effort="low""#,
                "-m",
                "gpt-5.5",
                &prompt(Kind::Busy),
            ]
        );
        let args = session(&files, Kind::Idle, None, Some("http://127.0.0.1:4100"));
        assert!(
            args.contains(
                &r#"model_providers.stub.base_url="http://127.0.0.1:4100/v1""#.to_owned()
            ),
            "{args:?}"
        );
        assert_eq!(args.last(), Some(&prompt(Kind::Idle)));
    }

    #[test]
    fn the_terminal_ui_on_the_stub_gets_the_inbox_environment_and_the_terminal() {
        let home = tempfile::tempdir().unwrap();
        let mut expected = Agent::Codex
            .environment(home.path(), "http://127.0.0.1:1", "/bin")
            .unwrap();
        expected.push(("TERM".to_owned(), "xterm-256color".to_owned()));
        assert_eq!(
            stub_environment(home.path(), "http://127.0.0.1:1", "/bin").unwrap(),
            expected
        );
    }

    #[test]
    fn reads_the_tokens_the_terminal_ui_prints_when_it_exits() {
        let screen = "\x1b[0 qToken usage: total=13,336 input=13,316 (+ 12,672 cached) output=20\r\n\
                      To continue this session, run:\r\n";
        assert_eq!(
            usage(screen),
            Some(Usage {
                input: 13_316,
                cached: 12_672,
                output: 20
            })
        );
        assert_eq!(
            usage(
                "Token usage: total=1 input=0 (+ 9 cached) output=1\nToken usage: total=5 input=4 output=1"
            ),
            Some(Usage {
                input: 4,
                cached: 0,
                output: 1
            }),
            "the last line counts"
        );
        assert_eq!(usage("no usage"), None);
        assert_eq!(usage("Token usage: total=5 output=1"), None);

        let mut session = real(&["OK"]);
        session.usage = usage(screen);
        assert_eq!(
            session.summary(),
            r#"replies ["OK"], tokens: input 13316 (cached 12672), output 20"#
        );
        // On the stub, the stub's requests count.
        session.stub = Some(StubLog::default());
        assert_eq!(
            session.summary(),
            r#"replies ["OK"], requests to the stub: 0"#
        );
    }

    #[test]
    fn reads_the_thread_from_the_first_hook_input() {
        let text = r#"{"hook_event_name":"PreToolUse","session_id":"t1"}{"hook_event_name":"Stop","session_id":"t2"}"#;
        assert_eq!(thread(text).as_deref(), Some("t1"));
        assert_eq!(
            thread(r#"{"hook_event_name":"Stop"}{"session_id":"t3"}"#).as_deref(),
            Some("t3")
        );
        assert_eq!(thread(""), None);
        assert_eq!(thread(r#"{"session_id":"#), None, "a partly written input");
    }

    fn sent(token: &str, state: Option<State>) -> Sent {
        Sent {
            token: token.to_owned(),
            added_at_ms: 2_000,
            state,
        }
    }

    fn handed() -> Option<State> {
        Some(State::HandedOver { at_ms: 2_200 })
    }

    #[test]
    fn checks_the_idle_and_busy_sessions() {
        let mut session = on_stub(&["Done.", "Done."], &["{}", "{\"tok-1\"}"], &[]);
        session.note(&["tok-1"], 11_300);
        let mut check = QueueCheck::default();
        assert_eq!(
            check.idle(&session, Some(&sent("tok-1", handed()))),
            r#"  idle session: pass — replies ["Done.", "Done."], requests to the stub: 2; turns ended: 2; queued 0.2 s and reached the model 9.3 s after adding"#
        );
        assert_eq!(
            check.busy(&session, Some(&sent("tok-1", handed()))),
            r#"  busy session: pass — replies ["Done.", "Done."], requests to the stub: 2; turns ended: 2; queued 0.2 s and reached the model 9.3 s after adding"#
        );
        assert!(check.pass());
        assert_eq!(
            check.verdict(&Options::default()),
            "codex queue on codex with the model stub: pass"
        );

        for (session, sent) in [
            // Never queued.
            (session.clone(), None),
            // The command failed: the message waits.
            (session.clone(), Some(sent("tok-1", Some(State::Waiting)))),
            // The token did not reach the model.
            (session.clone(), Some(sent("tok-2", handed()))),
            // A credential other than the placeholder.
            (
                on_stub(&["Done."], &["{\"tok-1\"}"], &["authorization"]),
                Some(sent("tok-1", handed())),
            ),
        ] {
            let mut check = QueueCheck::default();
            let line = check.idle(&session, sent.as_ref());
            assert!(line.contains(": fail — "), "{line}");
            assert!(!check.pass());
            let mut check = QueueCheck::default();
            let line = check.busy(&session, sent.as_ref());
            assert!(line.contains(": fail — "), "{line}");
            assert!(!check.pass());
        }
        let mut check = QueueCheck::default();
        assert_eq!(
            check.busy(&on_stub(&["Done."], &[] as &[&str], &[]), None),
            r#"  busy session: fail — replies ["Done."], requests to the stub: 0; no message was queued"#
        );
    }

    #[test]
    fn on_the_real_model_the_session_repeats_the_token() {
        let session = real(&["OK", "tok-1"]);
        let mut check = QueueCheck::default();
        let line = check.idle(&session, Some(&sent("tok-1", handed())));
        assert!(line.contains(": pass — "), "{line}");
        let real = Options {
            real: true,
            ..Options::default()
        };
        assert_eq!(
            check.verdict(&real),
            "codex queue on codex with the real model: pass"
        );
    }

    #[test]
    fn checks_the_other_of_two_sessions() {
        let mut check = QueueCheck::default();
        assert_eq!(
            check.other(&on_stub(&["Done."], &["{}"], &[]), "tok-1"),
            r#"  the other of two idle sessions: pass — replies ["Done."], requests to the stub: 1; the message is the first session's, its token did not reach the session"#
        );
        assert!(check.pass());
        for session in [
            on_stub(&["Done."], &["{\"tok-1\"}"], &[]),
            // Its first turn never ended.
            on_stub(&[], &["{}"], &[]),
            on_stub(&["Done."], &["{}"], &["x-api-key"]),
        ] {
            let mut check = QueueCheck::default();
            let line = check.other(&session, "tok-1");
            assert!(line.contains(": fail — "), "{line}");
            assert!(!check.pass());
        }
    }
}
