//! Claude Code sessions started by real hosts in temporary config homes on the model stub.

mod herdr;
mod paseo;

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::hooks::{INBOX_WORD, assert_own_line};
use crate::lines::{agent_lines, json_objects};
use crate::plugin::{Home, Market, put_json, read_json};
use crate::receipts::shows_read_once;
use crate::socket_claude::REWAKE_WAIT;
use crate::socket_model::{Model, Reply, conversation, last_user_text};
use crate::socket_sessions::{after_a_tool, assert_own_by_hook};
use crate::solo::{API, Solo};
use crate::stub::{DONE, PLACEHOLDER};
use crate::talk::ADA_HOST;
use crate::wake_fallbacks::{MOD_DIR, default_agent};

const WAIT: Duration = Duration::from_secs(45);
const PROMPT: &str = "Run the host integration test.";
const CLI: &str = "skald devices";
const INBOX: &str = "mcp__plugin_skald_skald__inbox";
const EMPTY_INBOX: &str = "No new messages.";
const OWN: &str = "host e2e: retrieve the migration report";
const MOD_PROMPT: &str = "skald: messages wait for you";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WakePath {
    Socket,
    Rewake,
    Mod,
}

impl WakePath {
    const FALLBACKS: [Self; 2] = [Self::Rewake, Self::Mod];

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Mod => &["--plugin-dir", MOD_DIR],
            Self::Socket | Self::Rewake => &[],
        }
    }

    fn project_settings(self) -> Value {
        let mut settings = json!({"permissions": {"allow": [format!("Bash({CLI})"), INBOX]}});
        if self != Self::Socket {
            settings["crossSessionInbound"] = json!("refuse");
        }
        if self == Self::Rewake {
            let hook = json!([{ "hooks": [{
                "type": "command",
                "command": format!("skald hook claude --wait {REWAKE_WAIT}"),
                "asyncRewake": true,
                "timeout": REWAKE_WAIT + 60,
            }] }]);
            settings["hooks"] = json!({ "SessionStart": hook, "Stop": hook });
        }
        settings
    }
}

fn host_on_path(name: &str) -> Option<PathBuf> {
    let found = env::var_os("PATH")
        .iter()
        .flat_map(env::split_paths)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file());
    if found.is_none() {
        let required = env::var_os("SKALD_E2E_REQUIRED_HOSTS")
            .is_some_and(|value| !value.is_empty() && value != "0");
        assert!(!required, "{name} is required but is not on PATH");
        eprintln!("skipped: {name} is not on PATH");
    }
    found
}

struct Hosted {
    solo: Solo,
    dir: PathBuf,
    home: Home,
    model: Model,
    wake: WakePath,
}

impl Hosted {
    fn new(claude: &Path, id: u64, login: &str, wake: WakePath) -> Self {
        let solo = Solo::new(id, login);
        let dir = solo.dir("host");
        let model = Model::start(|body| {
            let last = last_user_text(body);
            let tool_result = after_a_tool(body);
            if !tool_result && last.lines().any(|line| line == PROMPT) {
                Reply::Bash(CLI)
            } else if (tool_result && last.contains(ADA_HOST) && !last.contains(OWN))
                || (!tool_result && last.contains(MOD_PROMPT))
            {
                Reply::Mcp(INBOX)
            } else {
                Reply::Done
            }
        });
        let mut home = Home::new(claude, solo.machine(), true);
        home.env.extend([
            ("ANTHROPIC_BASE_URL".to_owned(), model.url().to_owned()),
            ("ANTHROPIC_API_KEY".to_owned(), PLACEHOLDER.to_owned()),
            (
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(),
                "1".to_owned(),
            ),
            ("ENABLE_TOOL_SEARCH".to_owned(), "false".to_owned()),
            ("SHELL".to_owned(), "/bin/sh".to_owned()),
            ("TERM".to_owned(), "xterm-256color".to_owned()),
        ]);
        if wake != WakePath::Socket {
            home.env
                .push(("SKALD_RESEND_AFTER".to_owned(), "5".to_owned()));
        }
        let profile = home.dir.join(".claude.json");
        let mut trusted = read_json(&profile);
        let canonical = dir.canonicalize().expect("the project's path");
        for path in [&canonical, &dir] {
            trusted["projects"][path.to_string_lossy().as_ref()] =
                json!({ "hasTrustDialogAccepted": true });
        }
        let tail = &PLACEHOLDER[PLACEHOLDER.len() - 20..];
        trusted["customApiKeyResponses"] = json!({ "approved": [tail], "rejected": [] });
        trusted["theme"] = json!("dark");
        put_json(&profile, &trusted);
        put_json(
            &dir.join(".claude/settings.local.json"),
            &wake.project_settings(),
        );
        Self {
            solo,
            dir,
            home,
            model,
            wake,
        }
    }

