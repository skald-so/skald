//! The live check with Paseo, an SDK host: Paseo's daemon runs Codex through Codex's app-server, as
//! `paseo run --provider codex` starts it, after the daemon, with its terminal agent hooks on, and
//! Skald's setup wrote their hooks into the session's config home. Paseo's hooks act only in
//! Paseo's terminals, so a second session runs Codex's terminal UI in one.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::claude::PROMPT;
use crate::host::{Live, STUB_MODEL, WAIT, as_fixture, on_path, record, version};
use crate::stub::played;

/// What api says to the session's agent once it is idle; it is nowhere else.
const WAKES: &str = "codex hosts check: api asks the session in Paseo for the test report";

/// The workspace states Paseo derives from its hooks' reports of a terminal: a turn running, or
/// finished.
const REPORTED: [&str; 2] = ["running", "attention"];

/// Paseo's daemon for the check; stopped and killed on drop, which ends its agents and terminals.
struct Daemon {
    paseo: PathBuf,
    settings: Vec<(String, String)>,
    child: Child,
    /// Its local endpoint.
    host: String,
}

impl Daemon {
    /// Starts the daemon in a home of its own on the machine, with the relay, speech and voice off
    /// and its terminal agent hooks on.
    fn start(paseo: &Path, live: &Live, settings: &[(String, String)]) -> Self {
        let home = live.codex.machine().root().join("paseo");
        fs::create_dir_all(&home).expect("make Paseo's home");
        let port = TcpListener::bind(("127.0.0.1", 0))
            .and_then(|listener| listener.local_addr())
            .expect("a free port")
            .port();
        let host = format!("127.0.0.1:{port}");
        let config = json!({
            "version": 1,
            "daemon": {
                "listen": host,
                "enableTerminalAgentHooks": true,
                "relay": { "enabled": false },
            },
            "features": {
                "dictation": { "enabled": false },
                "voiceMode": { "enabled": false },
            },
        });
        fs::write(home.join("config.json"), config.to_string()).expect("write Paseo's config");
        let mut settings = settings.to_vec();
        settings.push(("PASEO_HOME".to_owned(), home.to_string_lossy().into_owned()));
        let child = (live.command(paseo, &settings).args(["daemon", "run"]))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start Paseo's daemon");
        let daemon = Daemon {
            paseo: paseo.to_owned(),
            settings,
            child,
            host,
        };
        let deadline = Instant::now() + WAIT;
        while !daemon.run(&["daemon", "status"]).0 {
            assert!(Instant::now() < deadline, "Paseo's daemon did not start");
            thread::sleep(Duration::from_millis(500));
        }
        daemon
    }

    /// A program with the daemon's settings, in `dir`.
    fn command(&self, program: &Path, dir: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .envs(self.settings.iter().map(|(name, value)| (name, value)))
            .current_dir(dir)
            .stdin(Stdio::null());
        command
    }

    /// `paseo <args>`; gives whether it succeeded and its standard output.
    fn run(&self, args: &[&str]) -> (bool, String) {
        let home = self.settings.iter().find(|(name, _)| name == "HOME");
        let dir = home.map_or_else(|| PathBuf::from("/"), |(_, home)| PathBuf::from(home));
        let output = (self.command(&self.paseo, &dir).args(args))
            .output()
            .expect("run paseo");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        (output.status.success(), stdout)
    }

