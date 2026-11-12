//! The live checks of read confirmations, what tells the sender that a session's model got a
//! message: the `inbox` server's `ack` tool, with each message's secret, and its piggyback acks;
//! the session's own file, read by a hook and by the client; and a message no layer confirms, sent
//! again by the next path, its repeat dropped by `msg_id`.

use std::fs;
use std::io;
use std::path::Path;

use agent_source::{Ack, Record, Registration, Source, State};
use agent_transcript::read_seen;
use serde_json::json;

use crate::{
    AckWith, Agent, Call, Checks, Options, PAUSE, QUICK, SERVER, SHELL, Session, claude_print,
    codex_exec, codex_hook, exec_hooks, handed_over, handover_delay, pretty, seconds,
};

/// The signals of a message's records that confirm the model got it.
pub const READ: [&str; 5] = [
    "transcript",
    "session-file",
    "ack",
    "reply",
    "app-server-turn",
];

/// Whether `records` hold a read confirmation.
pub fn confirmed(records: &[Record]) -> bool {
    records
        .iter()
        .any(|record| READ.contains(&record.signal.as_str()))
}

/// The prompt of the `ack` check's session; with `piggyback`, of its case where several messages
/// wait and the agent acks them in its next calls.
pub fn ack_prompt(piggyback: bool) -> String {
    let server = format!("the {SERVER} MCP server");
    if piggyback {
        format!(
            "Call the inbox tool of {server}. Call it again, listing the messages it returned as \
             seen. Then answer the last message with the reply tool of {server}: its msg_id as \
             reply_to, its ack, the text Done. and the other messages as seen. Then reply with \
             only OK."
        )
    } else {
        format!(
            "Call the inbox tool of {server}. Confirm each message it returns with the ack tool \
             of {server}, giving its msg_id and ack. Then reply with only the check token from \
             the message, or NONE if there is none."
        )
    }
}

/// The model stub's tool calls in the `ack` check: an idle session acks with a wrong secret, with
/// none, then with the right one; with `piggyback`, it acks in its next `inbox` call and replies.
pub fn ack_script(piggyback: bool) -> &'static [Call] {
    if piggyback {
        &[Call::Inbox, Call::InboxSeen, Call::Reply]
    } else {
        &[
            Call::Inbox,
            Call::Ack(AckWith::Wrong),
            Call::Ack(AckWith::Missing),
            Call::Ack(AckWith::Right),
        ]
    }
}

/// The acks an agent that never saw the message would send for `msg_id`, each with what the
/// source answers: with no secret, a wrong one, and for a message the source does not have.
pub fn injected(source: &Source, msg_id: &str) -> io::Result<Vec<(&'static str, Ack)>> {
    let mut outcomes = Vec::new();
    for (what, id, ack) in [
        ("no ack", msg_id, None),
        ("a wrong ack", msg_id, Some("0".repeat(32))),
        (
            "an unknown msg_id",
            "01a1cebe-0000-7000-8000-000000000000",
            Some("0".repeat(32)),
        ),
    ] {
        outcomes.push((
            what,
            agent_inbox::acknowledge(source, id, ack.as_deref(), "ack", "injected")?,
        ));
    }
    Ok(outcomes)
}

/// The prompt of the transcript check: a tool call, after which the `PostToolUse` hook hands the
/// message over, a second one, after which it reads the session's file, then a reply. Claude Code
/// writes its transcript after the hooks of the same step run, so the hook finds a message only in
/// a later run.
pub fn transcript_prompt() -> String {
    format!(
        "Run the shell command `{QUICK}`, then the shell command `{PAUSE}`. Then reply with only \
         the check token from the {SERVER} message, or NONE if there is none."
    )
}

/// The model stub's tool calls in the transcript check.
pub const TRANSCRIPT_SCRIPT: &[Call] = &[Call::Quick, Call::Pause];

