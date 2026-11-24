//! Opt-in Windows diagnostics for the own-text receipt tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::helped::{Helped, IDLE_SETTING};
use crate::hooks::{CLAUDE_CODE, HOOK};
use crate::lines::refers_to;
use crate::solo::{Solo, WEB};
use crate::world::AGENT;

// Preserve the hook's input and output streams and report only lifecycle classes.
const STOP_SHIM: &str = r#"
const { appendFileSync } = require('node:fs');
const { spawnSync } = require('node:child_process');
const [trace, executable, ...args] = process.argv.slice(2);
function record(event) {
  try {
    appendFileSync(trace, Date.now() + ' ' + event + '\n');
  } catch {}
}
record('started');
const { status, error } = spawnSync(executable, args, { stdio: 'inherit', windowsHide: true });
record(error ? 'spawn-error' : status === 0 ? 'success' : status === 2 ? 'blocked' : 'failed');
process.exitCode = status ?? 1;
"#;

#[derive(Clone, Copy)]
pub enum TraceMode {
    Stop,
    Transcript,
}

impl TraceMode {
    pub fn from_env() -> Option<Self> {
        if !cfg!(windows) {
            return None;
        }
        match std::env::var("SKALD_E2E_TRACE_READ").as_deref() {
            Ok("1") => Some(Self::Stop),
            Ok("transcript") => Some(Self::Transcript),
            _ => None,
        }
    }
}

pub struct Trace {
    mode: TraceMode,
    config: PathBuf,
    cwd: PathBuf,
    wrapped: bool,
}

impl Trace {
    pub fn new(mode: TraceMode, config: &Path, cwd: &Path) -> Self {
        Self {
            mode,
            config: config.to_owned(),
            cwd: cwd.to_owned(),
            wrapped: false,
        }
    }

    pub fn wrap_stop(&mut self, executable: &str, args: Vec<String>) -> Option<Value> {
        if !matches!(self.mode, TraceMode::Stop) {
            return None;
        }
        let node = std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join("node.exe"))
            .find(|file| file.is_file())?;
        let shim = self.config.join("receipt-stop.cjs");
        let log = self.config.join("receipt-stop.log");
        fs::write(&shim, STOP_SHIM).ok()?;
        let node = node.to_str()?;
        let mut command = vec![shim.to_str()?, log.to_str()?, executable];
        command.extend(args.iter().map(String::as_str));
        self.wrapped = true;
        Some(json!({ "type": "command", "command": node, "args": command }))
    }

    pub fn failed(&self, solo: &Solo, helped: &Helped, id: MessageId, says: &str, resend: &str) {
        eprintln!("receipt trace: before retry");
        self.status(solo, id);
        self.stops();
        let Some((file, content)) = transcript(&self.config.join("home/projects"), says) else {
            eprintln!("receipt trace: transcript unavailable or ambiguous");
            return;
        };
        let given = first_offer(solo, id);
        eprintln!("receipt trace: first_offer_ms={given:?}");
        entries(&content, id, says, given);

        let outcome = self.retry_stop(helped, &file, resend);
        eprintln!("receipt trace: retry stop={outcome:?}");
        self.status(solo, id);
    }

    fn status(&self, solo: &Solo, id: MessageId) {
        let status = solo
            .machine()
            .command_in(&self.cwd, &["message-status", "--json", &id.to_string()])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());
        let read = status
            .as_ref()
            .and_then(|status| status["agent"]["read"].as_u64());
        let handed = status
            .as_ref()
            .and_then(|status| status["handed_to"].as_array())
            .map(Vec::len);
        eprintln!(
            "receipt trace: status_available={} read_at={read:?} handed_count={handed:?}",
            status.is_some()
        );
    }

    fn stops(&self) {
        eprintln!("receipt trace: stop_wrapper_configured={}", self.wrapped);
        let Ok(log) = fs::read_to_string(self.config.join("receipt-stop.log")) else {
            eprintln!("receipt trace: no Stop events recorded");
            return;
        };
        for line in log.lines() {
            if let Some((
                at,
                event @ ("started" | "success" | "blocked" | "failed" | "spawn-error"),
            )) = line.split_once(' ')
                && let Ok(at) = at.parse::<i64>()
            {
                eprintln!("receipt trace: stop {event} at_ms={at}");
            }
        }
    }

    fn retry_stop(&self, helped: &Helped, file: &Path, resend: &str) -> Option<&'static str> {
        let session = file.file_stem()?.to_str()?;
        let input = json!({
            "hook_event_name": "Stop",
            "session_id": session,
            "transcript_path": file.to_str()?,
            "cwd": self.cwd.to_str()?,
            "stop_hook_active": false,
        });
        let mut command = helped.command_in(
            &self.cwd,
            &[(IDLE_SETTING, "20"), ("SKALD_RESEND_AFTER", resend)],
            &[AGENT, WEB, HOOK, CLAUDE_CODE],
        );
        stop(&mut command, &input)
    }
}

