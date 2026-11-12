//! Reads a session's own file, a Claude Code transcript or a Codex rollout, for the entries that
//! went to the model, and finds there the `msg_id`s of handed-over messages: a read confirmation
//! that needs no help from the model. It keeps no line it reads.
//!
//! Only these entries count, as the CLIs write them (Claude Code 2.1.322, Codex 0.162.1):
//! - Claude Code: a `user` entry (a prompt, a cross-session message, a tool result), a
//!   `hook_additional_context` attachment, and an `api-request-blob`, the part of a request to the
//!   model it holds. A `queue-operation` entry is a prompt still queued, and a `hook_success`
//!   attachment only a hook's output: neither reached the model.
//! - Codex: a `response_item` other than the model's own: a user or developer message, or a tool's
//!   output.

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
/// the order of its first entry. A file that does not exist yet shows none.
pub fn read_seen(agent: Agent, path: &Path, msg_ids: &[String]) -> io::Result<Vec<Seen>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut seen: Vec<Seen> = Vec::new();
    for line in BufReader::new(file).lines() {
        // A line still being written is not an entry yet.
        let Ok(entry) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        if !went_to_model(agent, &entry) {
            continue;
        }
        let text = entry.to_string();
        for msg_id in msg_ids {
            if text.contains(msg_id.as_str()) && !seen.iter().any(|s| s.msg_id == *msg_id) {
                seen.push(Seen {
                    msg_id: msg_id.clone(),
                    at_ms: entry["timestamp"].as_str().and_then(timestamp_ms),
                });
            }
        }
    }
    Ok(seen)
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
        let output = r#"{"type":"response_item","payload":{"type":"function_call_output","output":"01a1cebe-27a9-7240-b4fa-000000000002"}}"#;
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