/// Returns the arguments of a non-interactive session of `agent` whose `SessionStart`,
/// `PostToolUse` and `Stop` hooks run `handler` on `source`, and which keeps its session file:
/// Claude Code's transcript in its config home, Codex's rollout in its `sessions/`. Otherwise as
/// the hook check's sessions: on the model stub at `stub` if given; Claude Code's settings file in
/// `dir`.
pub fn transcript_session(
    agent: Agent,
    dir: &Path,
    project: &Path,
    handler: &Path,
    source: &Path,
    model: Option<&str>,
    stub: Option<&str>,
) -> io::Result<Vec<String>> {
    let events = ["SessionStart", "PostToolUse", "Stop"];
    let mut args = match agent {
        Agent::Claude => {
            fs::create_dir_all(dir)?;
            let hook = exec_hooks(handler, &[source]);
            let hooks: serde_json::Map<String, serde_json::Value> = events
                .iter()
                .map(|event| ((*event).to_owned(), hook.clone()))
                .collect();
            let settings = dir.join("settings.json");
            fs::write(
                &settings,
                pretty(&json!({
                    "hooks": hooks,
                    "permissions": { "allow": [format!("Bash({QUICK})"), "Bash(sleep:*)"] },
                })),
            )?;
            claude_print(
                &transcript_prompt(),
                &[
                    "--include-hook-events",
                    "--settings",
                    &settings.to_string_lossy(),
                ],
                "Bash",
                model,
            )
        }
        Agent::Codex => {
            let command = [handler, Path::new("--codex"), source]
                .map(|arg| crate::shell_quote(&arg.to_string_lossy()))
                .join(" ");
            let hooks: Vec<String> = events
                .iter()
                .map(|event| codex_hook(event, &command))
                .collect();
            codex_exec(
                project,
                &["--dangerously-bypass-hook-trust"],
                &hooks,
                model,
                stub,
                &transcript_prompt(),
            )
        }
    };
    // The session file is what the check reads.
    args.retain(|arg| arg != "--no-session-persistence" && arg != "--ephemeral");
    Ok(args)
}

/// Reads the session file that `source`'s registration names for the handed-over messages it
/// shows reached the model, the client's own read, and records each as `session-file`, with the
/// entry's time; returns how many it found. Only that file is read, and only its entries' kinds
/// and the messages' IDs are kept.
pub fn read_session_file(agent: Agent, source: &Source) -> io::Result<usize> {
    let Some(Registration {
        transcript_path: Some(path),
        ..
    }) = source.registration()?
    else {
        return Ok(0);
    };
    let handed: Vec<String> = source
        .handed_messages()?
        .into_iter()
        .map(|message| message.msg_id)
        .collect();
    let kind = match agent {
        Agent::Claude => agent_transcript::Agent::Claude,
        Agent::Codex => agent_transcript::Agent::Codex,
    };
    let seen = read_seen(kind, Path::new(&path), &handed)?;
    for found in &seen {
        let detail = found.at_ms.map(|at| format!("entry at {at}"));
        source.record(&found.msg_id, "session-file", detail.as_deref())?;
    }
    Ok(seen.len())
}

/// The prompt of the resend check's session: the messages come while it runs `sleep 20`; a second
/// command gives Claude Code time to write the transcript before the turn's `Stop` hook reads it.
pub fn resend_prompt() -> String {
    format!(
        "Run the shell command `{SHELL}`, then the shell command `{PAUSE}`. Then reply with only \
         the check tokens from the {SERVER} messages, or NONE if there are none."
    )
}

/// The model stub's tool calls in the resend check.
pub const RESEND_SCRIPT: &[Call] = &[Call::Shell, Call::Pause];

/// Returns the arguments of the resend check's session: `claude -p` that skips permission prompts,
/// so a peer message without the session's token is held; its `SessionStart` hook registers it in
/// `primary`, the socket's source, and its `Stop` hook is the next path, on `fallback`. It keeps
/// its transcript, which the hook reads. Its settings file goes in `dir`.
pub fn resend_session(
    dir: &Path,
    handler: &Path,
    primary: &Path,
    fallback: &Path,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(dir)?;
    let on = |source: &Path| exec_hooks(handler, &[source]);
    let settings = dir.join("settings.json");
    fs::write(
        &settings,
        pretty(&json!({
            "hooks": { "SessionStart": on(primary), "Stop": on(fallback) },
            "permissions": { "allow": [format!("Bash({QUICK})"), "Bash(sleep:*)"] },
        })),
    )?;
    let mut args = claude_print(
        &resend_prompt(),
        &["--settings", &settings.to_string_lossy()],
        "Bash",
        model,
    );
    args.retain(|arg| arg != "--no-session-persistence");
    args.extend(
        [
            "--permission-mode",
            "bypassPermissions",
            "--allow-dangerously-skip-permissions",
        ]
        .map(String::from),
    );
    Ok(args)
}

