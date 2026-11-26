//! The real Claude Code, run as a session of one of Ada's agents on the model stub: `claude -p` in a
//! temporary config home with no sign-in, never the user's, the session's `skald mcp` as its only
//! MCP server and `skald hook claude` as its `PostToolUse` and `Stop` hooks.
//!
//! These tests run outside `script/test`: they are ignored unless run with `--run-ignored only`.
//! Without Claude Code on `PATH`, each says it is skipped and passes, unless
//! `SKALD_E2E_REQUIRED=1`, as in their CI job, makes the skip a failure.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::client::MCP;
use crate::hooks::{CLAUDE_CODE, HOOK};
use crate::machine::Machine;
use crate::stub::{PLACEHOLDER, Stub, skald_tool};
use crate::world::AGENT;

/// The setting that turns a skip into a failure.
pub const REQUIRED: &str = "SKALD_E2E_REQUIRED";

/// How long a session may run.
const SESSION_LIMIT: Duration = Duration::from_secs(180);

/// The prompt of every session: the stub, not the prompt, decides what the session does.
pub const PROMPT: &str = "Run the steps of the test.";

/// Claude Code's binary on `PATH`, if any. Without it, says that the test is skipped, or fails if
/// [`REQUIRED`] is set.
pub fn claude_code() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["claude.exe", "claude.cmd"]
    } else {
        &["claude"]
    };
    on_path(names, "Claude Code")
}

/// The first file on `PATH` of `names`, the names of the program `what`, if any. Without it, says
/// that the test is skipped, or fails if [`REQUIRED`] is set.
pub fn on_path(names: &[&str], what: &str) -> Option<PathBuf> {
    let found = (env::var_os("PATH").iter())
        .flat_map(env::split_paths)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file());
    if found.is_none() {
        let required = env::var_os(REQUIRED).is_some_and(|value| !value.is_empty() && value != "0");
        assert!(!required, "{what} is not on PATH, and {REQUIRED} is set");
        eprintln!("skipped: {what} is not on PATH");
    }
    found
}

/// A finished session.
#[derive(Debug)]
pub struct Ended {
    pub success: bool,
    /// Its stream of JSON events.
    pub stdout: String,
    pub stderr: String,
}

/// A running Claude Code session; killed on drop.
pub struct Claude {
    child: Child,
    started: Instant,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Claude {
    /// Starts `claude` on `machine` in `dir`, on `stub`, as the agent `agent` or, without one, as
    /// the agent its `skald mcp` registers by default; its `skald mcp` and hooks run as the
    /// machine's runs do. The session may run `true` and `sleep` and call the MCP server's tools.
    pub fn start(
        claude: &Path,
        machine: &Machine,
        dir: &Path,
        agent: Option<&str>,
        stub: &Stub,
    ) -> Self {
        let config = machine
            .root()
            .join("tmp")
            .join(format!("claude-{}", dir_name(dir)));
        let home = config.join("config-home");
        fs::create_dir_all(&home).expect("make Claude Code's config home");
        let with = |command: &[&'static str]| -> Vec<String> {
            let named = agent.map(|agent| vec![AGENT.to_owned(), agent.to_owned()]);
            let rest = command.iter().map(|arg| (*arg).to_owned());
            named.unwrap_or_default().into_iter().chain(rest).collect()
        };

        let mcp_args = with(&[MCP]);
        let mcp_args: Vec<&str> = mcp_args.iter().map(String::as_str).collect();
        let mcp = machine.command_in(dir, &mcp_args);
        let skald = mcp.get_program().to_owned();
        let env: Map<String, Value> = (mcp.get_envs())
            .filter_map(|(name, value)| Some((text(name), json!(text(value?)))))
            .collect();
        let server =
            json!({ "type": "stdio", "command": text(&skald), "args": args_of(&mcp), "env": env });
        let mcp_config = config.join("mcp.json");
        write_json(&mcp_config, &json!({ "mcpServers": { "skald": server } }));

        let hook = json!([{ "hooks": [{
            "type": "command", "command": text(&skald), "args": with(&[HOOK, CLAUDE_CODE]),
        }] }]);
        let settings = config.join("settings.json");
        write_json(
            &settings,
            &json!({
                "hooks": { "PostToolUse": hook, "Stop": hook },
                "permissions": { "allow": ["Bash(true)", "Bash(sleep:*)", skald_tool("inbox"), skald_tool("send")] },
            }),
        );

        let mut command = Command::new(claude);
        command
            .current_dir(dir)
            .env_clear()
            .envs(
                mcp.get_envs()
                    .filter_map(|(name, value)| Some((name, value?))),
            )
            .env("PATH", env::var_os("PATH").unwrap_or_default())
            .env("CLAUDE_CONFIG_DIR", &home)
            .env("ANTHROPIC_BASE_URL", stub.url())
            .env("ANTHROPIC_API_KEY", PLACEHOLDER)
            .args(["-p", PROMPT, "--output-format", "stream-json", "--verbose"])
            .args([
                "--setting-sources",
                "local",
                "--strict-mcp-config",
                "--mcp-config",
            ])
            .arg(&mcp_config)
            .arg("--settings")
            .arg(&settings)
            .args([
                "--disable-slash-commands",
                "--tools",
                "Bash",
                "--no-session-persistence",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("run Claude Code");
        let stdout = collect(child.stdout.take().expect("stdout is piped"));
        let stderr = collect(child.stderr.take().expect("stderr is piped"));
        Claude {
            child,
            started: Instant::now(),
            stdout,
            stderr,
        }
    }

    /// Waits for the session to end, for up to [`SESSION_LIMIT`] from its start.
    #[track_caller]
    pub fn finish(mut self) -> Ended {
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("wait for Claude Code") {
                break status;
            }
            if self.started.elapsed() > SESSION_LIMIT {
                let _ = self.child.kill();
                panic!(
                    "the session ran for more than {SESSION_LIMIT:?}: {}",
                    self.output(&self.stdout)
                );
            }
            thread::sleep(Duration::from_millis(100));
        };
        // The readers end with the outputs.
        thread::sleep(Duration::from_millis(200));
        Ended {
            success: status.success(),
            stdout: self.output(&self.stdout),
            stderr: self.output(&self.stderr),
        }
    }

    fn output(&self, of: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8_lossy(&of.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
            .into_owned()
    }
}

impl Drop for Claude {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reads `from` to its end on a thread of its own.
fn collect(mut from: impl Read + Send + 'static) -> Arc<Mutex<Vec<u8>>> {
    let into = Arc::new(Mutex::new(Vec::new()));
    let writing = Arc::clone(&into);
    thread::spawn(move || {
        let mut piece = [0; 4096];
        while let Ok(count) = from.read(&mut piece) {
            if count == 0 {
                break;
            }
            (writing
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()))
            .extend_from_slice(&piece[..count]);
        }
    });
    into
}

fn text(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

fn args_of(command: &Command) -> Vec<String> {
    command.get_args().map(text).collect()
}

/// The last parts of `dir`'s path, as a file name.
fn dir_name(dir: &Path) -> String {
    let parts: Vec<String> = dir
        .components()
        .rev()
        .take(2)
        .map(|part| text(part.as_os_str()))
        .collect();
    parts.join("-")
}

fn write_json(path: &Path, value: &Value) {
    let written = serde_json::to_string_pretty(value).expect("JSON encodes");
    fs::write(path, written).expect("write Claude Code's config");
}
