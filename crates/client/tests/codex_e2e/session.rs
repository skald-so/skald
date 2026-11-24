#[cfg(windows)]
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_proto::{ChannelId, MessageId};

use crate::capture::Capture;
use crate::fixtures::{CHANNEL, SEND, TO};
use crate::hooks::{Bob, REVIEW};
use crate::host::{Live, WAIT};
use crate::machine::Machine;
use crate::solo::API;
use crate::stub::{ResponsesReply, Step, Stub};
use crate::talk::one_message_sent;
use crate::terminal::Terminal;
use crate::world::AGENT;

pub const SIGNAL: &str =
    "skald: messages wait for you: call the inbox tool, or skald recv, to read them";
pub const READY: &str = "Use the fixture model.";
pub const QUIET: Duration = Duration::from_secs(4);

pub fn codex_path() -> Option<PathBuf> {
    #[cfg(not(windows))]
    return crate::claude::on_path(&["codex"], "Codex");
    #[cfg(windows)]
    return crate::claude::on_path_or(&["codex.exe"], "Codex", || {
        let path = env::var_os("PATH")?;
        windows_codex(env::split_paths(&path), env::consts::ARCH)
    });
}

fn windows_codex(dirs: impl Iterator<Item = PathBuf>, arch: &str) -> Option<PathBuf> {
    let npm_arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    let target = format!("{arch}-pc-windows-msvc");
    let winget = format!("codex-{target}.exe");
    let platform = format!("@openai/codex-win32-{npm_arch}/vendor/{target}/bin/codex.exe");
    for dir in dirs {
        let exe = dir.join(&winget);
        if exe.is_file() {
            return Some(exe);
        }
        if dir.join("codex.cmd").is_file() {
            let modules = if dir.ends_with("node_modules/.bin") {
                dir.parent().expect("npm modules").to_owned()
            } else {
                dir.join("node_modules")
            };
            if let Some(native) = [modules.clone(), modules.join("@openai/codex/node_modules")]
                .iter()
                .map(|modules| modules.join(&platform))
                .find(|native| native.is_file())
            {
                return Some(native);
            }
        }
    }
    None
}

pub enum PermissionPath {
    Hook,
    #[cfg(unix)]
    Daemon,
}

#[derive(Default)]
pub struct SessionOptions {
    pub permission: Option<PermissionPath>,
    pub resend: Option<u64>,
    pub capture: bool,
}

impl SessionOptions {
    pub const CAPTURE: Self = Self {
        permission: None,
        resend: None,
        capture: true,
    };
}

pub struct Session {
    // Drop the terminal before the app-server and the temporary homes.
    pub terminal: Terminal,
    #[cfg(unix)]
    daemon: Option<Daemon>,
    pub live: Live,
    capture: Option<Capture>,
}

impl Session {
    pub fn start(
        id: u64,
        login: &str,
        program: PathBuf,
        stub: Stub,
        SessionOptions {
            permission,
            resend,
            capture,
        }: SessionOptions,
    ) -> Self {
        let mut live = Live::new(id, login, program);
        live.stub = stub;
        let mut settings = live.settings(&[]);
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            settings.push((name.to_owned(), live.stub.url().to_owned()));
        }
        settings.push(("NO_PROXY".to_owned(), "127.0.0.1,localhost".to_owned()));
        if let Some(resend) = resend {
            settings.push(("SKALD_RESEND_AFTER".to_owned(), resend.to_string()));
        }
        #[cfg(windows)]
        {
            let root = env::var_os("SystemRoot").expect("Windows SystemRoot");
            let system = PathBuf::from(&root).join("System32");
            let path = settings
                .iter_mut()
                .find(|(key, _)| key == "PATH")
                .expect("PATH");
            path.1 = env::join_paths(env::split_paths(&path.1).chain([system.clone()]))
                .expect("CLI search path")
                .to_string_lossy()
                .into_owned();
            settings.push((
                "COMSPEC".to_owned(),
                system.join("cmd.exe").to_string_lossy().into_owned(),
            ));
        }