/// How long a handed-over message waits for a read confirmation before the next path sends it.
pub const RESEND_AFTER_MS: u64 = 5_000;

/// Sends again by `fallback` each message `primary` handed over at least `after_ms` before `now_ms`
/// that no layer confirmed and that `fallback` does not have yet; records `resent` for each and
/// returns their IDs.
pub fn resend_unconfirmed(
    primary: &Source,
    fallback: &Source,
    after_ms: u64,
    now_ms: u64,
) -> io::Result<Vec<String>> {
    let mut resent = Vec::new();
    for message in primary.handed_messages()? {
        let due = message
            .handed_over_at_ms
            .is_some_and(|at| now_ms.saturating_sub(at) >= after_ms);
        if !due
            || confirmed(&primary.records(&message.msg_id)?)
            || fallback.state(&message.msg_id)?.is_some()
        {
            continue;
        }
        primary.resend(&message.msg_id, fallback)?;
        primary.record(&message.msg_id, "resent", None)?;
        resent.push(message.msg_id);
    }
    Ok(resent)
}

/// The records' signals, each with how long after `from_ms` it was recorded and, for a session
/// file's entry, how long after it the entry was written, such as `transcript 2.1 s (entry 0.0 s)`.
pub fn signals(records: &[Record], from_ms: Option<u64>) -> String {
    let names: Vec<String> = records
        .iter()
        .map(|record| {
            let Some(from) = from_ms else {
                return record.signal.clone();
            };
            let after = |at: u64| seconds(at.saturating_sub(from));
            let entry = record
                .detail
                .as_deref()
                .and_then(|detail| detail.strip_prefix("entry at "))
                .and_then(|at| at.parse().ok());
            match entry {
                Some(entry) => format!(
                    "{} {} (entry {})",
                    record.signal,
                    after(record.at_ms),
                    after(entry)
                ),
                None => format!("{} {}", record.signal, after(record.at_ms)),
            }
        })
        .collect();
    if names.is_empty() {
        "no signal".to_owned()
    } else {
        names.join(", ")
    }
}

/// The details of a resent message: its signals on each path, from its first handover, and how
/// many times its token was in the model's last request.
fn resend_details(primary: &Confirmed, fallback: &Confirmed, reached: usize) -> String {
    let from = handed_at(primary.state);
    format!(
        "socket: {}; the next path: {}; in the model's last request {reached} time(s)",
        signals(&primary.records, from),
        signals(&fallback.records, from),
    )
}

/// When a message in `state` was handed over.
fn handed_at(state: Option<State>) -> Option<u64> {
    match state {
        Some(State::HandedOver { at_ms }) => Some(at_ms),
        _ => None,
    }
}

fn has(records: &[Record], signal: &str, detail: Option<&str>) -> bool {
    records
        .iter()
        .any(|r| r.signal == signal && (detail.is_none() || r.detail.as_deref() == detail))
}

/// A check message: its token, its state and its records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmed {
    pub token: String,
    pub state: Option<State>,
    pub records: Vec<Record>,
}

/// The read confirmations' checks: each returns its line in the output.
#[derive(Debug)]
pub struct ReadCheck {
    path: &'static str,
    checks: Checks,
}

impl ReadCheck {
    pub fn new(path: &'static str) -> Self {
        Self {
            path,
            checks: Checks::default(),
        }
    }

    /// The `ack` check's idle session: the message reached the model and the agent acked it with
    /// its secret; on the stub, its acks with a wrong secret and without one were refused; acks of
    /// an agent that never saw it, `injected`, were refused.
    pub fn ack(
        &mut self,
        session: &Session,
        message: &Confirmed,
        injected: &[(&str, Ack)],
    ) -> String {
        let refused_in_session =
            session.stub.is_none() || has(&message.records, "ack-refused", None);
        let pass = session.received(&message.token, message.state)
            && has(&message.records, "ack", Some("ack"))
            && refused_in_session
            && !injected.is_empty()
            && injected
                .iter()
                .all(|(_, outcome)| *outcome != Ack::Accepted);
        let injected: Vec<String> = injected
            .iter()
            .map(|(what, outcome)| format!("{what}: {}", outcome.refusal().unwrap_or("accepted")))
            .collect();
        let details = format!(
            "{}; {}; signals after the handover: {}; injected acks: {}",
            session.summary(&message.token),
            handover_delay(message.state, session.started_ms)
                .replace(" s", " s after the session started"),
            signals(&message.records, handed_at(message.state)),
            injected.join(", ")
        );
        self.checks.report("idle session", pass, &details)
    }