    fn command(&self, bin: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(bin);
        command
            .env_clear()
            .envs(self.home.env.iter().map(|(key, value)| (key, value)))
            .current_dir(&self.dir)
            .stdin(Stdio::null());
        command
    }

    fn install_skald(&self) {
        let market = Market::new(&self.solo.machine().root().join("tmp/host-market"));
        self.home.install(&market);
    }

    /// The model ran the CLI in the real session and called the plugin's inbox successfully.
    #[track_caller]
    fn assert_tools(&self, shown: impl Fn() -> String) {
        self.model
            .wait_turn_showing(ADA_HOST, "Skald CLI output", &shown);
        let turn = self
            .model
            .wait_turn_showing(EMPTY_INBOX, "Skald MCP inbox output", &shown);
        assert!(
            after_a_tool(&turn),
            "expected an inbox tool result: {}",
            conversation(&turn)
        );
    }

    /// After the host reported that the turn ended, another agent sends Skald a message. No
    /// input is sent to Claude Code by the test. Its Skald hook must have registered the session.
    #[track_caller]
    fn assert_wake(&self, shown: impl Fn() -> String) {
        let agent = if self.wake == WakePath::Socket {
            let listed = self.solo.person_runs(&["agents", "--json"]);
            assert!(listed.success, "{listed:?}");
            json_objects(&listed.stdout)
                .into_iter()
                .find(|agent| agent["runtime"] == "claude-code")
                .and_then(|agent| agent["agent"].as_str().map(str::to_owned))
                .unwrap_or_else(|| panic!("the host's Claude Code agent is missing: {listed:?}"))
        } else {
            self.solo.short(&default_agent(&self.solo))
        };
        let msg = self.solo.sends(&mut self.solo.session(API), &agent, OWN);
        let turn = self
            .model
            .wait_turn_showing(OWN, "a message woke the idle session", &shown);
        let from = self.solo.short(API);
        match self.wake {
            WakePath::Socket => drop(assert_own_line(&last_user_text(&turn), OWN, msg, &from)),
            WakePath::Rewake => {
                assert!(
                    conversation(&turn).contains("hook blocking error from command"),
                    "{turn}"
                );
                assert_own_by_hook(&turn, OWN, msg, &from);
            }
            WakePath::Mod => {
                let prompt = self
                    .model
                    .turns()
                    .iter()
                    .map(last_user_text)
                    .find(|text| text.contains(MOD_PROMPT) && text.contains(INBOX_WORD))
                    .expect("the mod's prompt");
                assert!(
                    !prompt.contains(OWN) && agent_lines(&prompt).is_empty(),
                    "{prompt}"
                );
                assert!(after_a_tool(&turn), "{turn}");
                assert_own_by_hook(&turn, OWN, msg, &from);
            }
        }
        shows_read_once(self.solo.machine(), msg);
        self.model.assert_no_foreign_credential();
    }
}

fn output(command: &mut Command) -> (bool, String) {
    let printed = command.output().expect("run the host CLI");
    let out = if printed.status.success() {
        printed.stdout
    } else {
        [printed.stdout, printed.stderr].concat()
    };
    (
        printed.status.success(),
        String::from_utf8_lossy(&out).into_owned(),
    )
}

#[track_caller]
fn run(command: &mut Command) -> String {
    let (ok, printed) = output(command);
    assert!(
        ok,
        "host command failed: {:?} {:?}\n{printed}",
        command.get_program(),
        command.get_args()
    );
    printed
}

#[track_caller]
fn until(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(250));
    }
}