fn transcript(projects: &Path, says: &str) -> Option<(PathBuf, String)> {
    let mut files = fs::read_dir(projects)
        .ok()?
        .flatten()
        .filter(|project| project.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|project| fs::read_dir(project.path()).ok())
        .flat_map(|project| project.flatten())
        .filter(|file| file.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|file| file.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|path| {
            fs::read_to_string(&path)
                .ok()
                .map(|contents| (path, contents))
        })
        .collect::<Vec<_>>();
    if files.len() > 1 {
        files.retain(|(_, contents)| contents.contains(says));
    }
    (files.len() == 1).then(|| files.remove(0))
}

fn first_offer(solo: &Solo, id: MessageId) -> Option<i64> {
    let db = Connection::open_with_flags(
        solo.machine().state_dir().join("state.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    db.busy_timeout(Duration::from_millis(500)).ok()?;
    db.query_row(
        "SELECT handover.given FROM handover JOIN kept ON handover.seq = kept.seq
         WHERE kept.msg_id = ?1 AND handover.agent = ?2",
        [id.to_string().as_str(), WEB],
        |row| row.get(0),
    )
    .ok()
}

fn entries(content: &str, id: MessageId, says: &str, given: Option<i64>) {
    for (index, line) in content.split_inclusive('\n').enumerate() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            eprintln!(
                "receipt trace: entry {index} unreadable complete={}",
                line.ends_with('\n')
            );
            continue;
        };
        let kind = match entry["type"].as_str() {
            Some(kind @ ("user" | "assistant" | "attachment" | "queue-operation")) => kind,
            _ => "other",
        };
        let attachment = match entry["attachment"]["type"].as_str() {
            Some(
                kind @ ("queued_command"
                | "hook_success"
                | "hook_non_blocking_error"
                | "hook_blocking_error"),
            ) => kind,
            Some(_) => "other",
            None => "none",
        };
        let operation = match entry["operation"].as_str() {
            Some(kind @ ("enqueue" | "dequeue")) => kind,
            Some(_) => "other",
            None => "none",
        };
        let body = match kind {
            "user" | "assistant" => &entry["message"]["content"],
            "attachment" => &entry["attachment"]["prompt"],
            _ => &entry["content"],
        };
        let shape = match body {
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Null => "none",
            _ => "other",
        };
        let has_reference = line.split('#').skip(1).any(|after| {
            after
                .split(|ch: char| !ch.is_ascii_alphanumeric())
                .next()
                .is_some_and(|reference| refers_to(reference, id))
        });
        let at = entry["timestamp"]
            .as_str()
            .and_then(|value| value.parse::<jiff::Timestamp>().ok())
            .map(|time| time.as_millisecond());
        let after_offer = at.zip(given).map(|(at, given)| at >= given);
        eprintln!(
            "receipt trace: entry {index} type={kind} attachment={attachment} operation={operation} shape={shape} fixture_text={} reference={has_reference} channel={} rewake={} at_ms={at:?} at_or_after_offer={after_offer:?} sidechain={:?} api_error={:?} complete={}",
            line.contains(says),
            line.contains("<channel source="),
            line.contains("hook blocking error from command"),
            entry["isSidechain"].as_bool(),
            entry["isApiErrorMessage"].as_bool(),
            line.ends_with('\n')
        );
    }
}

fn stop(command: &mut Command, input: &Value) -> Option<&'static str> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let written = child
        .stdin
        .take()
        .is_some_and(|stdin| serde_json::to_writer(stdin, input).is_ok());
    let start = Instant::now();
    while written && start.elapsed() < Duration::from_secs(20) {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Some(match status.code() {
                    Some(0) => "success",
                    Some(2) => "blocked",
                    _ => "failed",
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Some(if written { "no-exit" } else { "no-input" })
}
