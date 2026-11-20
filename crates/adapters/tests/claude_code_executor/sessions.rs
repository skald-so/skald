//! E2E: executors of the real Claude Code on the model stub. In `read-only` the stub's write,
//! command, fetch from a local server and reads outside the folder, also through a symlink in it,
//! all fail, and the answer comes back; in `full` the session writes in its folder and runs a
//! command there; a variable of the device's environment and the user's config planted in the
//! config home reach none of the stub's requests or the session's commands, while a plain session
//! on the same config gets them all.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fs, process, thread};

use serde_json::{Value, json};
use skald_adapters::claude_code::Executor;
use skald_adapters::executor::{Mode, Outcome};

use crate::fixtures::{
    MARKERS, Scratch, Witness, claude_code, device_environment, run, symlink_file, token,
};
use crate::planted::{HOOK_PATH, MCP_PATH, PLUGIN_HOOK_PATH, Planted, plant};
use crate::stub::{DONE, Step, Stub, played};

/// The prompt of every session: the stub, not the prompt, decides what the session does.
const PROMPT: &str = "Run the steps of the test.";

/// The command that shows the [`MARKERS`] the session's commands get, also in `seen.txt`.
const SEEN: &str = r#"echo "marker=${SKALD_E2E_MARKER-} token=${GITHUB_TOKEN-}" | tee seen.txt"#;

/// Everything the session sent the stub, each request a line apart.
fn sent(stub: &Stub) -> String {
    let bodies = stub.turns();
    let texts = bodies.iter().map(Value::to_string);
    texts.collect::<Vec<_>>().join("\n")
}

/// The names of the tools the session offered the model.
fn offered(stub: &Stub) -> Vec<String> {
    let bodies = stub.turns();
    let tools = bodies.iter().filter_map(|body| body["tools"].as_array());
    let names = tools.flatten().filter_map(|tool| tool["name"].as_str());
    names.map(str::to_owned).collect()
}

/// Fails unless the session played all `steps` of the stub's script and asked for its answer.
#[track_caller]
fn assert_played(stub: &Stub, steps: usize) {
    let most = stub.turns().iter().map(played).max().unwrap_or(0);
    assert_eq!(most, steps, "the session did not play the stub's script");
}

/// Each file under `dir` with its contents, links not followed.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    for entry in fs::read_dir(dir).expect("list a directory").flatten() {
        let path = entry.path();
        let kind = entry.file_type().expect("a file type");
        if kind.is_dir() {
            found.extend(files(&path));
        } else if kind.is_file() {
            found.insert(path.clone(), fs::read(&path).expect("read a file"));
        } else {
            found.insert(path, Vec::new());
        }
    }
    found
}

/// A session's world in a scratch directory of its own: the session's folder, a home and a config
/// home, and the values of the device's [`MARKERS`].
struct World {
    scratch: Scratch,
    folder: PathBuf,
    markers: [String; 2],
}

impl World {
    fn new() -> Self {
        let scratch = Scratch::new();
        let folder = scratch.dir("folder");
        World {
            scratch,
            folder,
            markers: [token("marker"), token("github")],
        }
    }

    /// The device's environment, signed in on `stub`.
    fn environment(&self, stub: &Stub) -> Vec<(OsString, OsString)> {
        let marker = |name: &str| {
            let at = MARKERS.iter().position(|marker| *marker == name);
            self.markers[at.expect("one of the markers")].clone()
        };
        let (home, config) = (self.scratch.dir("home"), self.scratch.dir("config"));
        device_environment(&home, &config, stub.url(), marker)
    }

    /// Runs Claude Code's executor at `mode` in the folder on a stub that plays `script`; the stub
    /// and the outcome. Fails unless the executor runs, plays the script and ends well with the
    /// stub's answer, having sent it no credential but its placeholder.
    #[track_caller]
    fn execute(&self, claude: &Path, mode: Mode, script: Vec<Step>) -> (Stub, Outcome) {
        let steps = script.len();
        let stub = Stub::start(script, None);
        let executor = Executor::new(claude).environment(self.environment(&stub));

        let outcome = run(executor, &self.folder, mode, PROMPT)
            .unwrap_or_else(|refused| panic!("the executor refused: {refused}"));

        assert_played(&stub, steps);
        stub.assert_no_foreign_credential();
        assert!(
            outcome.finished,
            "the run did not end well: {:?}",
            outcome.answer
        );
        assert!(
            outcome.answer.contains(DONE),
            "the stub's answer did not come back: {:?}",
            outcome.answer
        );
        (stub, outcome)
    }

