use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::Value;

use super::{DONE, Hosted, PROMPT, WAIT, WakePath, host_on_path, output, run, until};
use crate::claude::claude_code;
use crate::machine::TestDir;

const AGENT: &str = "skald-check";

struct Server<'a> {
    host: &'a Path,
    session: &'a Hosted,
    child: Child,
    config: TestDir,
}

impl<'a> Server<'a> {
    fn start(host: &'a Path, session: &'a Hosted, config: TestDir) -> Self {
        let child = Self::command_for(host, session, &config)
            .arg("server")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start Herdr");
        let server = Self {
            host,
            session,
            child,
            config,
        };
        until("Herdr's server", || {
            let (ok, status) = output(&mut server.command(&["status", "server", "--json"]));
            ok && serde_json::from_str::<Value>(&status)
                .is_ok_and(|status| status["running"].as_bool() == Some(true))
        });
        server
    }

    fn command_for(host: &Path, session: &Hosted, config: &Path) -> Command {
        let mut command = session.command(host);
        command
            .env("XDG_CONFIG_HOME", config.join("config"))
            .env("XDG_STATE_HOME", config.join("state"));
        command
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Self::command_for(self.host, self.session, &self.config);
        command.args(args);
        command
    }

    fn run(&self, args: &[&str]) -> String {
        run(&mut self.command(args))
    }
}

impl Drop for Server<'_> {
    fn drop(&mut self) {
        let _ = output(&mut self.command(&["server", "stop"]));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "e2e: runs Herdr and the real Claude Code"]
fn herdrs_claude_session_delivers_skald_messages_and_keeps_herdrs_hook() {
    herdr_session(WakePath::Socket);
}

#[test]
#[ignore = "e2e: runs Herdr and the real Claude Code"]
fn herdrs_claude_session_wakes_through_fallbacks() {
    for wake in WakePath::FALLBACKS {
        herdr_session(wake);
    }
}

fn herdr_session(wake: WakePath) {
    let (Some(host), Some(claude)) = (host_on_path("herdr"), claude_code()) else {
        return;
    };
    let session = Hosted::new(&claude, 4_919_101_001, "ada-claude-herdr", wake);
    // Herdr's socket must fit in a Unix-domain sockaddr, including on macOS.
    let short = PathBuf::from("/tmp").join(format!("skald-herdr-{}-{wake:?}", std::process::id()));
    let config = TestDir::named(short.to_str().expect("UTF-8 test path"));
    run(Server::command_for(&host, &session, &config).args(["integration", "install", "claude"]));
    session.install_skald();

    let server = Server::start(&host, &session, config);
    let mut create = server.command(&["workspace", "create", "--no-focus", "--cwd"]);
    create.arg(&session.dir);
    for (key, value) in &session.home.env {
        create.args(["--env", &format!("{key}={value}")]);
    }
    let created = run(&mut create);
    let work: Value = serde_json::from_str(&created).expect("Herdr's workspace");
    let pane = work["result"]["root_pane"]["pane_id"]
        .as_str()
        .expect("Herdr's new pane");
    let shown = || server.run(&["pane", "read", pane]);
    let timeout = WAIT.as_millis().to_string();
    let mut start = server.command(&[
        "agent",
        "start",
        AGENT,
        "--kind",
        "claude",
        "--pane",
        pane,
        "--timeout",
        &timeout,
    ]);
    if !wake.args().is_empty() {
        start.arg("--").args(wake.args());
    }
    let started = run(&mut start);
    assert!(started.contains("agent_started"), "{started}");
    server.run(&["agent", "prompt", AGENT, PROMPT]);
    session.assert_tools(shown);
    until("the model's reply in Herdr's pane", || {
        shown().contains(DONE)
    });
    server.run(&[
        "agent",
        "wait",
        AGENT,
        "--until",
        "idle",
        "--until",
        "done",
        "--timeout",
        &WAIT.as_millis().to_string(),
    ]);

    let agent = server.run(&["agent", "get", AGENT]);
    let status: Value = serde_json::from_str(&agent).expect("Herdr's agent status");
    let reported = &status["result"]["agent"]["agent_session"];
    assert_eq!(reported["source"], "herdr:claude", "{agent}");
    assert!(
        reported["value"].as_str().is_some_and(|id| !id.is_empty()),
        "{agent}"
    );
    session.assert_wake(shown);
}
