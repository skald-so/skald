//! The live check with Herdr, a PTY host: Codex's terminal UI runs in a pane of Herdr's own
//! server, started as Herdr's Codex agent, after Herdr's Codex integration and Skald's setup wrote
//! their hooks into the session's config home.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{fs, thread};

use serde_json::Value;

use crate::claude::PROMPT;
use crate::host::{Live, WAIT, as_fixture, on_path, record, version};

/// What api says to the session's agent once it is idle; it is nowhere else.
const WAKES: &str = "codex hosts check: api asks the session in Herdr for the build log";

/// The name of the Codex agent the check starts in Herdr.
const AGENT_NAME: &str = "skald-check";

/// The source of the session Herdr's own hook reports.
const HERDR_CODEX: &str = "herdr:codex";

/// A directory with a short path, as Herdr's sockets need, removed on drop.
struct Short(PathBuf);

impl Short {
    fn new() -> Self {
        let nanos = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH))
            .map(|since| since.subsec_nanos())
            .unwrap_or_default();
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let dir = base.join(format!("skh-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("make Herdr's directory");
        Short(dir)
    }
}

impl Drop for Short {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Herdr's server for the check; stopped and killed on drop, which ends its panes.
struct Server {
    herdr: PathBuf,
    settings: Vec<(String, String)>,
    child: Child,
    _dir: Short,
}

impl Server {
    fn start(herdr: &Path, live: &Live, settings: &[(String, String)]) -> Self {
        let dir = Short::new();
        let mut settings = settings.to_vec();
        settings.extend([
            ("XDG_CONFIG_HOME".to_owned(), path_text(&dir.0.join("c"))),
            ("XDG_STATE_HOME".to_owned(), path_text(&dir.0.join("s"))),
            ("SHELL".to_owned(), "/bin/sh".to_owned()),
        ]);
        let child = (live.command(herdr, &settings).arg("server"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start Herdr's server");
        let server = Server {
            herdr: herdr.to_owned(),
            settings,
            child,
            _dir: dir,
        };
        let deadline = Instant::now() + WAIT;
        while !server.run(&["status", "server"]).contains("running") {
            assert!(Instant::now() < deadline, "Herdr's server did not start");
            thread::sleep(Duration::from_millis(200));
        }
        server
    }

    /// `herdr <args>` against this server; gives its standard output.
    fn run(&self, args: &[&str]) -> String {
        let mut command = Command::new(&self.herdr);
        command
            .env_clear()
            .envs(self.settings.iter().map(|(name, value)| (name, value)))
            .args(args)
            .stdin(Stdio::null());
        let output = command.output().expect("run herdr");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.run(&["server", "stop"]);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The pane of the workspace `herdr workspace create` made, from its output.
pub fn root_pane(created: &str) -> Option<String> {
    let created: Value = serde_json::from_str(created.trim()).ok()?;
    let pane = &created["result"]["root_pane"]["pane_id"];
    pane.as_str().map(str::to_owned)
}

/// The session Herdr's own Codex hook reported for the agent, from `herdr agent get`.
pub fn reported_session(agent: &str) -> Option<String> {
    let agent: Value = serde_json::from_str(agent.trim()).ok()?;
    let session = &agent["result"]["agent"]["agent_session"];
    (session["source"] == HERDR_CODEX)
        .then(|| session["value"].as_str())
        .flatten()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Herdr's Codex integration and Skald's setup write their hooks into the session's config home:
/// Herdr's `hooks.json` and `config.toml` are those of the fixture `homes/herdr`. Codex trusts the
/// hooks, as Review hooks does. Herdr's server runs Codex's terminal UI as its Codex agent in a
/// pane, in the session's folder, on the stub: the stub calls `inbox` and runs `skald devices`, and
/// both succeed. Once the turn ends, api sends the session's agent a message: a turn starts with
/// the signal, and no request holds the text. Herdr's own hook reported the session to Herdr.
#[test]
#[ignore = "a live check with Herdr and Codex; run with --run-ignored only"]
fn a_codex_session_in_herdr_gets_skald_and_keeps_herdrs_hooks() {
    let (Some(herdr), Some(codex)) = (on_path("herdr"), on_path("codex")) else {
        return;
    };
    let host = format!("Herdr {}, Codex {}", version(&herdr), version(&codex));
    let live = Live::new(5_209_101_001, "ada-codex-herdr", codex);
    let settings = live.settings(&[&herdr]);

    let installed = (live.command(&herdr, &settings))
        .args(["integration", "install", "codex"])
        .stdout(Stdio::null())
        .status()
        .expect("run herdr integration install");
    assert!(installed.success(), "Herdr's integration failed");
    let read = |name: &str| fs::read_to_string(live.home().join(name)).expect("a written file");
    let fixture = |name: &str| {
        let homes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/codex_setup/homes/herdr");
        fs::read_to_string(homes.join(name)).expect("the fixture")
    };
    for name in ["hooks.json", "config.toml"] {
        let written = as_fixture(&read(name), live.home());
        record(&format!("herdr-{name}"), &written);
        assert_eq!(
            written,
            fixture(name),
            "Herdr's {name} is not the fixture's"
        );
    }
    live.set_up();
    live.configure(&settings);

    let server = Server::start(&herdr, &live, &settings);
    let dir = path_text(live.dir());
    let mut create = vec!["workspace", "create", "--cwd", &dir, "--no-focus"];
    let given: Vec<String> = (settings.iter())
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    for setting in &given {
        create.extend(["--env", setting]);
    }
    let created = server.run(&create);
    record("herdr-workspace-create.json", &created);
    let pane = root_pane(&created).expect("a pane");
    let timeout = WAIT.as_millis().to_string();
    let start = [
        "agent",
        "start",
        AGENT_NAME,
        "--kind",
        "codex",
        "--pane",
        &pane,
        "--timeout",
        &timeout,
        "--",
        "--no-daemon",
        "-C",
        &dir,
        PROMPT,
    ];
    let started = server.run(&start);
    assert!(started.contains("agent_started"), "{started}");

    let deadline = Instant::now() + WAIT;
    while live.stub.turns().is_empty() {
        let shown = || server.run(&["pane", "read", &pane]);
        assert!(
            Instant::now() < deadline,
            "no turn reached the stub; the pane shows:\n{}",
            shown()
        );
        thread::sleep(Duration::from_millis(500));
    }
    let results = live.results(0, WAKES);
    let agent = server.run(&["agent", "get", AGENT_NAME]);
    record("herdr-agent-get.json", &agent);
    let reported = reported_session(&agent);
    println!(
        "{}; Herdr's hook reported the session {}",
        results.line(&host),
        if reported.is_some() { "yes" } else { "no" }
    );
    drop(server);
    assert!(results.mcp && results.cli, "{results:?}");
    assert!(
        results.woke.is_some() && results.text_stayed_out,
        "{results:?}"
    );
    assert!(
        reported.is_some(),
        "Herdr's hook reported no session: {agent}"
    );
}

/// Herdr 0.9.5's outputs, recorded by the check: the pane of the workspace it made, and the session
/// its Codex hook reported.
#[test]
fn herdrs_recorded_outputs_give_the_pane_and_the_reported_session() {
    let created = include_str!("recorded/herdr-workspace-create.json");
    assert_eq!(root_pane(created).as_deref(), Some("w1:p1"));
    let agent = include_str!("recorded/herdr-agent-get.json");
    let session = reported_session(agent).expect("a reported session");
    assert_eq!(session.len(), 36, "{session}");
    let unreported = agent.replace(HERDR_CODEX, "herdr:other");
    assert_eq!(reported_session(&unreported), None);
    assert_eq!(reported_session("not JSON"), None);
}