    /// The `ack` check's piggyback case: the session's next `inbox` call acked every message and
    /// its reply confirmed the last.
    pub fn piggyback(&mut self, session: &Session, messages: &[Confirmed]) -> String {
        let pass = session.clean()
            && session.inbox_called
            && messages.len() > 1
            && messages.iter().all(|m| handed_over(m.state))
            && messages
                .iter()
                .all(|m| has(&m.records, "ack", Some("inbox")))
            && messages
                .last()
                .is_some_and(|m| has(&m.records, "reply", Some("Done.")));
        let each: Vec<String> = messages
            .iter()
            .map(|m| signals(&m.records, handed_at(m.state)))
            .collect();
        let details = format!(
            "{}; signals of each message: {}",
            session.summary(""),
            each.join("; ")
        );
        self.checks
            .report("several messages, acked in the next calls", pass, &details)
    }

    /// The transcript check's session: the message was handed over, the `Stop` hook found it in
    /// the session's file and the client did too.
    pub fn transcript(&mut self, session: &Session, message: &Confirmed) -> String {
        let pass = session.clean()
            && handed_over(message.state)
            && has(&message.records, "transcript", None)
            && has(&message.records, "session-file", None);
        let details = format!(
            "{}{}; signals after the handover: {}",
            session.head(),
            session.tail(&message.token),
            signals(&message.records, handed_at(message.state))
        );
        self.checks
            .report("message handed over by PostToolUse", pass, &details)
    }

    /// The resend check's message that the socket's session held: no layer confirmed it, so the
    /// next path sent it again, and the session's file then confirmed it.
    pub fn lost(&mut self, primary: &Confirmed, fallback: &Confirmed, reached: usize) -> String {
        let pass = has(&primary.records, "socket-held", None)
            && has(&primary.records, "resent", None)
            && !confirmed(&primary.records)
            && handed_over(fallback.state)
            && confirmed(&fallback.records)
            && !has(&fallback.records, "repeat-dropped", None)
            && reached == 1;
        let details = resend_details(primary, fallback, reached);
        self.checks
            .report("message held by the socket's session", pass, &details)
    }

