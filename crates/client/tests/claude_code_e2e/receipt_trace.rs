//! Opt-in Windows diagnostics for the own-text receipt tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    Debug,
}

impl TraceMode {
    pub fn from_env() -> Option<Self> {
        if !cfg!(windows) {
            return None;
        }
        match std::env::var("SKALD_E2E_TRACE_READ").as_deref() {
            Ok("1") => Some(Self::Stop),
            Ok("transcript") => Some(Self::Transcript),
            Ok("debug") => Some(Self::Debug),
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

    pub fn debug_args(&self) -> Option<[String; 2]> {
        if !matches!(self.mode, TraceMode::Debug) {
            return None;
        }
        let file = self.config.join("receipt-claude-debug.log");
        Some(["--debug-file".to_owned(), file.to_str()?.to_owned()])
    }

    pub fn failed(&self, solo: &Solo, helped: &Helped, id: MessageId, says: &str, resend: &str) {
        eprintln!("receipt trace: before retry");
        self.status(solo, id);
        let debug = matches!(self.mode, TraceMode::Debug);
        if debug {
            debug_stops(&self.config.join("receipt-claude-debug.log"));
        } else {
            self.stops();
        }
        let Some((file, content)) = transcript(&self.config.join("home/projects"), says) else {
            eprintln!("receipt trace: transcript unavailable or ambiguous");
            return;
        };
        let given = first_offer(solo, id);
        eprintln!("receipt trace: first_offer_ms={given:?}");
        if debug {
            let modified = fs::metadata(&file)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(unix_millis);
            let observed = unix_millis(SystemTime::now());
            eprintln!(
                "receipt trace: transcript snapshot_ms={observed:?} modified_ms={modified:?}"
            );
        }
        entries(&content, id, says, given, debug);

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

fn unix_millis(time: SystemTime) -> Option<u128> {
    Some(time.duration_since(UNIX_EPOCH).ok()?.as_millis())
}

fn timestamp(value: Option<&str>) -> Option<i64> {
    value?
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|time| time.as_millisecond())
}

fn debug_stops(file: &Path) {
    let mut any = false;
    if let Ok(log) = fs::read_to_string(file) {
        for line in log.lines() {
            let Some((at, message)) = line.split_once(' ') else {
                continue;
            };
            let Some((level, event)) = message.split_once(' ') else {
                continue;
            };
            if !matches!(level, "[DEBUG]" | "[INFO]" | "[WARN]" | "[ERROR]") {
                continue;
            }
            let class = if event.starts_with("Getting matching hook commands for Stop with query:")
            {
                "matching"
            } else if event.starts_with("Skipping Stop hook ") {
                "skipped"
            } else if event.starts_with("Stop hook cancelled") {
                "cancelled"
            } else if event.starts_with("Stop hook failed:") {
                "failed"
            } else {
                continue;
            };
            if let Some(at) = timestamp(Some(at)) {
                any = true;
                eprintln!("receipt trace: vendor Stop event={class} at_ms={at}");
            }
        }
    }
    if !any {
        eprintln!("receipt trace: vendor Stop event=unknown");
    }
}

fn stop_outcome(entry: &Value, at: Option<i64>) -> bool {
    let attachment = &entry["attachment"];
    if entry["type"] == "attachment" && attachment["hookEvent"] == "Stop" {
        let outcome = match attachment["type"].as_str() {
            Some("hook_success") => "success",
            Some("hook_blocking_error") => "blocked",
            Some("hook_non_blocking_error") => "failed",
            Some("hook_cancelled") => "cancelled",
            _ => "unknown",
        };
        let exit = match attachment["exitCode"].as_i64() {
            Some(0) => "success",
            Some(2) => "blocked",
            Some(_) => "other",
            None => "unknown",
        };
        let command = match attachment["command"].as_str() {
            Some(command) if command.contains("--wait") => "rewake",
            Some(command)
                if command.contains("--agent")
                    && command.contains("hook")
                    && command.contains("claude") =>
            {
                "sync"
            }
            _ => "unknown",
        };
        eprintln!(
            "receipt trace: vendor Stop outcome={outcome} exit={exit} command={command} at_ms={at:?}"
        );
        return true;
    }
    if entry["type"] == "system"
        && entry["subtype"] == "stop_hook_summary"
        && entry["hookLabel"] == "Stop"
    {
        let count = entry["hookCount"].as_u64();
        let errors = entry["hookErrors"].as_array().map(Vec::len);
        let duration = entry["totalDurationMs"].as_u64();
        eprintln!(
            "receipt trace: vendor Stop summary at_ms={at:?} count={count:?} errors={errors:?} duration_ms={duration:?}"
        );
        return true;
    }
    false
}

fn entries(content: &str, id: MessageId, says: &str, given: Option<i64>, debug: bool) {
    let mut saw_stop = false;
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
        let at = timestamp(entry["timestamp"].as_str());
        if debug && line.ends_with('\n') {
            saw_stop |= stop_outcome(&entry, at);
        }
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
    if debug && !saw_stop {
        eprintln!("receipt trace: vendor Stop outcome=unknown");
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