    /// Plants the user's config in the config home, its MCP server and hooks posting to the
    /// witness it gives.
    fn plant(&self) -> (Witness, Planted) {
        let witness = Witness::start();
        let config = self.scratch.dir("config");
        let planted = plant(&config, &self.folder, &witness.url(""));
        (witness, planted)
    }
}

#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn read_only_writes_runs_fetches_and_reads_outside_its_folder_nothing() {
    let Some(claude) = claude_code() else { return };
    let world = World::new();
    let (folder, outside) = (&world.folder, world.scratch.dir("outside"));
    let (inside, secret, linked) = (token("inside"), token("secret"), token("linked"));
    fs::write(folder.join("notes.txt"), &inside).expect("write the folder's file");
    fs::write(outside.join("secret.txt"), &secret).expect("write a file outside");
    fs::write(outside.join("linked.txt"), &linked).expect("write a file outside");
    symlink_file(&outside.join("linked.txt"), &folder.join("link.txt"));
    let (before, outside_before) = (files(folder), files(&outside));
    let server = Witness::start();
    let script = vec![
        Step::Tool(
            "Write",
            json!({ "file_path": folder.join("written.txt"), "content": "written" }),
        ),
        Step::Tool(
            "Edit",
            json!({ "file_path": folder.join("notes.txt"), "old_string": inside, "new_string": "edited" }),
        ),
        Step::Bash("touch ran.txt"),
        Step::Tool(
            "WebFetch",
            json!({ "url": server.url("/fetched"), "prompt": "Return the page." }),
        ),
        Step::Tool("Read", json!({ "file_path": outside.join("secret.txt") })),
        Step::Tool("Read", json!({ "file_path": folder.join("link.txt") })),
        Step::Tool(
            "Grep",
            json!({ "pattern": ".", "path": outside, "output_mode": "content" }),
        ),
        Step::Tool("Read", json!({ "file_path": folder.join("notes.txt") })),
    ];

    let (stub, outcome) = world.execute(&claude, Mode::ReadOnly, script);

    assert_eq!(files(folder), before, "the folder changed");
    assert_eq!(files(&outside), outside_before, "the files outside changed");
    let fetched = server.paths();
    assert!(fetched.is_empty(), "the server got {fetched:?}");
    let sent = sent(&stub);
    assert!(sent.contains(&inside), "the folder's own file was not read");
    for (what, token) in [("outside", &secret), ("through the symlink", &linked)] {
        let token = token.as_str();
        assert!(!sent.contains(token), "a file {what} reached the model");
        assert!(
            !outcome.answer.contains(token),
            "a file {what} is in the answer"
        );
    }
}

#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn full_writes_and_runs_a_command_in_its_folder() {
    let Some(claude) = claude_code() else { return };
    let world = World::new();
    let written = world.folder.join("written.txt");
    let script = vec![
        Step::Tool(
            "Write",
            json!({ "file_path": written, "content": "written" }),
        ),
        Step::Bash("touch ran.txt"),
    ];

    world.execute(&claude, Mode::Full, script);

    let text = fs::read_to_string(&written).unwrap_or_default();
    assert_eq!(text.trim(), "written", "the session wrote no file");
    let ran = world.folder.join("ran.txt");
    assert!(ran.exists(), "the command did not run in the folder");
}

/// What one session let through of the planted config and the [`MARKERS`].
struct Reached {
    /// The planted MCP server, hooks and plugin hook that sent the witness a request.
    requested: Vec<&'static str>,
    /// The planted tokens that reached the stub.
    tokens: Vec<&'static str>,
    /// The MCP tools the session offered the model.
    mcp_tools: Vec<String>,
    /// The markers that reached the stub or the session's command.
    markers: Vec<&'static str>,
}