    /// The resend check's message that the socket delivered late: sent again, and the next path
    /// dropped the repeat by `msg_id`.
    pub fn repeated(
        &mut self,
        primary: &Confirmed,
        fallback: &Confirmed,
        reached: usize,
    ) -> String {
        let pass = !has(&primary.records, "socket-held", None)
            && has(&primary.records, "resent", None)
            && has(&fallback.records, "repeat-dropped", None)
            && reached == 1;
        let details = resend_details(primary, fallback, reached);
        self.checks.report(
            "message the socket delivered after the timeout",
            pass,
            &details,
        )
    }

    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output.
    pub fn verdict(&self, agent: Agent, options: &Options) -> String {
        self.checks.verdict(self.path, agent, options)
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{read_json, replied, stubbed};
    use crate::{DONE, Event};

    use super::*;

    fn record(signal: &str, at_ms: u64, detail: Option<&str>) -> Record {
        Record {
            signal: signal.to_owned(),
            at_ms,
            detail: detail.map(String::from),
            seq: String::new(),
        }
    }

    #[test]
    fn counts_only_the_layers_that_tell_the_model_got_it() {
        for signal in READ {
            assert!(confirmed(&[record(signal, 1, None)]), "{signal}");
        }
        for signal in [
            "socket-held",
            "socket-expired",
            "resent",
            "ack-refused",
            "repeat-dropped",
        ] {
            assert!(!confirmed(&[record(signal, 1, None)]), "{signal}");
        }
        assert!(!confirmed(&[]));
    }

    #[test]
    fn prompts_and_scripts_each_case() {
        assert!(ack_prompt(false).contains("Confirm each message it returns with the ack tool"));
        assert!(ack_prompt(true).contains("listing the messages it returned as seen"));
        assert_eq!(
            ack_script(true),
            [Call::Inbox, Call::InboxSeen, Call::Reply]
        );
        assert_eq!(
            ack_script(false),
            [
                Call::Inbox,
                Call::Ack(AckWith::Wrong),
                Call::Ack(AckWith::Missing),
                Call::Ack(AckWith::Right)
            ]
        );
        assert!(
            transcript_prompt()
                .starts_with("Run the shell command `true`, then the shell command `sleep 2`.")
        );
        assert_eq!(TRANSCRIPT_SCRIPT, [Call::Quick, Call::Pause]);
        assert!(resend_prompt().starts_with("Run the shell command `sleep 20`, then"));
        assert_eq!(RESEND_SCRIPT, [Call::Shell, Call::Pause]);
    }

    #[test]
    fn refuses_the_acks_of_an_agent_that_never_saw_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let message = source.add_with_ack("alice", "hi").unwrap();
        source
            .handed_over(&[source.take().unwrap()[0].msg_id.clone()])
            .unwrap();
        let outcomes = injected(&source, &message.msg_id).unwrap();
        assert_eq!(
            outcomes,
            [
                ("no ack", Ack::Missing),
                ("a wrong ack", Ack::Wrong),
                ("an unknown msg_id", Ack::Unknown)
            ]
        );
        assert!(!confirmed(&source.records(&message.msg_id).unwrap()));
    }

