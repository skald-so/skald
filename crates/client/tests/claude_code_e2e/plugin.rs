//! The Claude Code plugin of the monorepo marketplace (`docs/dev/infrastructure.md`, "Channels"),
//! installed as a person installs it, with `claude plugin marketplace add` and `claude plugin
//! install`, into a temporary config home, from a local marketplace of this checkout ([`Market`]):
//! the marketplace's file as `.claude-plugin/marketplace.json` has it, but for the plugin's source,
//! a local git repository whose `main` holds the plugin's directory of this checkout where the
//! source's path names it, as `main` of skald-so/skald holds the latest stable release's
//! ("Release"). Its sessions run as the desktop app and VS Code run Claude Code, on the model stub
//! of `socket_model.rs`, with nothing for Skald but the plugin: no MCP config, settings or hooks of
//! the test's own. The plugin runs the `skald` on `PATH`: this build's, or none.

use std::{
    env,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::claude::claude_code;
use crate::client::EVERYONE;
use crate::common::CLIENT as SKALD;
use crate::machine::{Machine, ONE_SHOT};
use crate::socket_claude::SOCKET;
use crate::socket_model::{Model, Reply, conversation};
use crate::socket_sessions::assert_own_in;
use crate::solo::{API, Solo};
use crate::stub::PLACEHOLDER;
use crate::world::dir_of;

/// The plugin's name, in the marketplace and in its manifest.
const PLUGIN: &str = "skald";

/// How the names Claude Code gives the tools of the plugin's MCP servers start.
const PLUGIN_TOOLS: &str = "mcp__plugin_skald_";

/// Tools of Skald's MCP server that a session of the plugin gets.
const SKALD_TOOLS: [&str; 3] = ["inbox", "send", "reply"];

/// How many characters of a skill's description the tests look for in what reaches the model.
const DESCRIPTION_PART: usize = 40;

/// How long a `claude plugin` command may take.
const COMMAND_LIMIT: Duration = Duration::from_secs(180);

/// How long a session may take to end a turn.
const TURN_LIMIT: Duration = Duration::from_secs(120);

/// How long a session may take to exit once its input closes.
const EXIT_LIMIT: Duration = Duration::from_secs(30);

/// How long a test waits for a turn that must not come.
const NO_MORE: Duration = Duration::from_secs(10);

/// The helper's idle time, in seconds, once no session is registered, as for the socket's sessions.
const HELPER_IDLE: &str = "20";

/// The root of this checkout.
fn checkout() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn text_of(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

fn put_json(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().expect("a file in a directory")).expect("make the directory");
    let text = serde_json::to_string_pretty(value).expect("JSON encodes");
    fs::write(path, text).expect("write a JSON file");
}

/// Copies the directory `from` into the new directory `to`, following links.
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("make a directory of the copy");
    for item in fs::read_dir(from).unwrap_or_else(|error| panic!("{}: {error}", from.display())) {
        let item = item.expect("a directory entry");
        let target = to.join(item.file_name());
        if fs::metadata(item.path())
            .expect("an entry's metadata")
            .is_dir()
        {
            copy_dir(&item.path(), &target);
        } else {
            fs::copy(item.path(), &target).expect("copy a file");
        }
    }
}

/// `path` as a `file://` URL, which git clones.
fn file_url(path: &Path) -> String {
    let slashed = text_of(path.as_os_str()).replace('\\', "/");
    match slashed.starts_with('/') {
        true => format!("file://{slashed}"),
        false => format!("file:///{slashed}"),
    }
}

/// The monorepo marketplace as this checkout's `.claude-plugin/marketplace.json` has it.
struct Catalog {
    /// The marketplace's file.
    file: Value,
    /// The plugin's entry in it.
    entry: Value,
    /// Where the plugin's source holds its directory: the source's path, or its root.
    path: String,
}