/// What reached the session of `world` that `stub`, `witness` and its folder tell, for `planted`.
fn reached(stub: &Stub, witness: &Witness, world: &World, planted: &Planted) -> Reached {
    let paths = witness.paths();
    let requested = [
        ("the MCP server", MCP_PATH),
        ("the hooks", HOOK_PATH),
        ("the plugin's hook", PLUGIN_HOOK_PATH),
    ];
    let requested = (requested.into_iter())
        .filter(|(_, path)| paths.iter().any(|got| got.starts_with(path)))
        .map(|(what, _)| what)
        .collect();
    let sent = sent(stub);
    let seen =
        fs::read_to_string(world.folder.join("seen.txt")).expect("the session's command ran");
    Reached {
        requested,
        tokens: (planted.tokens().into_iter())
            .filter(|(_, token)| sent.contains(token))
            .map(|(what, _)| what)
            .collect(),
        mcp_tools: (offered(stub).into_iter())
            .filter(|name| name.starts_with("mcp__"))
            .collect(),
        markers: (MARKERS.into_iter().zip(&world.markers))
            .filter(|(_, marker)| sent.contains(marker.as_str()) || seen.contains(marker.as_str()))
            .map(|(name, _)| name)
            .collect(),
    }
}

/// Runs a plain `claude -p` in `folder` with `environment`, as a person would, with no flag that
/// leaves out the user's config; fails unless it ends within the executor's limit.
fn plain_session(claude: &Path, folder: &Path, environment: Vec<(OsString, OsString)>) {
    let mut child = process::Command::new(claude)
        .current_dir(folder)
        .env_clear()
        .envs(environment)
        .args(["-p", PROMPT, "--output-format", "stream-json", "--verbose"])
        .arg("--dangerously-skip-permissions")
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .spawn()
        .expect("run Claude Code");
    let started = Instant::now();
    while child.try_wait().expect("wait for Claude Code").is_none() {
        if started.elapsed() > Duration::from_secs(180) {
            let _ = child.kill();
            panic!("the plain session did not end");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// The user's MCP server, hooks, plugin, skill, instruction file and project memory, planted in
/// the config home, and variables of the device's environment that Claude Code needs neither to run
/// nor to sign in reach a plain session on that config, and none of them reaches an executor's
/// requests to the model or its commands.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn an_executor_gets_none_of_the_user_s_config_and_the_device_s_other_variables() {
    let Some(claude) = claude_code() else { return };

    // The control: the planted config is where Claude Code reads it.
    let world = World::new();
    let (witness, planted) = world.plant();
    let stub = Stub::start(vec![Step::Bash(SEEN)], None);
    plain_session(&claude, &world.folder, world.environment(&stub));
    assert_played(&stub, 1);
    let control = reached(&stub, &witness, &world, &planted);
    assert_eq!(
        control.requested,
        ["the MCP server", "the hooks", "the plugin's hook"],
        "the planted config does not load in a plain session"
    );
    assert_eq!(
        control.tokens,
        [
            "the skill",
            "the plugin's skill",
            "the instruction file",
            "the project memory"
        ],
        "the planted config does not reach a plain session's model"
    );
    assert_eq!(
        control.markers, MARKERS,
        "the variables do not reach a plain session"
    );

    let world = World::new();
    let (witness, planted) = world.plant();

    let (stub, _) = world.execute(&claude, Mode::Full, vec![Step::Bash(SEEN)]);

    let got = reached(&stub, &witness, &world, &planted);
    assert!(
        got.requested.is_empty(),
        "the executor loaded {:?}",
        got.requested
    );
    assert!(
        got.tokens.is_empty(),
        "{:?} reached the executor's model",
        got.tokens
    );
    assert!(
        got.mcp_tools.is_empty(),
        "the executor offered {:?}",
        got.mcp_tools
    );
    assert!(got.markers.is_empty(), "the executor got {:?}", got.markers);
}