        live.set_up();
        live.configure(&settings);
        let capture = capture.then(|| Capture::install(&live, &mut settings));
        if permission.is_some() {
            let config = live.home().join("config.toml");
            let text = fs::read_to_string(&config).expect("fixture config");
            let mut config_doc: toml_edit::DocumentMut = text.parse().expect("fixture TOML");
            config_doc["approval_policy"] = toml_edit::value("on-request");
            config_doc["sandbox_mode"] = toml_edit::value("read-only");
            if cfg!(windows) {
                config_doc["windows"]["sandbox"] = toml_edit::value("unelevated");
            }
            #[cfg(unix)]
            if matches!(permission, Some(PermissionPath::Daemon)) {
                config_doc["hooks"]
                    .as_table_like_mut()
                    .expect("hooks")
                    .remove("PermissionRequest");
            }
            fs::write(config, config_doc.to_string()).expect("write fixture permissions");
        }

        #[cfg(unix)]
        let daemon = matches!(permission, Some(PermissionPath::Daemon))
            .then(|| Daemon::start(&live, &settings));

        let mut command = live.command(&live.program, &settings);
        #[cfg(unix)]
        if daemon.is_none() {
            command.arg("--no-daemon");
        }
        #[cfg(windows)]
        command.arg("--no-daemon");
        command.arg("-C").arg(live.dir()).arg(READY);
        let terminal = Terminal::spawn(&command, live.dir());

