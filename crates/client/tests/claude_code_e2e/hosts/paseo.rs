use std::fs::File;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use serde_json::{Value, json};

use super::{Hosted, PROMPT, host_on_path, output, run, until};
use crate::claude::claude_code;
use crate::plugin::put_json;

struct Daemon<'a> {
    host: &'a Path,
    session: &'a Hosted,
    child: Child,
    home: PathBuf,
    endpoint: String,
}

impl<'a> Daemon<'a> {
    fn start(host: &'a Path, session: &'a Hosted) -> Self {
        let home = session.home.dir.join("paseo");
        let free = TcpListener::bind(("127.0.0.1", 0)).expect("a local port for Paseo");
        let endpoint = free.local_addr().expect("Paseo's address").to_string();
        put_json(
            &home.join("config.json"),
            &json!({
                "version": 1,
                "daemon": { "listen": endpoint, "enableTerminalAgentHooks": true,
                            "relay": { "enabled": false } },
                "features": { "dictation": { "enabled": false }, "voiceMode": { "enabled": false } },
            }),
        );
        let log = File::create(home.join("test-daemon.log")).expect("Paseo's log file");
        drop(free);
        let child = session
            .command(host)
            .env("PASEO_HOME", &home)
            .args(["daemon", "run"])
            .stdout(log.try_clone().expect("Paseo's log file"))
            .stderr(log)
            .spawn()
            .expect("start Paseo");
        let daemon = Self {
            host,
            session,
            child,
            home,
            endpoint,
        };
        until("Paseo's daemon", || {
            output(&mut daemon.command(&["daemon", "status"])).0
        });
        daemon
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = self.session.command(self.host);
        command.env("PASEO_HOME", &self.home).args(args);
        command
    }

    fn run(&self, args: &[&str]) -> String {
        run(&mut self.command(args))
    }

    /// The CLI lists workspaces without status. Read it from Paseo's client API.
    fn workspace_status(&self, workspace: &str) -> Value {
        let bin = self.host.canonicalize().expect("Paseo's executable");
        let modules = bin.ancestors().nth(3).expect("Paseo's npm modules");
        let script = r#"
            import { createPaseoClient } from '@getpaseo/client';
            const [host, id] = process.argv.slice(1);
            const client = createPaseoClient({ url: `ws://${host}/ws`, reconnect: { enabled: false } });
            await client.connect();
            try {
                const { entries } = await client.workspaces.list({});
                console.log(JSON.stringify(entries.find(workspace => workspace.id === id)?.status ?? null));
            } finally {
                await client.close();
            }
        "#;
        let printed = run(self
            .session
            .command("node")
            .env("PASEO_HOME", &self.home)
            .current_dir(modules)
            .args([
                "--input-type=module",
                "-e",
                script,
                &self.endpoint,
                workspace,
            ]));
        serde_json::from_str(&printed).expect("Paseo's workspace status")
    }
}

impl Drop for Daemon<'_> {
    fn drop(&mut self) {
        let _ = output(&mut self.command(&["daemon", "stop", "--force"]));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "e2e: runs Paseo and the real Claude Code"]
fn paseos_sdk_session_delivers_skald_messages() {
    let (Some(host), Some(claude)) = (host_on_path("paseo"), claude_code()) else {
        return;
    };
    let session = Hosted::new(&claude, 4_919_102_001, "ada-claude-paseo");
    let daemon = Daemon::start(&host, &session);
    let settings = session.home.dir.join("settings.json");
    until("Paseo's Claude settings", || settings.is_file());
    session.install_skald();

    let printed = daemon.run(&[
        "run",
        "--provider",
        "claude",
        "--model",
        "claude-sonnet-4-6",
        "--mode",
        "default",
        "--wait-timeout",
        "45s",
        "--json",
        PROMPT,
    ]);
    let agent: Value = serde_json::from_str(&printed).expect("Paseo's agent run result");
    let id = agent["agentId"].as_str().expect("Paseo's agent ID");
    let shown = || daemon.run(&["logs", id, "--tail", "20"]);
    session.assert_tools(shown);
    session.assert_wake(shown);
}

#[test]
#[ignore = "e2e: runs Paseo and the real Claude Code"]
fn paseos_terminal_session_delivers_skald_messages_and_keeps_paseos_hook() {
    let (Some(host), Some(claude), Some(_node)) =
        (host_on_path("paseo"), claude_code(), host_on_path("node"))
    else {
        return;
    };
    let session = Hosted::new(&claude, 4_919_103_001, "ada-claude-paseo-terminal");
    let daemon = Daemon::start(&host, &session);
    let settings = session.home.dir.join("settings.json");
    until("Paseo's Claude settings", || settings.is_file());
    session.install_skald();

    let printed = run(daemon
        .command(&["terminal", "create", "--json", "--cwd"])
        .arg(&session.dir));
    let terminal: Value = serde_json::from_str(&printed).expect("Paseo's terminal");
    let id = terminal["id"].as_str().expect("Paseo's terminal ID");
    let workspace = terminal["workspaceId"]
        .as_str()
        .expect("Paseo's workspace ID");
    assert_ne!(daemon.workspace_status(workspace), "attention");
    let shown = || daemon.run(&["terminal", "capture", id]);
    let open = format!("claude '{}'", PROMPT.replace('\'', "'\\''"));
    daemon.run(&["terminal", "send-keys", id, &open, "Enter"]);
    session.assert_tools(shown);
    until("the model's reply in Paseo's terminal", || {
        shown().contains(super::DONE)
    });
    until("Paseo's hook reporting the completed turn", || {
        daemon.workspace_status(workspace) == "attention"
    });
    session.assert_wake(shown);
}
