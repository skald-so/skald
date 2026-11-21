//! The real Claude Code as a running session of one of Ada's agents that gets messages over its
//! cross-session socket: in a pseudo-terminal, as a person runs it, or as the desktop app and VS
//! Code run it, `-p --input-format stream-json`, with or without `--permission-mode
//! bypassPermissions`. As in `claude.rs`: a temporary config home with no sign-in on a model stub,
//! the session's `skald mcp` its only MCP server. Unlike there, it keeps its session file, and
//! `skald hook claude` is also its `SessionStart` and `SessionEnd` hook, run as Claude Code runs
//! hooks: without the one-shot setting, so `SessionStart` starts the helper. Where the client
//! writes into no socket ([`SOCKET`]), a session's `skald` runs in one-shot mode, as in
//! `claude.rs`, unless the test runs its hooks ([`Options::hooks`]): there a helper would outlive
//! the session and keep its machine's directory from being removed.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{env, fs, process::Child, process::ChildStdin, process::Command, process::Stdio};
use std::{sync::Arc, sync::Mutex, thread};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::{Value, json};

use crate::client::MCP;
use crate::hooks::{CLAUDE_CODE, HOOK};
use crate::machine::{Machine, ONE_SHOT};
use crate::socket_model::{Model, conversation};
use crate::stub::{PLACEHOLDER, skald_tool};
use crate::world::AGENT;

/// The helper's idle time, in seconds, once no session is registered: a helper a test leaves exits
/// soon.
const HELPER_IDLE: &str = "20";

/// What Claude Code's prompt box starts with in a terminal.
const PROMPT_BOX: &str = "\u{276f}";

/// How long a turn may take.
const TURN_LIMIT: Duration = Duration::from_secs(120);

/// How long a terminal session sends nothing more once its turn ended.
const SETTLED: Duration = Duration::from_secs(5);

/// How long a session may take to exit once asked to.
const EXIT_LIMIT: Duration = Duration::from_secs(30);

/// Whether the client writes into Claude Code's cross-session socket here: on Windows the socket
/// is a named pipe, which it does not write to, and a session gets its messages by the hooks.
pub const SOCKET: bool = cfg!(unix);

/// How Claude Code runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// In a terminal.
    Terminal,
    /// As the desktop app and VS Code run it.
    App,
    /// As the desktop app and VS Code run it, bypassing permission prompts.
    AppBypassing,
}

/// The terminal's input, shared with what answers its cursor queries.
type Typing = Arc<Mutex<Box<dyn Write + Send>>>;

enum Running {
    Terminal {
        child: Box<dyn portable_pty::Child + Send + Sync>,
        input: Typing,
    },
    App {
        child: Child,
        input: Option<ChildStdin>,
    },
}

/// How a test's session differs from what [`Session::start`] runs.
#[derive(Default)]
pub struct Options {
    /// The `skald` its hooks run, if not the machine's; its `skald mcp` then runs in one-shot mode,
    /// so only the hooks start a helper, which runs this `skald`.
    pub hooks: Option<PathBuf>,
    /// Claude Code settings beside the hooks and permissions, such as `crossSessionInbound`.
    pub settings: serde_json::Map<String, Value>,
}

/// A running session; asked to exit, then killed, on drop.
pub struct Session {
    running: Running,
    output: Arc<Mutex<Vec<u8>>>,
}

impl Session {
    /// Starts `claude` on `machine` in `dir` on `model`, as `agent`, on `surface`. The session may
    /// run `true`, `sleep` and `echo` and call `skald`'s `inbox`; it takes no prompt yet.
    pub fn start(
        claude: &Path,
        machine: &Machine,
        dir: &Path,
        agent: &str,
        model: &Model,
        surface: Surface,
    ) -> Self {
        let place = (machine, dir);
        Self::start_with(claude, place, agent, model, surface, Options::default())
    }