impl Catalog {
    fn of_checkout() -> Self {
        let file = read_json(&checkout().join(".claude-plugin").join("marketplace.json"));
        let entry = (file["plugins"].as_array().into_iter().flatten())
            .find(|entry| entry["name"] == PLUGIN)
            .cloned()
            .unwrap_or_else(|| panic!("the marketplace lists no plugin {PLUGIN}"));
        let source = &entry["source"];
        let path = (source.as_str().or(source["path"].as_str()))
            .unwrap_or(".")
            .trim_start_matches("./")
            .to_owned();
        Catalog { file, entry, path }
    }

    /// The plugin's directory in this checkout.
    fn plugin_dir(&self) -> PathBuf {
        checkout().join(&self.path)
    }

    /// The start of the description of each skill of the plugin, as its `SKILL.md` gives it; fails
    /// unless there is one.
    fn skill_descriptions(&self) -> Vec<String> {
        let mut found = Vec::new();
        let mut pending = vec![self.plugin_dir().join("skills")];
        while let Some(dir) = pending.pop() {
            for item in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = item.path();
                if path.is_dir() {
                    pending.push(path);
                } else if item.file_name() == "SKILL.md" {
                    found.push(description_of(&path));
                }
            }
        }
        assert!(!found.is_empty(), "the plugin has no skill");
        found
    }
}

/// The first [`DESCRIPTION_PART`] characters of the description in the front matter of the skill
/// file `path`.
fn description_of(path: &Path) -> String {
    let text = fs::read_to_string(path).expect("read a skill");
    let mut lines = text.lines().skip(1).take_while(|line| *line != "---");
    let mut said = (lines.by_ref())
        .find_map(|line| line.strip_prefix("description:"))
        .unwrap_or_else(|| panic!("{} gives no description", path.display()))
        .trim()
        .trim_matches(|quote| quote == '"' || quote == '\'')
        .to_owned();
    if said.is_empty() || said.starts_with(['>', '|']) {
        said = lines.next().unwrap_or_default().trim().to_owned();
    }
    said.chars().take(DESCRIPTION_PART).collect()
}

/// A local marketplace of this checkout ([`Catalog`]), its plugin's source a local git repository.
struct Market {
    /// The marketplace's name.
    name: String,
    /// The marketplace's directory, which `claude plugin marketplace add` takes.
    dir: PathBuf,
    /// The marketplace's file.
    file: Value,
    /// The plugin's repository and its directory there.
    repo: PathBuf,
    plugin: PathBuf,
    /// An empty git config, the only one the repository's git reads.
    git_config: PathBuf,
}

impl Market {
    /// The marketplace, made under `under`, with the plugin of this checkout on `main`.
    fn new(under: &Path) -> Self {
        let catalog = Catalog::of_checkout();
        let repo = under.join("plugin-repository");
        let plugin = repo.join(&catalog.path);
        copy_dir(&catalog.plugin_dir(), &plugin);
        let git_config = under.join("gitconfig");
        fs::write(&git_config, "").expect("write an empty git config");
        let url = file_url(&repo);
        let mut entry = catalog.entry.clone();
        entry["source"] = match catalog.path.as_str() {
            "." | "" => json!({ "source": "url", "url": url, "ref": "main" }),
            path => json!({ "source": "git-subdir", "url": url, "path": path, "ref": "main" }),
        };
        let mut file = catalog.file.clone();
        file["plugins"] = json!([entry]);
        let market = Market {
            name: (file["name"].as_str())
                .expect("the marketplace's name")
                .to_owned(),
            dir: under.join("marketplace"),
            file,
            repo,
            plugin,
            git_config,
        };
        market.git(&["init", "--quiet"]);
        market.commit("the plugin of this checkout");
        market.write_file();
        market
    }

    /// The plugin as `claude plugin install` takes it.
    fn plugin_id(&self) -> String {
        format!("{PLUGIN}@{}", self.name)
    }

    fn write_file(&self) {
        let path = self.dir.join(".claude-plugin").join("marketplace.json");
        put_json(&path, &self.file);
    }