    /// The workspaces Paseo's client lists, as its daemon gives them to its apps: the CLI shows no
    /// status. Runs Node with Paseo's own client modules.
    fn workspaces(&self, node: &Path) -> String {
        let modules = (fs::canonicalize(&self.paseo).ok())
            .and_then(|bin| Some(bin.parent()?.parent()?.parent()?.to_owned()))
            .expect("Paseo's modules");
        let url = |module: &str| format!("file://{}", modules.join(module).to_string_lossy());
        let script = format!(
            "const c = await import({cli:?});\n\
             const {{ createPaseoApi }} = await import({client:?});\n\
             const client = await c.connectToDaemon({{ target: {{ kind: 'endpoint', host: {host:?} }} }});\n\
             const listed = await createPaseoApi(client).workspaces.list({{}});\n\
             console.log(JSON.stringify(listed));\n\
             await client.close();\n\
             process.exit(0);",
            cli = url("cli/dist/utils/client.js"),
            client = url("client/dist/index.js"),
            host = self.host,
        );
        let dir = modules.clone();
        let output = (self.command(node, &dir))
            .args(["--input-type=module", "-e", &script])
            .output()
            .expect("run node");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.run(&["daemon", "stop"]);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The status of the workspace `id` in the workspaces Paseo's client lists.
pub fn status_of(listed: &str, id: &str) -> Option<String> {
    let listed: Value = serde_json::from_str(listed.trim()).ok()?;
    let entries = listed["entries"].as_array()?;
    let workspace = entries.iter().find(|entry| entry["id"] == id)?;
    workspace["status"].as_str().map(str::to_owned)
}

/// The terminal and its workspace, from `paseo terminal create --json`.
pub fn terminal_of(created: &str) -> Option<(String, String)> {
    let created: Value = serde_json::from_str(created.trim()).ok()?;
    let field = |name: &str| created[name].as_str().map(str::to_owned);
    Some((field("id")?, field("workspaceId")?))
}

/// A shell's quoting of `text`, for the line the check types into Paseo's terminal.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Paseo's daemon writes its terminal agent hooks into the session's config home: its `hooks.json`
/// is that of the fixture `homes/paseo`. Skald's setup adds its own; Codex trusts the hooks, as
/// Review hooks does. `paseo run --provider codex` runs a session in its folder on the stub: the
/// stub calls `inbox` and runs `skald devices`, and both succeed. Once the turn ends, api sends the
/// session's agent a message: a turn starts with the signal, and no request holds the text. A
/// second session, Codex's terminal UI in a Paseo terminal, ends its turn, and Paseo's hooks
/// report it: the terminal's workspace shows it running or finished.
#[test]
#[ignore = "a live check with Paseo and Codex; run with --run-ignored only"]
fn a_codex_session_in_paseo_gets_skald_and_keeps_paseos_hooks() {
    let (Some(paseo), Some(node), Some(codex)) =
        (on_path("paseo"), on_path("node"), on_path("codex"))
    else {
        return;
    };
    let host = format!("Paseo {}, Codex {}", version(&paseo), version(&codex));
    let live = Live::new(5_209_102_001, "ada-codex-paseo", codex.clone());
    let settings = live.settings(&[&paseo, &node]);

    let daemon = Daemon::start(&paseo, &live, &settings);
    let hooks = live.home().join("hooks.json");
    let deadline = Instant::now() + WAIT;
    while !hooks.is_file() {
        assert!(Instant::now() < deadline, "Paseo wrote no hooks.json");
        thread::sleep(Duration::from_millis(200));
    }
    let written = as_fixture(
        &fs::read_to_string(&hooks).expect("hooks.json"),
        live.home(),
    );
    record("paseo-hooks.json", &written);
    let homes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/codex_setup/homes/paseo");
    let fixture = fs::read_to_string(homes.join("hooks.json")).expect("the fixture");
    assert_eq!(written, fixture, "Paseo's hooks.json is not the fixture's");
    live.set_up();
    live.configure(&settings);

    let dir = live.dir().to_string_lossy().into_owned();
    let run = [
        "run",
        "--provider",
        "codex",
        "--model",
        STUB_MODEL,
        "--mode",
        "full-access",
        "--cwd",
        &dir,
        "--json",
        PROMPT,
    ];
    let (ran, printed) = daemon.run(&run);
    assert!(ran, "paseo run failed: {printed}");
    let results = live.results(0, WAKES);

    let from = live.stub.turns().len();
    let (created, printed) = daemon.run(&["terminal", "create", "--cwd", &dir, "--json"]);
    assert!(created, "paseo terminal create failed: {printed}");
    record("paseo-terminal-create.json", &printed);
    let (terminal, workspace) = terminal_of(&printed).expect("a terminal");
    let line = format!("codex --no-daemon -C {} {}", quoted(&dir), quoted(PROMPT));
    let (sent, printed) = daemon.run(&["terminal", "send-keys", &terminal, &line, "Enter"]);
    assert!(sent, "paseo terminal send-keys failed: {printed}");
    live.await_turn(from, |body| played(body) >= 2);
    let deadline = Instant::now() + WAIT;
    let mut status = None;
    while Instant::now() < deadline {
        let listed = daemon.workspaces(&node);
        status = status_of(&listed, &workspace);
        if status
            .as_deref()
            .is_some_and(|status| REPORTED.contains(&status))
        {
            record("paseo-workspaces.json", &listed);
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    let reported = status
        .as_deref()
        .is_some_and(|status| REPORTED.contains(&status));
    println!(
        "{}; Paseo's hooks reported its terminal's session {}",
        results.line(&host),
        if reported { "yes" } else { "no" }
    );
    drop(daemon);
    assert!(results.mcp && results.cli, "{results:?}");
    assert!(
        results.woke.is_some() && results.text_stayed_out,
        "{results:?}"
    );
    assert!(reported, "Paseo's hooks reported nothing: {status:?}");
}

/// Paseo 0.17.1's outputs, recorded by the check: the terminal it made, and its workspace's status
/// once its hooks reported the session.
#[test]
fn paseos_recorded_outputs_give_the_terminal_and_the_reported_status() {
    let created = include_str!("recorded/paseo-terminal-create.json");
    let (terminal, workspace) = terminal_of(created).expect("a terminal");
    assert_eq!(terminal.len(), 36, "{terminal}");
    let listed = include_str!("recorded/paseo-workspaces.json");
    let status = status_of(listed, &workspace).expect("a status");
    assert!(REPORTED.contains(&status.as_str()), "{status}");
    assert_eq!(status_of(listed, "wks_none"), None);
    assert_eq!(terminal_of("{}"), None);
}

/// The line typed into the terminal quotes the folder and the prompt for the shell.
#[test]
fn the_typed_line_quotes_for_the_shell() {
    assert_eq!(quoted("Ada's work"), r"'Ada'\''s work'");
}