    #[test]
    fn sessions_keep_their_files_and_run_the_handler_on_each_event() {
        let dir = tempfile::tempdir().unwrap();
        let handler = Path::new("/bin/agent-hook");
        let source = Path::new("/w/source");
        let claude = transcript_session(
            Agent::Claude,
            dir.path(),
            dir.path(),
            handler,
            source,
            None,
            None,
        )
        .unwrap();
        assert!(
            !claude.iter().any(|arg| arg == "--no-session-persistence"),
            "{claude:?}"
        );
        let settings = read_json(dir.path().join("settings.json"));
        for event in ["SessionStart", "PostToolUse", "Stop"] {
            assert_eq!(
                settings["hooks"][event][0]["hooks"][0],
                json!({ "type": "command", "command": handler, "args": [source] }),
                "{event}"
            );
        }
        assert_eq!(
            settings["permissions"]["allow"],
            json!(["Bash(true)", "Bash(sleep:*)"])
        );
        let codex = transcript_session(
            Agent::Codex,
            dir.path(),
            dir.path(),
            handler,
            source,
            None,
            Some("http://s"),
        )
        .unwrap();
        assert!(!codex.iter().any(|arg| arg == "--ephemeral"), "{codex:?}");
        for event in ["SessionStart", "PostToolUse", "Stop"] {
            assert!(
                codex.contains(&codex_hook(
                    event,
                    "'/bin/agent-hook' '--codex' '/w/source'"
                )),
                "{event}: {codex:?}"
            );
        }

        let resend = resend_session(
            dir.path(),
            handler,
            Path::new("/w/p"),
            Path::new("/w/f"),
            None,
        )
        .unwrap();
        assert!(!resend.iter().any(|arg| arg == "--no-session-persistence"));
        assert!(resend.ends_with(&[
            "--permission-mode".to_owned(),
            "bypassPermissions".to_owned(),
            "--allow-dangerously-skip-permissions".to_owned()
        ]));
        let settings = read_json(dir.path().join("settings.json"));
        assert_eq!(
            settings["hooks"]["SessionStart"][0]["hooks"][0]["args"],
            json!(["/w/p"])
        );
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["args"],
            json!(["/w/f"])
        );
    }

    #[test]
    fn the_client_reads_the_registered_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path().join("source")).unwrap();
        assert_eq!(read_session_file(Agent::Claude, &source).unwrap(), 0);
        let message = source.add("alice", "hi").unwrap();
        source
            .handed_over(&[source.take().unwrap()[0].msg_id.clone()])
            .unwrap();
        let file = dir.path().join("t.jsonl");
        fs::write(
            &file,
            format!(
                r#"{{"type":"user","timestamp":"1970-01-01T00:00:02Z","message":{{"content":"{}"}}}}"#,
                message.msg_id
            ),
        )
        .unwrap();
        source
            .register(&Registration {
                session_id: "s".to_owned(),
                transcript_path: Some(file.to_string_lossy().into_owned()),
                ..Registration::default()
            })
            .unwrap();
        assert_eq!(read_session_file(Agent::Codex, &source).unwrap(), 0);
        assert_eq!(read_session_file(Agent::Claude, &source).unwrap(), 1);
        let records = source.records(&message.msg_id).unwrap();
        assert_eq!(records[0].signal, "session-file");
        assert_eq!(records[0].detail.as_deref(), Some("entry at 2000"));
        let told = signals(&records, Some(1_000));
        assert!(
            told.starts_with("session-file ") && told.ends_with(" s (entry 1.0 s)"),
            "{told}"
        );
    }

    #[test]
    fn resends_only_due_unconfirmed_messages_once() {
        let (dir, next) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let primary = Source::open(dir.path()).unwrap();
        let fallback = Source::open(next.path()).unwrap();
        let lost = primary.add("alice", "lost").unwrap();
        let read = primary.add("bob", "read").unwrap();
        let ids: Vec<String> = primary
            .take()
            .unwrap()
            .into_iter()
            .map(|m| m.msg_id)
            .collect();
        primary.handed_over(&ids).unwrap();
        primary.record(&read.msg_id, "transcript", None).unwrap();
        let at = primary
            .handed(&lost.msg_id)
            .unwrap()
            .handed_over_at_ms
            .unwrap();
        // Not due yet.
        assert!(
            resend_unconfirmed(&primary, &fallback, 5_000, at + 4_999)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            resend_unconfirmed(&primary, &fallback, 5_000, at + 5_000).unwrap(),
            std::slice::from_ref(&lost.msg_id)
        );
        assert_eq!(fallback.state(&lost.msg_id).unwrap(), Some(State::Waiting));
        assert!(has(&primary.records(&lost.msg_id).unwrap(), "resent", None));
        // Already resent: once only.
        assert!(
            resend_unconfirmed(&primary, &fallback, 5_000, at + 60_000)
                .unwrap()
                .is_empty()
        );
        let still = primary.add("carol", "waiting").unwrap();
        assert!(
            resend_unconfirmed(&primary, &fallback, 0, at + 60_000)
                .unwrap()
                .is_empty()
        );
        assert_eq!(primary.state(&still.msg_id).unwrap(), Some(State::Waiting));
    }

    #[test]
    fn tells_each_signal_and_its_delay() {
        let records = [
            record("transcript", 3_500, Some("entry at 2_000")),
            record("session-file", 9_000, Some("entry at 2100")),
            record("ack", 2_400, Some("ack")),
        ];
        assert_eq!(
            signals(&records, Some(2_000)),
            "transcript 1.5 s, session-file 7.0 s (entry 0.1 s), ack 0.4 s"
        );
        assert_eq!(signals(&records, None), "transcript, session-file, ack");
        assert_eq!(signals(&[], Some(1)), "no signal");
    }

    fn confirmed_with(signals: &[(&str, Option<&str>)]) -> Confirmed {
        Confirmed {
            token: "tok-1".to_owned(),
            state: Some(State::HandedOver { at_ms: 1_000 }),
            records: signals
                .iter()
                .map(|(signal, detail)| record(signal, 1_500, *detail))
                .collect(),
        }
    }

    #[test]
    fn judges_the_ack_checks() {
        let refused = [("no ack", Ack::Missing), ("a wrong ack", Ack::Wrong)];
        let acked = confirmed_with(&[("ack-refused", None), ("ack", Some("ack"))]);
        let mut check = ReadCheck::new("ack");
        let line = check.ack(&stubbed(Some("tok-1"), &[]), &acked, &refused);
        assert!(line.contains(": pass — "), "{line}");
        assert!(
            line.ends_with(
                "injected acks: no ack: no ack given, a wrong ack: the ack is not the message's"
            ),
            "{line}"
        );
        assert!(check.pass());
        assert_eq!(
            check.verdict(Agent::Codex, &Options::default()),
            "ack on codex with the model stub: pass"
        );
        // On the stub the session's own wrong acks must have been refused; an accepted injected
        // ack fails.
        for (session, message, injected) in [
            (
                stubbed(Some("tok-1"), &[]),
                confirmed_with(&[("ack", Some("ack"))]),
                &refused[..],
            ),
            (
                stubbed(Some("tok-1"), &[]),
                acked.clone(),
                &[("x", Ack::Accepted)][..],
            ),
            (stubbed(Some("tok-1"), &[]), acked.clone(), &[][..]),
            (stubbed(None, &[]), acked.clone(), &refused[..]),
        ] {
            let mut check = ReadCheck::new("ack");
            assert!(
                check
                    .ack(&session, &message, injected)
                    .contains(": fail — ")
            );
        }
        // On the real model, the reply carries the token and no refusal is needed.
        let mut check = ReadCheck::new("ack");
        let real = confirmed_with(&[("ack", Some("ack"))]);
        assert!(
            check
                .ack(&replied("tok-1"), &real, &refused)
                .contains(": pass — ")
        );

        let mut session = replied("OK");
        session.record(&Event::InboxCalled);
        let seen = confirmed_with(&[("ack", Some("inbox"))]);
        let last = confirmed_with(&[("ack", Some("inbox")), ("reply", Some(DONE))]);
        let mut check = ReadCheck::new("ack");
        assert!(
            check
                .piggyback(&session, &[seen.clone(), last.clone()])
                .contains(": pass — ")
        );
        for messages in [
            vec![last.clone()],
            vec![seen.clone(), seen.clone()],
            vec![confirmed_with(&[]), last],
        ] {
            let mut check = ReadCheck::new("ack");
            assert!(check.piggyback(&session, &messages).contains(": fail — "));
        }
    }

    #[test]
    fn judges_the_transcript_and_resend_checks() {
        let session = stubbed(None, &[]);
        let both = confirmed_with(&[
            ("transcript", None),
            ("session-file", Some("entry at 1200")),
        ]);
        let mut check = ReadCheck::new("transcript");
        let line = check.transcript(&session, &both);
        assert!(
            line.ends_with(
                "signals after the handover: transcript 0.5 s, session-file 0.5 s (entry 0.2 s)"
            ),
            "{line}"
        );
        assert!(check.pass());
        for missing in [
            confirmed_with(&[("transcript", None)]),
            confirmed_with(&[("session-file", None)]),
        ] {
            let mut check = ReadCheck::new("transcript");
            assert!(check.transcript(&session, &missing).contains(": fail — "));
        }

        let held = confirmed_with(&[("socket-held", None), ("resent", None)]);
        let shown = confirmed_with(&[("session-file", None)]);
        let mut check = ReadCheck::new("resend");
        assert!(check.lost(&held, &shown, 1).contains(": pass — "));
        assert!(
            check
                .repeated(
                    &confirmed_with(&[("resent", None)]),
                    &confirmed_with(&[("transcript", None), ("repeat-dropped", None)]),
                    1
                )
                .contains(": pass — ")
        );
        assert!(check.pass());
        for (primary, fallback, reached) in [
            (held.clone(), shown.clone(), 2),
            (held.clone(), confirmed_with(&[]), 1),
            (confirmed_with(&[("resent", None)]), shown.clone(), 1),
            (
                held.clone(),
                confirmed_with(&[("transcript", None), ("repeat-dropped", None)]),
                1,
            ),
            (
                confirmed_with(&[("socket-held", None), ("resent", None), ("ack", None)]),
                shown.clone(),
                1,
            ),
        ] {
            let mut check = ReadCheck::new("resend");
            assert!(
                check
                    .lost(&primary, &fallback, reached)
                    .contains(": fail — ")
            );
        }
        for (primary, fallback, reached) in [
            (held, confirmed_with(&[("repeat-dropped", None)]), 1),
            (confirmed_with(&[("resent", None)]), shown, 1),
            (
                confirmed_with(&[("resent", None)]),
                confirmed_with(&[("repeat-dropped", None)]),
                2,
            ),
        ] {
            let mut check = ReadCheck::new("resend");
            assert!(
                check
                    .repeated(&primary, &fallback, reached)
                    .contains(": fail — ")
            );
        }
    }
}