    fn git(&self, args: &[&str]) {
        let done = Command::new("git")
            .current_dir(&self.repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.git_config)
            .args(["-c", "init.defaultBranch=main", "-c", "core.autocrlf=false"])
            .args([
                "-c",
                "user.name=Skald tests",
                "-c",
                "user.email=tests@skald.invalid",
            ])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("run git");
        let said = String::from_utf8_lossy(&done.stderr);
        assert!(done.status.success(), "git {args:?}: {said}");
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "--message", message]);
    }

    /// Moves `main` to the plugin at `version`, with a skill added whose description is `marker`,
    /// and the marketplace's entry to `version` if it gives one, as a newer release moves them.
    fn move_to(&mut self, version: &str, marker: &str) {
        let manifest = self.plugin.join(".claude-plugin").join("plugin.json");
        let mut fields = read_json(&manifest);
        fields["version"] = json!(version);
        put_json(&manifest, &fields);
        let skill = self.plugin.join("skills").join("e2e-newer-version");
        fs::create_dir_all(&skill).expect("make the newer version's skill");
        let body =
            format!("---\nname: e2e-newer-version\ndescription: {marker}\n---\n\n{marker}\n");
        fs::write(skill.join("SKILL.md"), body).expect("write the newer version's skill");
        self.commit("a newer version");
        if let Some(listed) = self.file["plugins"][0].get_mut("version") {
            *listed = json!(version);
            self.write_file();
        }
    }
}

/// A temporary config home of Claude Code and the environment its commands and sessions run in.
struct Home {
    claude: PathBuf,
    dir: PathBuf,
    env: Vec<(String, String)>,
}

impl Home {
    /// A config home under `machine`'s root; its runs have `machine`'s settings, as a session's
    /// hooks have them in `socket_claude.rs`, and on `PATH` this build's `skald` if `skald`, else no
    /// `skald` at all.
    fn new(claude: &Path, machine: &Machine, skald: bool) -> Self {
        let dir = machine.root().join("tmp").join("claude-plugin-home");
        put_json(
            &dir.join(".claude.json"),
            &json!({ "hasCompletedOnboarding": true }),
        );
        let runs = machine.command_in(machine.root(), &[]);
        let mut settings: Vec<(String, String)> = (runs.get_envs())
            .filter(|(name, _)| !(SOCKET && *name == OsStr::new(ONE_SHOT)))
            .filter_map(|(name, value)| Some((text_of(name), text_of(value?))))
            .filter(|(name, _)| name != "PATH" && name != "HOME")
            .collect();
        let named_skald = |dir: &PathBuf| {
            ["skald", "skald.exe", "skald.cmd"]
                .iter()
                .any(|name| dir.join(name).is_file())
        };
        let mut path: Vec<PathBuf> = env::split_paths(&env::var_os("PATH").unwrap_or_default())
            .filter(|dir| !named_skald(dir))
            .collect();
        if skald {
            path.insert(
                0,
                Path::new(SKALD)
                    .parent()
                    .expect("skald's directory")
                    .to_owned(),
            );
        }
        let path = env::join_paths(path).expect("a PATH");
        let home = text_of(dir.as_os_str());
        settings.extend([
            ("PATH".to_owned(), text_of(&path)),
            ("HOME".to_owned(), home.clone()),
            ("CLAUDE_CONFIG_DIR".to_owned(), home),
            ("SKALD_HELPER_IDLE".to_owned(), HELPER_IDLE.to_owned()),
        ]);
        Home {
            claude: claude.to_owned(),
            dir,
            env: settings,
        }
    }

