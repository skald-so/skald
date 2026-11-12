//! Reads a session's own file, a Claude Code transcript or a Codex rollout, for the entries that
//! went to the model, and finds there the `msg_id`s of handed-over messages: a read confirmation
//! that needs no help from the model. It keeps no line it reads.
//!
//! Both CLIs write a message to the file before they call the model, and the entry stays when the
//! call fails. So an entry counts only once a later entry is the model's own: a reply, a tool call
//! or reasoning, not an API error.
//!
//! Only these entries count, as the CLIs write them (Claude Code 2.1.322, Codex 0.162.1):
//! - Claude Code: a `user` entry (a prompt, a cross-session message, a tool result), a
//!   `hook_additional_context` attachment, and an `api-request-blob`, the part of a request to the
//!   model it holds. A `queue-operation` entry is a prompt still queued, and a `hook_success`
//!   attachment only a hook's output: neither reached the model. The model's own is an `assistant`
//!   entry; on a failed call Claude Code writes one with `isApiErrorMessage` and the model
//!   `<synthetic>`.
//! - Codex: a `response_item` other than the model's own: a user or developer message, or a tool's
//!   output. The model's own is an assistant message, reasoning or a call; a failed call writes no
//!   `response_item`.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The CLI whose session file it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

/// A message found in an entry that went to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub msg_id: String,
    /// The first such entry's time, in milliseconds since the Unix epoch, if it has one.
    pub at_ms: Option<u64>,
}

/// The messages of `msg_ids` that the session file `path` shows reached the model, each once, in
/// the order of its first entry: an entry that went to the model, followed later by the model's
/// own. A file that does not exist yet shows none.
pub fn read_seen(agent: Agent, path: &Path, msg_ids: &[String]) -> io::Result<Vec<Seen>> {
    let (mut written, answered) = scan(agent, path, msg_ids)?;
    written.truncate(answered);
    Ok(written)
}

/// The messages of `msg_ids` that the session file `path` holds in an entry that went to the model,
/// whether or not the model's own followed, each once, in the order of its first entry.
pub fn read_written(agent: Agent, path: &Path, msg_ids: &[String]) -> io::Result<Vec<Seen>> {
    Ok(scan(agent, path, msg_ids)?.0)
}

/// The messages of `msg_ids` that `path` holds in an entry that went to the model, and how many of
/// the first of them the model's own entries followed.
fn scan(agent: Agent, path: &Path, msg_ids: &[String]) -> io::Result<(Vec<Seen>, usize)> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(error) => return Err(error),
    };
    let mut written: Vec<Seen> = Vec::new();
    let mut answered = 0;
    for line in BufReader::new(file).lines() {
        // A line still being written is not an entry yet.
        let Ok(entry) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        if from_model(agent, &entry) {
            answered = written.len();
            continue;
        }
        if !went_to_model(agent, &entry) {
            continue;
        }
        let text = entry.to_string();
        for msg_id in msg_ids {
            if text.contains(msg_id.as_str()) && !written.iter().any(|s| s.msg_id == *msg_id) {
                written.push(Seen {
                    msg_id: msg_id.clone(),
                    at_ms: entry["timestamp"].as_str().and_then(timestamp_ms),
                });
            }
        }
    }
    Ok((written, answered))
}

/// Whether `entry` of a session file is the model's own: a reply, a tool call or reasoning, but
/// not an API error that stands in for one.
pub fn from_model(agent: Agent, entry: &Value) -> bool {
    match agent {
        Agent::Claude => {
            entry["type"] == "assistant"
                && entry["isApiErrorMessage"] != true
                && entry["message"]["model"] != "<synthetic>"
        }
        Agent::Codex => {
            let payload = &entry["payload"];
            entry["type"] == "response_item"
                && match payload["type"].as_str() {
                    Some("message") => payload["role"] == "assistant",
                    Some(kind) => kind == "reasoning" || kind.ends_with("_call"),
                    None => false,
                }
        }
    }
}