        Self {
            terminal,
            #[cfg(unix)]
            daemon,
            live,
            capture,
        }
    }

    pub fn hook_stdout(&self) -> Vec<(String, Vec<u8>)> {
        self.capture
            .as_ref()
            .expect("hook capture enabled")
            .outputs()
    }

    pub fn wait_hook_stdout(&self, event: &str) -> Vec<u8> {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some((_, bytes)) = self
                .hook_stdout()
                .into_iter()
                .find(|(name, bytes)| name == event && !bytes.is_empty())
            {
                return bytes;
            }
            assert!(
                Instant::now() < deadline,
                "{event} printed nothing. Terminal: {}",
                self.recent_terminal_output()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn prompt(&self, text: &str) {
        self.terminal.send(text.as_bytes());
        // Codex treats Enter in the same input burst as part of the paste.
        thread::sleep(Duration::from_millis(300));
        self.terminal.enter();
    }

    pub fn model(&self) -> &Stub {
        &self.live.stub
    }

    pub fn wait_request(&self, from: usize, holds: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(turn) = self.model().requests().into_iter().skip(from).find(&holds) {
                return turn;
            }
            assert!(
                Instant::now() < deadline,
                "no matching request. Terminal: {}",
                self.recent_terminal_output()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn wait_idle(&self, earlier_completions: usize) {
        let deadline = Instant::now() + WAIT;
        loop {
            if self.completions() > earlier_completions {
                // Give Codex time to finish its Stop hook after persisting the rollout event.
                thread::sleep(Duration::from_millis(500));
                return;
            }
            assert!(
                Instant::now() < deadline,
                "no finished turn. Terminal: {}",
                self.recent_terminal_output()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn completions(&self) -> usize {
        rollout(&self.live.home().join("sessions"))
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|entry| {
                entry["type"] == "event_msg"
                    && matches!(
                        entry["payload"]["type"].as_str(),
                        Some("task_complete" | "turn_aborted")
                    )
            })
            .count()
    }

    pub fn send(&self, text: &str) -> MessageId {
        crate::hooks::api_sends(&self.live.codex.web.solo, &self.live.agent(), text)
    }

    pub fn send_in(&self, channel: ChannelId, peer: Option<&Bob>, text: &str) -> MessageId {
        let solo = &self.live.codex.web.solo;
        let sender_dir = solo.dir(API);
        let (machine, dir, agent) = peer
            .map_or((solo.machine(), sender_dir.as_path(), API), |bob| {
                (&bob.person.machine, bob.dir.as_path(), REVIEW)
            });
        let mark = solo.mark();
        let recipient = self.live.agent();
        let channel_arg = channel.to_string();
        let sent = machine
            .start_in(
                dir,
                &[
                    AGENT,
                    agent,
                    SEND,
                    TO,
                    &recipient,
                    CHANNEL,
                    &channel_arg,
                    text,
                ],
            )
            .finish();
        assert!(sent.success, "{sent:?}");
        one_message_sent(&solo.talk.front, channel, mark)
    }

    fn recent_terminal_output(&self) -> String {
        let output = self.terminal.shown();
        let tail: Vec<&str> = output.lines().rev().take(35).collect();
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    }

    pub fn sender(&self) -> &Machine {
        self.live.codex.machine()
    }
}

#[test]
fn windows_discovery_finds_native_winget_and_npm_codex() {
    let scratch = crate::machine::TestDir::new("codex-e2e-discovery");
    let winget = scratch.join("codex-x86_64-pc-windows-msvc.exe");
    fs::write(&winget, []).expect("WinGet executable");
    let dirs = || std::iter::once(scratch.to_path_buf());
    assert_eq!(windows_codex(dirs(), "x86_64"), Some(winget.clone()));
    fs::remove_file(winget).expect("remove WinGet executable");
    let npm = scratch
        .join("node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe");
    fs::create_dir_all(npm.parent().expect("npm native directory")).expect("npm directories");
    fs::write(scratch.join("codex.cmd"), []).expect("npm shim");
    assert_eq!(windows_codex(dirs(), "x86_64"), None);
    fs::write(&npm, []).expect("npm native executable");
    assert_eq!(windows_codex(dirs(), "x86_64"), Some(npm));
}

fn rollout(dir: &Path) -> String {
    let Ok(files) = fs::read_dir(dir) else {
        return String::new();
    };
    let mut all = String::new();
    for entry in files.flatten() {
        let path = entry.path();
        if path.is_dir() {
            all.push_str(&rollout(&path));
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            all.push_str(&fs::read_to_string(path).unwrap_or_default());
        }
    }
    all
}

pub fn input_items(body: &Value) -> &[Value] {
    body["input"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

pub fn user_text(body: &Value) -> String {
    input_items(body)
        .iter()
        .rev()
        .find(|item| item["type"] == "message" && item["role"] == "user")
        .map(|item| item["content"].to_string())
        .unwrap_or_default()
}

pub fn after_prompt<'a>(body: &'a Value, prompt: &str) -> &'a [Value] {
    let items = input_items(body);
    items
        .iter()
        .rposition(|item| {
            item["type"] == "message"
                && item["role"] == "user"
                && item["content"].to_string().contains(prompt)
        })
        .map(|position| &items[position + 1..])
        .unwrap_or_default()
}

pub fn called_after(body: &Value, prompt: &str) -> bool {
    after_prompt(body, prompt)
        .iter()
        .any(|item| item["type"] == "function_call_output")
}

pub fn inbox() -> ResponsesReply {
    ResponsesReply::Call(Step::Skald("inbox", json!({})))
}

pub fn command(text: &str, args: Value) -> ResponsesReply {
    let mut input = args.as_object().expect("tool arguments").clone();
    input.insert("cmd".to_owned(), json!(text));
    ResponsesReply::Call(Step::Tool("exec_command", Value::Object(input)))
}

#[cfg(unix)]
struct Daemon {
    stop: Command,
}

#[cfg(unix)]
impl Daemon {
    fn start(live: &Live, settings: &[(String, String)]) -> Self {
        let launch = |action| {
            let mut command = live.command(&live.program, settings);
            command
                .args(["app-server", "daemon", action])
                .stdin(Stdio::null());
            command
        };
        let mut stop = launch("stop");
        stop.stdout(Stdio::null()).stderr(Stdio::null());
        let daemon = Self { stop };
        let out = launch("start").output().expect("launch fixture app-server");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        daemon
    }
}

#[cfg(unix)]
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.stop.status();
    }
}