    /// `claude` with `args`, in the config home's environment.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.claude);
        command
            .env_clear()
            .envs(self.env.iter().map(|(name, value)| (name, value)))
            .args(args);
        command
    }

    /// Runs `claude plugin <args>`; fails unless it succeeds. Gives what it printed.
    #[track_caller]
    fn plugin(&self, args: &[&str]) -> String {
        let mut command = self.command(&[&["plugin"][..], args].concat());
        command.current_dir(&self.dir);
        let (succeeded, printed) = run_to_end(command, COMMAND_LIMIT);
        assert!(succeeded, "claude plugin {args:?} failed: {printed}");
        printed
    }

    /// Adds `market` and installs its plugin, as a person does.
    #[track_caller]
    fn install(&self, market: &Market) {
        self.plugin(&["marketplace", "add", &text_of(market.dir.as_os_str())]);
        self.plugin(&["install", &market.plugin_id()]);
    }

    /// Updates `market` and its plugin, as Claude Code updates them.
    #[track_caller]
    fn update(&self, market: &Market) {
        self.plugin(&["marketplace", "update", &market.name]);
        self.plugin(&["update", &market.plugin_id()]);
    }

    /// The version of `market`'s plugin that `claude plugin list` shows installed.
    #[track_caller]
    fn installed_version(&self, market: &Market) -> String {
        let listed = self.plugin(&["list", "--json"]);
        let at = listed
            .find('[')
            .unwrap_or_else(|| panic!("no list: {listed}"));
        let plugins = serde_json::Deserializer::from_str(&listed[at..])
            .into_iter::<Vec<Value>>()
            .next()
            .and_then(Result::ok)
            .unwrap_or_else(|| panic!("not a JSON list: {listed}"));
        let id = market.plugin_id();
        let installed = (plugins.iter())
            .find(|plugin| plugin["id"] == id.as_str())
            .unwrap_or_else(|| panic!("{id} is not installed: {listed}"));
        installed["version"].as_str().unwrap_or_default().to_owned()
    }
}