/// Whether `entry` of a session file went to the model.
pub fn went_to_model(agent: Agent, entry: &Value) -> bool {
    match agent {
        Agent::Claude => match entry["type"].as_str() {
            Some("user" | "api-request-blob") => true,
            Some("attachment") => entry["attachment"]["type"] == "hook_additional_context",
            _ => false,
        },
        Agent::Codex => {
            let payload = &entry["payload"];
            entry["type"] == "response_item"
                && match payload["type"].as_str() {
                    Some("message") => payload["role"] != "assistant",
                    Some(kind) => kind.ends_with("_output"),
                    None => false,
                }
        }
    }
}

/// Claude Code's transcript of the session `session_id` in the project `cwd`, under the config
/// home `config`, such as `~/.claude`: Claude Code names a project's directory after its real path,
/// with `-` for each character but letters and digits.
pub fn claude_transcript(config: &Path, cwd: &Path, session_id: &str) -> io::Result<PathBuf> {
    Ok(claude_project(config, cwd)?.join(format!("{session_id}.jsonl")))
}

/// Claude Code's directory of the project `cwd` under the config home `config`.
pub fn claude_project(config: &Path, cwd: &Path) -> io::Result<PathBuf> {
    let key: String = fs::canonicalize(cwd)?
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    Ok(config.join("projects").join(key))
}

/// The rollout of the Codex thread `thread` under the config home `codex`, such as `~/.codex`:
/// `sessions/<year>/<month>/<day>/rollout-<time>-<thread>.jsonl`. Reads only directory names.
pub fn codex_rollout(codex: &Path, thread: &str) -> io::Result<Option<PathBuf>> {
    let suffix = format!("-{thread}.jsonl");
    // The files in `sessions/<year>/<month>/<day>/`: four levels down.
    let mut days = vec![codex.join("sessions")];
    for _ in 0..4 {
        let mut next = Vec::new();
        for dir in days {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries {
                next.push(entry?.path());
            }
        }
        days = next;
    }
    Ok(days.into_iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(&suffix))
    }))
}