    /// As [`Session::start`], with `options`.
    pub fn start_with(
        claude: &Path,
        (machine, dir): (&Machine, &Path),
        agent: &str,
        model: &Model,
        surface: Surface,
        options: Options,
    ) -> Self {
        // Each session of a test has a config home of its own.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let next = NEXT.fetch_add(1, Ordering::Relaxed);
        let config =
            (machine.root().join("tmp")).join(format!("socket-{agent}-{surface:?}-{next}"));
        let home = config.join("home");
        fs::create_dir_all(&home).expect("make Claude Code's config home");
        let named = |args: &[&str]| -> Vec<String> {
            [AGENT, agent]
                .iter()
                .chain(args)
                .map(|arg| (*arg).to_owned())
                .collect()
        };
        let mcp = machine.command_in(
            dir,
            &named(&[MCP]).iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let skald = lossy(mcp.get_program());
        let helped = SOCKET || options.hooks.is_some();
        let mut settings: Vec<(String, String)> = (mcp.get_envs())
            .filter(|(name, _)| !helped || *name != OsStr::new(ONE_SHOT))
            .filter_map(|(name, value)| Some((lossy(name), lossy(value?))))
            .collect();
        settings.push(("SKALD_HELPER_IDLE".to_owned(), HELPER_IDLE.to_owned()));
        let hooks_skald = (options.hooks.as_deref())
            .map_or_else(|| skald.clone(), |path| lossy(path.as_os_str()));

        let mcp_args: Vec<String> = mcp.get_args().map(lossy).collect();
        let mut server_env: serde_json::Map<String, Value> = (settings.iter())
            .map(|(name, value)| (name.clone(), json!(value)))
            .collect();
        if options.hooks.is_some() {
            server_env.insert(ONE_SHOT.to_owned(), json!("1"));
        }
        let mcp_file = config.join("mcp.json");
        let servers = json!({ "skald": {
            "type": "stdio", "command": skald, "args": mcp_args, "env": server_env,
        } });
        write(&mcp_file, &json!({ "mcpServers": servers }));
        let hook = json!([{ "hooks": [{
            "type": "command", "command": hooks_skald, "args": named(&[HOOK, CLAUDE_CODE]),
        }] }]);
        let events = ["SessionStart", "SessionEnd", "PostToolUse", "Stop"];
        let hooks: serde_json::Map<String, Value> = events
            .iter()
            .map(|event| ((*event).to_owned(), hook.clone()))
            .collect();
        let allowed = json!([
            "Bash(true)",
            "Bash(sleep:*)",
            "Bash(echo:*)",
            skald_tool("inbox")
        ]);
        let settings_file = config.join("settings.json");
        let mut given = options.settings;
        given.insert("hooks".to_owned(), Value::Object(hooks));
        given.insert("permissions".to_owned(), json!({ "allow": allowed }));
        write(&settings_file, &Value::Object(given));
        // No onboarding, the placeholder approved as Claude Code keeps it, and the folder trusted:
        // the terminal asks nothing first.
        let tail = &PLACEHOLDER[PLACEHOLDER.len() - 20..];
        // Claude Code names the folder as it is given, or resolved on Unix; on Windows with `/`
        // or `\`, so the folder is trusted under each name.
        let real = fs::canonicalize(dir).expect("the agent's folder");
        let real = lossy(real.as_os_str());
        let given = lossy(dir.as_os_str());
        let names = [given.as_str(), real.strip_prefix(r"\\?\").unwrap_or(&real)];
        let trusted: serde_json::Map<String, Value> = (names.iter())
            .flat_map(|name| [name.to_string(), name.replace('\\', "/")])
            .map(|name| (name, json!({ "hasTrustDialogAccepted": true })))
            .collect();
        write(
            &home.join(".claude.json"),
            &json!({
                "hasCompletedOnboarding": true, "theme": "dark",
                "customApiKeyResponses": { "approved": [tail], "rejected": [] },
                "projects": trusted,
            }),
        );
        settings.extend([
            ("PATH".to_owned(), env::var("PATH").unwrap_or_default()),
            ("HOME".to_owned(), lossy(home.as_os_str())),
            ("CLAUDE_CONFIG_DIR".to_owned(), lossy(home.as_os_str())),
            ("ANTHROPIC_BASE_URL".to_owned(), model.url().to_owned()),
            ("ANTHROPIC_API_KEY".to_owned(), PLACEHOLDER.to_owned()),
            ("TERM".to_owned(), "xterm-256color".to_owned()),
        ]);
        let mut args: Vec<String> = Vec::new();
        if surface != Surface::Terminal {
            args.extend(
                [
                    "-p",
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                ]
                .map(str::to_owned),
            );
            args.extend(["--verbose", "--include-hook-events"].map(str::to_owned));
        }
        if surface == Surface::AppBypassing {
            args.extend(["--permission-mode", "bypassPermissions"].map(str::to_owned));
        }
        args.extend(["--setting-sources", "local", "--strict-mcp-config"].map(str::to_owned));
        args.extend([
            "--mcp-config".to_owned(),
            lossy(mcp_file.as_os_str()),
            "--settings".to_owned(),
            lossy(settings_file.as_os_str()),
            "--tools".to_owned(),
            "Bash".to_owned(),
        ]);

        let output = Arc::new(Mutex::new(Vec::new()));
        let running = if surface == Surface::Terminal {
            let pair = (native_pty_system())
                .openpty(PtySize {
                    rows: 50,
                    cols: 200,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("open a pseudo-terminal");
            let mut command = CommandBuilder::new(claude);
            command.cwd(dir);
            command.env_clear();
            for (name, value) in &settings {
                command.env(name, value);
            }
            command.args(&args);
            let child = pair.slave.spawn_command(command).expect("run Claude Code");
            drop(pair.slave);
            let input: Typing = Arc::new(Mutex::new(
                pair.master.take_writer().expect("the terminal's input"),
            ));
            read_answering(
                pair.master
                    .try_clone_reader()
                    .expect("the terminal's output"),
                &output,
                &input,
            );
            // The terminal lives as long as its reader and writer.
            std::mem::forget(pair.master);
            Running::Terminal { child, input }
        } else {
            let mut child = Command::new(claude)
                .current_dir(dir)
                .env_clear()
                .envs(settings.iter().map(|(name, value)| (name, value)))
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("run Claude Code");
            read_into(child.stdout.take().expect("piped"), &output);
            read_into(child.stderr.take().expect("piped"), &output);
            let input = child.stdin.take();
            Running::App { child, input }
        };
        Session { running, output }
    }

    /// The person gives the session a prompt: types it and Enter in the terminal, or sends it as
    /// the apps do.
    pub fn prompt(&mut self, text: &str) {
        match &mut self.running {
            Running::Terminal { .. } => {
                // Claude Code reads typing once its prompt box shows; Enter that comes with the
                // text is taken as part of a paste.
                let deadline = Instant::now() + TURN_LIMIT;
                while !self.output().contains(PROMPT_BOX) {
                    assert!(
                        Instant::now() < deadline,
                        "no prompt box: {}",
                        self.output()
                    );
                    thread::sleep(Duration::from_millis(200));
                }
                thread::sleep(Duration::from_secs(1));
                let Running::Terminal { input, .. } = &self.running else {
                    unreachable!()
                };
                typed(input, text.as_bytes()).expect("type the prompt");
                thread::sleep(Duration::from_secs(1));
                typed(input, b"\r").expect("press Enter");
            }
            Running::App { input, .. } => {
                let line =
                    json!({ "type": "user", "message": { "role": "user", "content": text } });
                let input = input.as_mut().expect("the session's input is open");
                writeln!(input, "{line}").expect("send the prompt");
                input.flush().expect("send the prompt");
            }
        }
    }

    /// The person dismisses the permission prompt the terminal session shows: Esc, as Claude Code
    /// takes it, so the session then exits when asked.
    pub fn dismiss_prompt(&mut self) {
        let Running::Terminal { input, .. } = &self.running else {
            panic!("only a terminal session shows a permission prompt");
        };
        typed(input, b"\x1b").expect("press Esc");
        thread::sleep(Duration::from_secs(1));
    }

    /// Waits until a turn holding `text` reached `model`; fails after a while, showing what the
    /// session printed.
    #[track_caller]
    pub fn wait_turn_holding(&self, model: &Model, text: &str) -> Value {
        let deadline = Instant::now() + TURN_LIMIT;
        loop {
            let turns = model.turns();
            if let Some(turn) = turns
                .into_iter()
                .find(|body| conversation(body).contains(text))
            {
                return turn;
            }
            assert!(
                Instant::now() < deadline,
                "no turn held {text:?}; the session printed: {}",
                self.output()
            );
            thread::sleep(Duration::from_millis(200));
        }
    }

    /// Waits until a turn holding `text` reached `model` and the session's `ended`th turn ended:
    /// the apps' stream has its result; in a terminal, the session sent nothing more for a while.
    #[track_caller]
    pub fn wait_turn_end(&self, model: &Model, text: &str, ended: usize) {
        self.wait_turn_holding(model, text);
        let deadline = Instant::now() + TURN_LIMIT;
        if matches!(self.running, Running::Terminal { .. }) {
            let mut seen = model.turns().len();
            let mut still = Instant::now();
            while still.elapsed() < SETTLED {
                assert!(
                    Instant::now() < deadline,
                    "the turn did not end: {}",
                    self.output()
                );
                thread::sleep(Duration::from_millis(200));
                let now = model.turns().len();
                if now != seen {
                    (seen, still) = (now, Instant::now());
                }
            }
            return;
        }
        while self.results() < ended {
            assert!(
                Instant::now() < deadline,
                "the turn did not end: {}",
                self.output()
            );
            thread::sleep(Duration::from_millis(200));
        }
    }

    /// The results of turns in the apps' stream so far.
    fn results(&self) -> usize {
        (self.output().lines())
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|value| value["type"] == "result")
            .count()
    }

    /// What the session printed so far.
    pub fn output(&self) -> String {
        String::from_utf8_lossy(
            &self
                .output
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
        .into_owned()
    }

    /// The session's hook events of `event` so far, as the apps' stream has them.
    pub fn hook_responses(&self, event: &str) -> Vec<Value> {
        (self.output().lines())
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|value| value["subtype"] == "hook_response" && value["hook_event"] == event)
            .collect()
    }

    /// Asks the session to exit, as a person or the apps do, and waits for it; kills it if it does
    /// not exit in time. Gives whether it exited by itself.
    pub fn end(&mut self) -> bool {
        match &mut self.running {
            Running::Terminal { input, .. } => {
                for _ in 0..2 {
                    let _ = typed(input, b"\x03");
                    thread::sleep(Duration::from_millis(500));
                }
            }
            Running::App { input, .. } => drop(input.take()),
        }
        let deadline = Instant::now() + EXIT_LIMIT;
        while Instant::now() < deadline {
            let exited = match &mut self.running {
                Running::Terminal { child, .. } => child.try_wait().ok().flatten().is_some(),
                Running::App { child, .. } => child.try_wait().ok().flatten().is_some(),
            };
            if exited {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        match &mut self.running {
            Running::Terminal { child, .. } => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Running::App { child, .. } => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        false
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.end();
    }
}

/// Types `bytes` into the terminal whose input is `input`.
fn typed(input: &Typing, bytes: &[u8]) -> std::io::Result<()> {
    let mut input = input
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    input.write_all(bytes)?;
    input.flush()
}

/// As [`read_into`] for a terminal's output: answers each query of where the cursor is, which a
/// pseudo-console makes before it shows anything, with its start, as a new terminal does.
fn read_answering(
    mut from: impl Read + Send + 'static,
    into: &Arc<Mutex<Vec<u8>>>,
    input: &Typing,
) {
    const QUERY: &[u8] = b"\x1b[6n";
    let (into, input) = (Arc::clone(into), Arc::clone(input));
    thread::spawn(move || {
        let mut piece = vec![0; 8192];
        let mut seen: usize = 0;
        while let Ok(count) = from.read(&mut piece) {
            if count == 0 {
                break;
            }
            let mut output = into.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            output.extend_from_slice(&piece[..count]);
            // The queries in what is new, a query cut by the last read included.
            let from_byte = seen.saturating_sub(QUERY.len() - 1);
            let new = output[from_byte..]
                .windows(QUERY.len())
                .filter(|w| *w == QUERY);
            for _ in new {
                let _ = typed(&input, b"\x1b[1;1R");
            }
            seen = output.len();
        }
    });
}

/// Reads `from` into `into` to its end, on a thread of its own.
fn read_into(mut from: impl Read + Send + 'static, into: &Arc<Mutex<Vec<u8>>>) {
    let into = Arc::clone(into);
    thread::spawn(move || {
        let mut piece = vec![0; 8192];
        loop {
            match from.read(&mut piece) {
                Ok(0) | Err(_) => break,
                Ok(count) => (into.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
                    .extend_from_slice(&piece[..count]),
            }
        }
    });
}

fn lossy(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

fn write(path: &Path, value: &Value) {
    let text = serde_json::to_string_pretty(value).expect("JSON encodes");
    fs::write(path, text).expect("write Claude Code's config");
}