/// Runs `command` to its end, for up to `limit`; gives whether it succeeded and what it printed.
fn run_to_end(mut command: Command, limit: Duration) -> (bool, String) {
    let mut running = (command.stdin(Stdio::null()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run Claude Code");
    let printed = Arc::new(Mutex::new(Vec::new()));
    keep(running.stdout.take().expect("piped"), &printed);
    keep(running.stderr.take().expect("piped"), &printed);
    let deadline = Instant::now() + limit;
    let status = loop {
        match running.try_wait().expect("wait for Claude Code") {
            Some(status) => break status,
            None if Instant::now() > deadline => {
                let _ = running.kill();
                let _ = running.wait();
                panic!("ran for more than {limit:?}: {}", shown(&printed));
            }
            None => thread::sleep(Duration::from_millis(100)),
        }
    };
    // The readers end with the outputs.
    thread::sleep(Duration::from_millis(200));
    (status.success(), shown(&printed))
}

/// Appends what `source` gives to `sink` until it ends, on a thread of its own.
fn keep(mut source: impl Read + Send + 'static, sink: &Arc<Mutex<Vec<u8>>>) {
    let sink = Arc::clone(sink);
    thread::spawn(move || {
        let mut chunk = [0_u8; 8192];
        while let Ok(got) = source.read(&mut chunk) {
            if got == 0 {
                return;
            }
            let mut kept = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            kept.extend_from_slice(&chunk[..got]);
        }
    });
}

fn shown(printed: &Arc<Mutex<Vec<u8>>>) -> String {
    let bytes = printed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A session of the plugin's, run as the desktop app and VS Code run Claude Code; its input
/// closed, then killed, on drop.
struct Session {
    child: Child,
    input: Option<ChildStdin>,
    printed: Arc<Mutex<Vec<u8>>>,
}

impl Session {
    /// Starts `claude` in `home`'s environment in `dir` on `model`; it takes no prompt yet.
    fn start(home: &Home, dir: &Path, model: &Model) -> Self {
        let args = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
        ];
        let mut command = home.command(&args);
        command
            .args(["--verbose", "--include-hook-events"])
            .current_dir(dir)
            .env("ANTHROPIC_BASE_URL", model.url())
            .env("ANTHROPIC_API_KEY", PLACEHOLDER)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("run Claude Code");
        let printed = Arc::new(Mutex::new(Vec::new()));
        keep(child.stdout.take().expect("piped"), &printed);
        keep(child.stderr.take().expect("piped"), &printed);
        let input = child.stdin.take();
        Session {
            child,
            input,
            printed,
        }
    }

    /// The person sends the session a prompt, as the apps do.
    fn prompt(&mut self, text: &str) {
        let line = json!({ "type": "user", "message": { "role": "user", "content": text } });
        let input = self.input.as_mut().expect("the session's input is open");
        writeln!(input, "{line}").expect("send a prompt");
        input.flush().expect("send a prompt");
    }

    /// The JSON events the session printed so far.
    fn events(&self) -> Vec<Value> {
        (shown(&self.printed).lines())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// Waits until the session's `count`th turn ended; gives its result.
    #[track_caller]
    fn wait_result(&self, count: usize) -> Value {
        let deadline = Instant::now() + TURN_LIMIT;
        loop {
            let results: Vec<Value> = (self.events().into_iter())
                .filter(|event| event["type"] == "result")
                .collect();
            if let Some(result) = results.get(count - 1) {
                return result.clone();
            }
            assert!(
                Instant::now() < deadline,
                "turn {count} did not end: {}",
                shown(&self.printed)
            );
            thread::sleep(Duration::from_millis(200));
        }
    }

    /// The responses of the session's hooks of `event` so far.
    fn hook_responses(&self, event: &str) -> Vec<Value> {
        (self.events().into_iter())
            .filter(|value| value["subtype"] == "hook_response" && value["hook_event"] == event)
            .collect()
    }

    /// Closes the session's input and waits for it to exit; kills it if it does not in time. Gives
    /// whether it exited by itself, with success.
    fn end(&mut self) -> bool {
        drop(self.input.take());
        let deadline = Instant::now() + EXIT_LIMIT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().ok().flatten() {
                return status.success();
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        false
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.end();
    }
}

/// Every string of the request `body` that reaches the model: its system prompt, its conversation
/// and its tools, a line apart.
fn reaching_the_model(body: &Value) -> String {
    let parts = [&body["system"], &body["messages"], &body["tools"]];
    conversation(&json!({ "messages": parts }))
}

/// The names of the plugin's tools that the request `body` offers the model: in its tools, or by
/// name for Claude Code's tool search.
fn plugin_tools(body: &Value) -> Vec<String> {
    let identifier = |part: char| part.is_ascii_alphanumeric() || part == '_';
    (reaching_the_model(body).split(|part: char| !identifier(part)))
        .filter(|name| name.starts_with(PLUGIN_TOOLS))
        .map(str::to_owned)
        .collect()
}

/// Where a test keeps its marketplace on `machine`.
fn scratch(machine: &Machine) -> PathBuf {
    machine.root().join("tmp").join("plugin")
}

/// The prompts of the sessions; each is nowhere else.
const OPENS: &str = "e2e plugin test: wait for messages";
const NUDGE: &str = "e2e plugin test: anything new?";
const UPDATED: &str = "e2e plugin test: run the updated plugin";
const ALONE: &str = "e2e plugin test: run without skald";

/// What api says to every agent in `clienta`; it is nowhere else.
const OWN_SAYS: &str = "e2e plugin test: api asks the plugin's session to rerun the migration";

/// The plugin, installed from the local marketplace into a temporary config home, runs a session in
/// `~/work/clientA/plugin`, a bound folder; its prompt's turn ends. That turn offers the inbox, send
/// and reply tools of the plugin's MCP server, and what reached the model holds each skill of the
/// plugin by its description. api's send tool sends every agent in `clienta` a message, `to` `*`:
/// without another prompt, where the client writes into the socket, a turn of the session holds it
/// as its line from api at `own`; elsewhere once the person gives it a prompt, by the plugin's
/// hooks. The stub got no credential but its placeholder.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn installed_plugin_gives_a_bound_session_skald_s_tools_its_skill_and_an_own_message() {
    let Some(claude) = claude_code() else { return };
    let solo = Solo::new(4_919_901_001, "ada-installs-the-claude-plugin");
    let market = Market::new(&scratch(solo.machine()));
    let home = Home::new(&claude, solo.machine(), true);
    home.install(&market);
    let model = Model::start(|_| Reply::Done);
    let mut session = Session::start(&home, &solo.dir("plugin"), &model);
    session.prompt(OPENS);
    session.wait_result(1);

    let first = model.wait_turn_holding(OPENS, "the session's first turn");
    let tools = plugin_tools(&first);
    for tool in SKALD_TOOLS {
        let suffix = format!("__{tool}");
        assert!(
            tools.iter().any(|name| name.ends_with(&suffix)),
            "no {tool} among the plugin's tools {tools:?}"
        );
    }
    let reached = reaching_the_model(&first);
    for description in Catalog::of_checkout().skill_descriptions() {
        assert!(
            reached.contains(&description),
            "the skill {description:?} did not reach the model"
        );
    }

    let mut api = solo.session(API);
    let msg_id = solo.sends(&mut api, EVERYONE, OWN_SAYS);
    if !SOCKET {
        session.prompt(NUDGE);
    }
    let turn = model.wait_turn_holding(OWN_SAYS, "api's message");
    assert_own_in(&turn, OWN_SAYS, msg_id, &solo.short(API));
    session.end();
    model.assert_no_foreign_credential();
}

/// The newer version the local marketplace moves to, and the description of the skill only it has.
const NEWER: &str = "99.0.0";
const NEWER_SKILL: &str = "e2e plugin test: a skill only the newer version has";

/// The plugin, installed from the local marketplace, has the release's version, the workspace's.
/// The marketplace moves to a newer version, with a skill added; once Claude Code updates the
/// marketplace and the plugin, the installed plugin has the newer version, and a new session's
/// model gets the added skill.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn installed_plugin_updates_to_a_newer_version_of_the_marketplace() {
    let Some(claude) = claude_code() else { return };
    let machine = Machine::new();
    let mut market = Market::new(&scratch(&machine));
    let home = Home::new(&claude, &machine, true);
    home.install(&market);
    assert_eq!(
        home.installed_version(&market),
        env!("CARGO_PKG_VERSION"),
        "the installed plugin does not have the release's version"
    );

    market.move_to(NEWER, NEWER_SKILL);
    home.update(&market);

    assert_eq!(home.installed_version(&market), NEWER);
    let model = Model::start(|_| Reply::Done);
    let mut session = Session::start(&home, &dir_of(&machine, "work/updated"), &model);
    session.prompt(UPDATED);
    session.wait_result(1);
    let turn = model.wait_turn_holding(UPDATED, "the updated session's turn");
    assert!(
        reaching_the_model(&turn).contains(NEWER_SKILL),
        "the newer version's skill did not reach the model"
    );
    assert!(session.end(), "the session did not exit with success");
}

/// With no `skald` on `PATH`, a session of the installed plugin takes its prompt, and its turn ends
/// with success after one request to the model: the plugin's `Stop` hook ran and kept nothing going,
/// and no more request comes. The session exits with success once its input closes.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn without_skald_on_path_a_session_of_the_plugin_runs_and_no_hook_keeps_its_turn_going() {
    let Some(claude) = claude_code() else { return };
    let machine = Machine::new();
    let market = Market::new(&scratch(&machine));
    let home = Home::new(&claude, &machine, false);
    home.install(&market);
    let model = Model::start(|_| Reply::Done);
    let mut session = Session::start(&home, &dir_of(&machine, "work/no-skald"), &model);

    session.prompt(ALONE);
    let result = session.wait_result(1);
    thread::sleep(NO_MORE);

    assert_eq!(result["subtype"], "success", "{result}");
    assert!(
        !session.hook_responses("Stop").is_empty(),
        "the plugin's Stop hook did not run: {}",
        shown(&session.printed)
    );
    let turns = model.turns();
    assert_eq!(turns.len(), 1, "a hook kept the turn going: {turns:?}");
    assert!(session.end(), "the session did not exit with success");
    model.assert_no_foreign_credential();
}