/// An RFC 3339 time in UTC, such as `2026-11-12T07:52:11.460Z`, in milliseconds since the Unix
/// epoch.
pub fn timestamp_ms(text: &str) -> Option<u64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let mut date = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let (hms, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut hms = hms.split(':').map(|part| part.parse::<i64>().ok());
    let (hour, minute, second) = (hms.next()??, hms.next()??, hms.next()??);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let millis: i64 = format!("{fraction:0<3}").get(..3)?.parse().ok()?;
    // Days from the civil date, after Howard Hinnant's `days_from_civil`.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let ms = ((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis;
    u64::try_from(ms).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01a1cebe-27a9-7240-b4fa-5a2184ce744f";
    const OTHER: &str = "01a1cebe-27a9-7240-b4fa-000000000002";

    // Entries recorded from Claude Code 2.1.322, shortened; `ID` stands for a message's `msg_id`.
    const CLAUDE: &str = r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-11-12T07:49:13.734Z","content":"<message msg_id=\"01a1cebe-27a9-7240-b4fa-000000000002\">"}
{"type":"attachment","timestamp":"2026-11-12T07:49:14.000Z","attachment":{"type":"hook_success","hookEvent":"PostToolUse","stdout":"<message msg_id=\"01a1cebe-27a9-7240-b4fa-000000000002\">"}}
{"type":"system","subtype":"stop_hook_summary","hookAdditionalContext":["01a1cebe-27a9-7240-b4fa-000000000002"]}
{"type":"attachment","timestamp":"2026-11-12T07:49:14.100Z","attachment":{"type":"hook_additional_context","content":["<message msg_id=\"01a1cebe-27a9-7240-b4fa-5a2184ce744f\">x</message>"],"hookName":"PostToolUse:Bash"}}
{"type":"api-request-blob","message":{"role":"system","content":[{"type":"text","text":"PostToolUse:Bash hook additional context: 01a1cebe-27a9-7240-b4fa-5a2184ce744f"}]}}
{"type":"user","timestamp":"2026-11-12T07:49:15.000Z","message":{"role":"user","content":"Another Claude session sent a message:\n01a1cebe-27a9-7240-b4fa-5a2184ce744f"}}
{"type":"assistant","timestamp":"2026-11-12T07:49:16.000Z","message":{"content":[{"type":"text","text":"01a1cebe-27a9-7240-b4fa-000000000002"}]}}
{"type":"user","timestamp":"2026-11-12T07:49:17"#;

    // Lines recorded from Codex 0.162.1's rollout, shortened.
    const CODEX: &str = r#"{"timestamp":"2026-11-12T07:52:11.632Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<message msg_id=\"01a1cebe-27a9-7240-b4fa-5a2184ce744f\">x</message>"}]}}
{"timestamp":"2026-11-12T07:52:11.700Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"text":"01a1cebe-27a9-7240-b4fa-000000000002"}]}}}
{"timestamp":"2026-11-12T07:52:11.800Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"01a1cebe-27a9-7240-b4fa-000000000002"}]}}
{"timestamp":"2026-11-12T07:52:11.900Z","type":"response_item","payload":{"type":"reasoning","summary":["01a1cebe-27a9-7240-b4fa-000000000002"]}}"#;

    fn ids() -> Vec<String> {
        vec![ID.to_owned(), OTHER.to_owned()]
    }

    fn written(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn finds_a_claude_code_message_only_where_it_went_to_the_model() {
        let (_dir, path) = written(CLAUDE);
        assert_eq!(
            read_seen(Agent::Claude, &path, &ids()).unwrap(),
            [Seen {
                msg_id: ID.to_owned(),
                at_ms: timestamp_ms("2026-11-12T07:49:14.100Z"),
            }]
        );
        // The model's own reply and a queued prompt are not its context.
        let (_dir, path) = written(&CLAUDE.replace(ID, "x"));
        assert!(read_seen(Agent::Claude, &path, &ids()).unwrap().is_empty());
        // A file that exists but cannot be read is an error, unlike one not written yet.
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = written(CLAUDE);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let unreadable = read_seen(Agent::Claude, &path, &ids());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            unreadable.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let missing = path.with_file_name("none.jsonl");
        assert!(
            read_seen(Agent::Claude, &missing, &ids())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn finds_a_codex_message_in_a_response_item_but_the_models_own() {
        let (_dir, path) = written(CODEX);
        assert_eq!(
            read_seen(Agent::Codex, &path, &ids()).unwrap(),
            [Seen {
                msg_id: ID.to_owned(),
                at_ms: timestamp_ms("2026-11-12T07:52:11.632Z"),
            }]
        );
        let output = r#"{"type":"response_item","payload":{"type":"function_call_output","output":"01a1cebe-27a9-7240-b4fa-000000000002"}}
{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{}"}}"#;
        let (_dir, path) = written(output);
        assert_eq!(
            read_seen(Agent::Codex, &path, &ids()).unwrap(),
            [Seen {
                msg_id: OTHER.to_owned(),
                at_ms: None
            }]
        );
        // Each agent reads only its own entries.
        assert!(read_seen(Agent::Claude, &path, &ids()).unwrap().is_empty());
        let (_dir, path) = written(CLAUDE);
        assert!(read_seen(Agent::Codex, &path, &ids()).unwrap().is_empty());
    }

    // A failed request, recorded from Claude Code 2.1.322 on a model stub that fails each request,
    // shortened: the prompt, the request's part, and the API error Claude Code writes for the reply.
    const CLAUDE_FAILED: &str = r#"{"type":"user","timestamp":"2026-11-12T13:02:10.484Z","message":{"role":"user","content":"<message msg_id=\"01a1cebe-27a9-7240-b4fa-5a2184ce744f\">x</message>"}}
{"type":"api-request-blob","message":{"role":"user","content":[{"type":"text","text":"<message msg_id=\"01a1cebe-27a9-7240-b4fa-5a2184ce744f\">x</message>"}]}}
{"type":"api-request","timestamp":"2026-11-12T13:02:10.542Z"}
{"type":"assistant","timestamp":"2026-11-12T13:02:10.601Z","isApiErrorMessage":true,"apiErrorStatus":400,"error":"unknown","message":{"model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"API Error: 400 the model stub fails each request"}]}}
{"type":"last-prompt"}"#;

    // The same from Codex 0.162.1: the prompt, then the turn's end with the error.
    const CODEX_FAILED: &str = r#"{"timestamp":"2026-11-12T13:02:46.300Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<message msg_id=\"01a1cebe-27a9-7240-b4fa-5a2184ce744f\">x</message>"}]}}
{"timestamp":"2026-11-12T13:02:46.400Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage"}}}
{"timestamp":"2026-11-12T13:02:46.500Z","type":"event_msg","payload":{"type":"task_complete","last_agent_message":null,"error":{"message":"the model stub fails each request"}}}"#;

    // The model's own entries, shortened.
    const CLAUDE_REPLY: &str = r#"{"type":"assistant","timestamp":"2026-11-12T13:02:11.000Z","message":{"model":"claude-opus-5-5","role":"assistant","content":[{"type":"text","text":"OK"}]}}"#;
    const CODEX_REPLY: &str = r#"{"timestamp":"2026-11-12T13:02:47.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"OK"}]}}"#;

    #[test]
    fn counts_a_message_only_once_the_models_own_entry_follows() {
        let id = [ID.to_owned()];
        for (agent, failed, reply, at) in [
            (
                Agent::Claude,
                CLAUDE_FAILED,
                CLAUDE_REPLY,
                "2026-11-12T13:02:10.484Z",
            ),
            (
                Agent::Codex,
                CODEX_FAILED,
                CODEX_REPLY,
                "2026-11-12T13:02:46.300Z",
            ),
        ] {
            let message = Seen {
                msg_id: ID.to_owned(),
                at_ms: timestamp_ms(at),
            };
            // Followed by nothing or only by an API error: in the file, but not seen.
            let first = failed.lines().next().unwrap();
            for text in [first, failed] {
                let (_dir, path) = written(text);
                assert!(
                    read_seen(agent, &path, &id).unwrap().is_empty(),
                    "{agent:?}"
                );
                assert_eq!(
                    read_written(agent, &path, &id).unwrap(),
                    std::slice::from_ref(&message),
                    "{agent:?}"
                );
            }
            // Followed by the model's reply, at once or after the error: seen, at its entry.
            for text in [format!("{first}\n{reply}"), format!("{failed}\n{reply}")] {
                let (_dir, path) = written(&text);
                assert_eq!(
                    read_seen(agent, &path, &id).unwrap(),
                    std::slice::from_ref(&message),
                    "{agent:?}"
                );
            }
            // A reply before the message is not the model's to it.
            let (_dir, path) = written(&format!("{reply}\n{failed}"));
            assert!(
                read_seen(agent, &path, &id).unwrap().is_empty(),
                "{agent:?}"
            );
        }
    }

    #[test]
    fn tells_the_models_own_entries() {
        let entry = |text: &str| -> Value { serde_json::from_str(text).unwrap() };
        assert!(from_model(Agent::Claude, &entry(CLAUDE_REPLY)));
        assert!(from_model(Agent::Codex, &entry(CODEX_REPLY)));
        assert!(!from_model(Agent::Claude, &entry(CODEX_REPLY)));
        assert!(!from_model(Agent::Codex, &entry(CLAUDE_REPLY)));
        let error = CLAUDE_FAILED.lines().nth(3).unwrap();
        assert!(!from_model(Agent::Claude, &entry(error)));
        // Either mark of an API error is enough.
        assert!(!from_model(
            Agent::Claude,
            &entry(&error.replace(r#""isApiErrorMessage":true,"#, ""))
        ));
        assert!(!from_model(
            Agent::Claude,
            &entry(&error.replace("<synthetic>", "claude-opus-5-5"))
        ));
        assert!(from_model(
            Agent::Claude,
            &entry(
                &error
                    .replace(r#""isApiErrorMessage":true,"#, "")
                    .replace("<synthetic>", "claude-opus-5-5")
            )
        ));
        for codex in [
            r#"{"type":"response_item","payload":{"type":"reasoning","summary":[]}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call","name":"exec_command"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch"}}"#,
        ] {
            assert!(from_model(Agent::Codex, &entry(codex)), "{codex}");
            let other = codex.replace("response_item", "event_msg");
            assert!(!from_model(Agent::Codex, &entry(&other)), "{other}");
        }
        for codex in [
            r#"{"type":"response_item","payload":{"type":"message","role":"user"}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call_output"}}"#,
            r#"{"type":"response_item","payload":{"type":"additional_tools","role":"developer"}}"#,
            r#"{"type":"response_item","payload":{}}"#,
        ] {
            assert!(!from_model(Agent::Codex, &entry(codex)), "{codex}");
        }
    }

    #[test]
    fn a_missing_file_shows_nothing_and_an_unreadable_one_fails() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.jsonl");
        assert!(
            read_seen(Agent::Codex, &missing, &ids())
                .unwrap()
                .is_empty()
        );
        assert!(read_seen(Agent::Codex, dir.path(), &ids()).is_err());
        assert!(
            read_written(Agent::Codex, &missing, &ids())
                .unwrap()
                .is_empty()
        );
        assert!(read_written(Agent::Codex, dir.path(), &ids()).is_err());
    }

    #[test]
    fn names_the_session_files() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("my project.d");
        fs::create_dir(&cwd).unwrap();
        let real = fs::canonicalize(&cwd).unwrap();
        let key: String = real
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        assert!(key.ends_with("-my-project-d"), "{key}");
        assert_eq!(
            claude_transcript(Path::new("/c"), &cwd, "s1").unwrap(),
            Path::new("/c/projects").join(key).join("s1.jsonl")
        );
        assert!(claude_project(Path::new("/c"), &home.path().join("none")).is_err());

        let codex = home.path().join(".codex");
        assert_eq!(codex_rollout(&codex, ID).unwrap(), None);
        let day = codex.join("sessions/2026/11/12");
        fs::create_dir_all(&day).unwrap();
        let rollout = day.join(format!("rollout-2026-11-12T14-52-10-{ID}.jsonl"));
        fs::write(&rollout, "").unwrap();
        fs::write(day.join(format!("other-{ID}.jsonl")), "").unwrap();
        fs::write(day.join(format!("rollout-{OTHER}.jsonl")), "").unwrap();
        assert_eq!(codex_rollout(&codex, ID).unwrap(), Some(rollout));
        assert_eq!(codex_rollout(&codex, "01a1").unwrap(), None);
    }

    #[test]
    fn reads_rfc_3339_times_in_utc() {
        assert_eq!(timestamp_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(timestamp_ms("1970-01-01T00:00:01.5Z"), Some(1_500));
        assert_eq!(
            timestamp_ms("2026-11-12T07:52:11.460Z"),
            Some(1_794_469_931_460)
        );
        assert_eq!(
            timestamp_ms("2000-03-01T00:00:00.000Z"),
            Some(951_868_800_000)
        );
        assert_eq!(
            timestamp_ms("2024-02-29T23:59:59.999Z"),
            Some(1_709_251_199_999)
        );
        for bad in [
            "2026-11-12T07:52:11.460",
            "2026-11-12 07:52:11Z",
            "2026-13-12T07:52:11Z",
            "2026-11-12T24:00:00Z",
            "2026-11-12T07:5x:11Z",
            "1969-12-31T23:59:59Z",
        ] {
            assert_eq!(timestamp_ms(bad), None, "{bad}");
        }
    }
}
