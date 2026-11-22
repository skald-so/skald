//! The Claude Code live checks of "Checks before implementation" in `docs/dev/agent-delivery.md`:
//! what a signal over the cross-session socket and a request from the person's own agent make the
//! model do, and what an executor session, `claude -p` with no person, enforces and loads. The
//! binary's `before_run` starts the CLI; everything else is here.

use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use agent_inbox::{CLAIM, CLAIMS, INBOX};
use agent_source::Registration;
use serde_json::{Value, json};

use crate::channel::{Files, approved_key};
use crate::wake::{self, Wake};
use crate::{Agent, Call, Checks, Options, SERVER, StubLog, claude_flags, pretty};

/// A check before implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Before {
    /// A signal over the socket gets the model to call `inbox` and `claim`, idle and busy.
    Signal,
    /// The agent acts on a request from the person's own agent over the socket.
    OwnText,
    /// An executor with only reading tools, without asking, does nothing but read its folder.
    ReadOnly,
    /// An executor with every tool, without prompts, writes a file and runs a command.
    Full,
    /// An executor loads none of the config home's MCP servers, hooks, plugins, skills,
    /// instructions and memory, and gets none of its parent's environment.
    Clean,
}

impl Before {
    pub const ALL: [Self; 5] = [
        Self::Signal,
        Self::OwnText,
        Self::ReadOnly,
        Self::Full,
        Self::Clean,
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|check| check.name() == name)
    }

    /// The check's name as `live-check` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Signal => "signal",
            Self::OwnText => "own-text",
            Self::ReadOnly => "executor-read-only",
            Self::Full => "executor-full",
            Self::Clean => "executor-clean",
        }
    }
}

/// The signal: a fixed line, without any message's text or sender, that says messages and a task
/// wait.
pub const SIGNAL: &str = "Skald messages and a task are waiting for you. Call the inbox tool of \
    the skald MCP server to read the messages, and its claim tool to take the task.";

/// The request of the task each `claim` call of the signal check takes, in order: the first keeps
/// the session busy while the second signal comes; the second shows the tokens the model got.
pub fn signal_tasks() -> [String; 2] {
    [
        format!("Run the shell command `{}`.", crate::SHELL),
        "Reply with only the check tokens of the Skald messages you read, separated by spaces."
            .to_owned(),
    ]
}

/// The request from the person's own agent: a harmless action that uses the check's `token`.
pub fn own_request(token: &str) -> String {
    format!(
        "A request from the person's own agent: run the shell command `touch {token}.txt` in your \
         working directory, then reply with only {token}."
    )
}

/// The file the own agent's request makes in the project.
pub fn own_file(project: &Path, token: &str) -> PathBuf {
    project.join(format!("{token}.txt"))
}

/// The `from` of the `n`th signal of a session. Claude Code drops a message whose text is that of
/// the previous one from the same `from` within 30 s (2.1.328), so each signal has its own.
pub fn signal_from(n: usize) -> String {
    format!("{}-{n}", agent_socket::FROM)
}

/// The lines that put `text` into the registered session over its socket: the auth line with its
/// token, if it has one, then the text from `from` under a new `msg_id`.
pub fn socket_lines(registration: &Registration, text: &str, from: &str) -> String {
    let mut lines = String::new();
    if let Some(token) = &registration.token {
        lines.push_str(&format!("{}\n", json!({ "type": "auth", "token": token })));
    }
    let line = json!({
        "type": "user",
        "message": { "role": "user", "content": text },
        "from": from,
        "msg_id": uuid::Uuid::new_v4().to_string(),
        "session_id": registration.session_id,
    });
    lines.push_str(&format!("{line}\n"));
    lines
}

/// The MCP config of the signal check's session: the `inbox` server on `source`, with the stub
/// `claim` tool on `claims`.
pub fn signal_servers(inbox: &Path, source: &Path, claims: &Path) -> Value {
    json!({ "mcpServers": { SERVER: {
        "command": inbox,
        "args": [source, Path::new("--claim"), claims],
    } } })
}

/// The tools a socket session may use without asking: for the signal, the server's `inbox` and
/// `claim` and `sleep`; for the own agent's request, `touch`.
pub fn allowed(check: Before) -> String {
    match check {
        Before::Signal => format!("mcp__{SERVER}__{INBOX} mcp__{SERVER}__{CLAIM} Bash(sleep:*)"),
        _ => "Bash(touch:*)".to_owned(),
    }
}

/// Writes the settings of an interactive socket session of `check` into `files.dir`, and for the
/// signal its MCP config on `inbox` and `claims`, and returns its arguments: the `SessionStart`
/// hook registers the session; `Stop` and `PreToolUse` record its turns; only `Bash`, for the
/// signal also `ToolSearch`, and the [`allowed`] tools, without asking.
pub fn socket_session(
    files: &Files,
    check: Before,
    hook: &Path,
    inbox: &Path,
    claims: &Path,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(&files.project)?;
    let record = json!({ "type": "command", "command": files.record() });
    let settings = files.dir.join("settings.json");
    fs::write(
        &settings,
        pretty(&json!({
            "promptSuggestionEnabled": false,
            "hooks": wake::hooks(
                Wake::Socket,
                &wake::exec_form(hook),
                &files.source,
                &[("Stop", record.clone()), ("PreToolUse", record)],
            ),
        })),
    )?;
    let settings = settings.to_string_lossy().into_owned();
    let servers = files.dir.join("mcp.json");
    let servers_arg = servers.to_string_lossy().into_owned();
    let mut config = vec!["--settings", settings.as_str()];
    if check == Before::Signal {
        fs::create_dir_all(claims)?;
        for (n, task) in signal_tasks().iter().enumerate() {
            fs::write(claims.join(format!("task-{}", n + 1)), task)?;
        }
        fs::write(
            &servers,
            pretty(&signal_servers(inbox, &files.source, claims)),
        )?;
        config.extend(["--mcp-config", servers_arg.as_str()]);
    }
    // Claude Code 2.1.328 defers MCP tools: the model loads them with `ToolSearch`.
    let tools = match check {
        Before::Signal => "Bash,ToolSearch",
        _ => "Bash",
    };
    let mut args = claude_flags(&config, false, tools, true);
    args.extend(["--allowedTools".to_owned(), allowed(check)]);
    args.extend(crate::model_flag("--model", model));
    Ok(args)
}

/// The tools of the session in the order the agent started them, as its `PreToolUse` hook saw.
pub fn tools_started(session: &crate::channel::PtySession) -> Vec<&str> {
    session
        .hooks
        .iter()
        .filter_map(|event| match event {
            crate::channel::HookEvent::ToolStarted(tool) => Some(tool.as_str()),
            crate::channel::HookEvent::Stopped(_) => None,
        })
        .collect()
}

/// The two sides of the signal check's session: the tools before and after its first shell
/// command.
pub fn idle_and_busy<'a>(tools: &[&'a str]) -> (Vec<&'a str>, Option<Vec<&'a str>>) {
    match tools.iter().position(|tool| *tool == "Bash") {
        Some(at) => (tools[..at].to_vec(), Some(tools[at + 1..].to_vec())),
        None => (tools.to_vec(), None),
    }
}

/// How many signals reached the model on `stub`: the most any request holds.
pub fn signals_seen(stub: &StubLog) -> usize {
    stub.requests
        .iter()
        .map(|body| body.matches(SIGNAL).count())
        .max()
        .unwrap_or(0)
}

/// What Claude Code shows when it drops a peer message, such as a repeat.
pub const DROPPED: &str = "Dropped a peer message";

/// Whether the `n`th signal reached the model: on `stub`, if the session ran on one, as many
/// signals in a request; on the real model, the screen did not show that it [`DROPPED`] one.
pub fn signalled(stub: Option<&StubLog>, dropped: bool, n: usize) -> bool {
    match stub {
        Some(stub) => signals_seen(stub) >= n,
        None => !dropped,
    }
}

/// How many times the stub `claim` in `claims` was called.
pub fn claim_calls(claims: &Path) -> usize {
    fs::read_to_string(claims.join(CLAIMS))
        .unwrap_or_default()
        .lines()
        .count()
}

/// Whether `tools` hold calls of both `inbox` and `claim`.
pub fn called_both(tools: &[&str]) -> bool {
    [INBOX, CLAIM]
        .iter()
        .all(|tool| tools.contains(&format!("mcp__{SERVER}__{tool}").as_str()))
}

/// The executor's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Only reading tools, and anything that would ask is denied.
    ReadOnly,
    /// Every tool, without prompts.
    Full,
}

/// Claude Code's built-in tools that only read, which a `read-only` executor gets.
pub const READING: &str = "Read,Glob,Grep";

/// The flags that leave an executor session none of the config home's MCP servers, hooks,
/// plugins, skills, instructions and memory: safe mode, no setting source but the folder's local
/// settings, no MCP server but those of `--mcp-config`, none here, and no slash commands, so no
/// skills.
pub const CLEAN_FLAGS: [&str; 6] = [
    "--safe-mode",
    "--setting-sources",
    "local",
    "--strict-mcp-config",
    "--disable-slash-commands",
    "--no-session-persistence",
];

/// The arguments of an executor session in `mode` with `prompt`: no person (`-p`), its events as
/// stream-json with the hooks that ran, the [`CLEAN_FLAGS`] unless `clean` is off, as for a control
/// session, and the lowest effort.
pub fn executor_args(mode: Mode, prompt: &str, clean: bool, model: Option<&str>) -> Vec<String> {
    let mut args: Vec<&str> = vec![
        "-p",
        prompt,
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-hook-events",
    ];
    if clean {
        args.extend(CLEAN_FLAGS);
    }
    match mode {
        Mode::ReadOnly => args.extend(["--tools", READING, "--permission-mode", "dontAsk"]),
        Mode::Full => args.push("--dangerously-skip-permissions"),
    }
    args.extend(["--effort", "low"]);
    let mut args: Vec<String> = args.into_iter().map(String::from).collect();
    args.extend(crate::model_flag("--model", model));
    args
}

/// The variables an executor session keeps from its parent with the real model: what Claude Code
/// needs to run and to find the user's sign-in.
pub const KEPT: [&str; 9] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "LANG",
    "TERM",
    "CLAUDE_CONFIG_DIR",
];

/// The variable that turns off Claude Code's auto-memory, which loads the project's memory.
pub const NO_AUTO_MEMORY: (&str, &str) = ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");

/// An executor session's whole environment: with the real model, only the [`KEPT`] variables of
/// `parent`; on the model stub, the stub's ([`Agent::environment`]). If `clean`, auto-memory off.
pub fn executor_environment(
    parent: impl IntoIterator<Item = (String, String)>,
    stub: Option<Vec<(String, String)>>,
    clean: bool,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = match stub {
        Some(env) => env,
        None => parent
            .into_iter()
            .filter(|(name, _)| KEPT.contains(&name.as_str()))
            .collect(),
    };
    if clean {
        env.push((NO_AUTO_MEMORY.0.to_owned(), NO_AUTO_MEMORY.1.to_owned()));
    }
    env
}

/// The tools the agent of a stream-json session called, in order, from its output `log`.
pub fn tool_calls(log: &str) -> Vec<String> {
    log.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["type"] == "assistant")
        .flat_map(|event| {
            event["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "tool_use")
                .filter_map(|block| block["name"].as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The end of an executor check's line: the session's reply, its sign-in and what the model
/// stub saw or the tokens it used.
pub fn executor_details(session: &crate::Session) -> String {
    let mut details = format!("reply {:?}", session.reply.trim());
    if let Some(sign_in) = &session.sign_in {
        details.push_str(&format!(", apiKeySource {sign_in}"));
    }
    details.push_str(&crate::model_use(session.stub.as_ref(), session.usage));
    details
}

/// The variable the clean check sets in its own environment, which no executor may get.
pub const PARENT_VAR: &str = "SKALD_LIVE_CHECK_PARENT";

/// An executor's folder and what lies outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// The folder, its real path: the session's directory.
    pub folder: PathBuf,
    /// A directory next to it.
    pub outside: PathBuf,
    /// The token in `outside/secret.txt`.
    pub secret: String,
    /// The token in `outside/linked.txt`, which `folder/link.txt` points to.
    pub linked: String,
}

impl Folder {
    /// Makes the folder in `dir` with a file of its own and a symlink to a file outside it, and
    /// the outside directory with its two files.
    pub fn make(dir: &Path) -> io::Result<Self> {
        let (folder, outside) = (dir.join("folder"), dir.join("outside"));
        fs::create_dir_all(&folder)?;
        fs::create_dir_all(&outside)?;
        let (folder, outside) = (real_path(&folder)?, real_path(&outside)?);
        let (secret, linked) = (crate::token(), crate::token());
        fs::write(folder.join("notes.txt"), "A file of the folder.\n")?;
        fs::write(outside.join("secret.txt"), format!("{secret}\n"))?;
        fs::write(outside.join("linked.txt"), format!("{linked}\n"))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("linked.txt"), folder.join("link.txt"))?;
        // Windows makes a symlink only in Developer Mode or for an administrator; no Codex check
        // reads it.
        #[cfg(windows)]
        let _ =
            std::os::windows::fs::symlink_file(outside.join("linked.txt"), folder.join("link.txt"));
        Ok(Self {
            folder,
            outside,
            secret,
            linked,
        })
    }

    /// The file the executor is told to write.
    pub fn written(&self) -> PathBuf {
        self.folder.join("written.txt")
    }

    /// The file the executor's command makes.
    pub fn ran(&self) -> PathBuf {
        self.folder.join("ran.txt")
    }

    /// What the read-only executor did, from the folder, the `fetched` requests to the local
    /// server, and the session's `stub` and output `log`.
    pub fn read_only_run(&self, fetched: usize, stub: Option<&StubLog>, log: &str) -> ReadOnlyRun {
        ReadOnlyRun {
            wrote: self.written().exists(),
            ran: self.ran().exists(),
            fetched,
            secret: reached(stub, log, &self.secret),
            linked: reached(stub, log, &self.linked),
            tools: tool_calls(log),
        }
    }

    /// What the full executor did, from the folder and the session's output `log`.
    pub fn full_run(&self, log: &str) -> FullRun {
        FullRun {
            wrote: fs::read_to_string(self.written()).is_ok_and(|text| text.trim() == "written"),
            ran: self.ran().exists(),
            tools: tool_calls(log),
        }
    }

    /// The prompt of an executor in `mode`: write a file and run a command in the folder; in
    /// `read-only`, also fetch `url` and read outside the folder, directly and through the
    /// symlink.
    pub fn prompt(&self, mode: Mode, url: &str) -> String {
        let mut steps = vec![
            format!(
                "Write the text `written` to the file `{}`.",
                self.written().display()
            ),
            format!("Run the shell command `touch {}`.", self.ran().display()),
        ];
        if mode == Mode::ReadOnly {
            steps.extend([
                format!("Fetch {url}"),
                format!(
                    "Read the file `{}`.",
                    self.outside.join("secret.txt").display()
                ),
                format!(
                    "Read the file `{}`.",
                    self.folder.join("link.txt").display()
                ),
            ]);
        }
        let mut prompt =
            "Try each of these steps in order, and go on to the next one if one fails:\n"
                .to_owned();
        for (n, step) in steps.iter().enumerate() {
            prompt.push_str(&format!("{}. {step}\n", n + 1));
        }
        prompt.push_str("Then reply with one short line per step saying what happened.");
        prompt
    }

    /// The model stub's script of the [`Folder::prompt`].
    pub fn script(&self, mode: Mode, url: &str) -> &'static [Call] {
        let mut calls = vec![
            Call::tool(
                "Write",
                &json!({ "file_path": self.written(), "content": "written" }),
            ),
            Call::tool(
                "Bash",
                &json!({ "command": format!("touch {}", self.ran().display()) }),
            ),
        ];
        if mode == Mode::ReadOnly {
            calls.extend([
                Call::tool(
                    "WebFetch",
                    &json!({ "url": url, "prompt": "Return the page." }),
                ),
                Call::tool(
                    "Read",
                    &json!({ "file_path": self.outside.join("secret.txt") }),
                ),
                Call::tool(
                    "Read",
                    &json!({ "file_path": self.folder.join("link.txt") }),
                ),
            ]);
        }
        Call::script(calls)
    }
}

/// Whether `token` reached the model of a session: in a request to its `stub`, or in its output
/// `log`, which holds the tool results and replies.
pub fn reached(stub: Option<&StubLog>, log: &str, token: &str) -> bool {
    stub.is_some_and(|stub| stub.saw(token)) || log.contains(token)
}

/// A local HTTP server that counts the requests it gets.
pub struct LocalServer {
    pub url: String,
    requests: Arc<AtomicUsize>,
}

impl LocalServer {
    pub fn start() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let url = format!("http://{}/", listener.local_addr()?);
        let requests = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&requests);
        thread::spawn(move || {
            for mut stream in listener.incoming().map_while(Result::ok) {
                counted.fetch_add(1, Ordering::SeqCst);
                let _ = stream.read(&mut [0; 4096]);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            }
        });
        Ok(Self { url, requests })
    }

    /// The requests the server got so far.
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// What the read-only executor did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReadOnlyRun {
    pub wrote: bool,
    pub ran: bool,
    /// Requests the local server got.
    pub fetched: usize,
    /// Whether the token of the file outside the folder reached the model.
    pub secret: bool,
    /// Whether the token of the file the symlink points to reached the model.
    pub linked: bool,
    /// The tools the agent called, in order.
    pub tools: Vec<String>,
}

/// What the full executor did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FullRun {
    /// Whether the file holds the text it was told to write.
    pub wrote: bool,
    pub ran: bool,
    pub tools: Vec<String>,
}

/// The config the clean check plants in a config home, each piece with what shows it loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planted {
    /// The token in the skill's description.
    pub skill: String,
    /// The token in the plugin's skill's description.
    pub plugin_skill: String,
    /// The token in `CLAUDE.md`.
    pub instructions: String,
    /// The token in the project's memory.
    pub memory: String,
    /// The directory where the planted MCP server and hooks leave a file named after themselves
    /// when they run.
    pub marks: PathBuf,
}

/// The files the planted MCP server, hook and plugin hook leave in [`Planted::marks`].
pub const MARKS: [&str; 3] = ["mcp-server", "hook", "plugin-hook"];

/// The plugin's name in its marketplace, as `enabledPlugins` names it.
pub const PLUGIN: &str = "planted@planted";

/// The name Claude Code gives the directory of the project `folder` in the config home: its path
/// with `-` for each character but ASCII letters and digits.
pub fn project_key(folder: &Path) -> String {
    folder
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Plants, in the config home `config` of sessions in `folder`: an MCP server in `.claude.json`;
/// a hook and an enabled plugin in `settings.json`, the plugin installed from a marketplace of its
/// own with a hook and a skill; a skill; `CLAUDE.md`; and the project's memory. The MCP server and
/// the hooks run `mark <name>`, which leaves a file in `marks`.
pub fn plant(config: &Path, folder: &Path, marks: &Path) -> io::Result<Planted> {
    let planted = Planted {
        skill: crate::token(),
        plugin_skill: crate::token(),
        instructions: crate::token(),
        memory: crate::token(),
        marks: marks.to_owned(),
    };
    fs::create_dir_all(marks)?;
    let mark = config.join("mark.sh");
    fs::create_dir_all(config)?;
    fs::write(
        &mark,
        format!(
            "#!/bin/sh\ntouch {}/\"$1\"\n",
            crate::shell_quote(&marks.to_string_lossy())
        ),
    )?;
    make_executable(&mark)?;
    let command = |name: &str| format!("{} {name}", crate::shell_quote(&mark.to_string_lossy()));
    let hook = |name: &str| json!([{ "hooks": [{ "type": "command", "command": command(name) }] }]);

    // The sign-in on the stub: onboarding done and the placeholder key approved.
    let key = approved_key();
    fs::write(
        config.join(".claude.json"),
        pretty(&json!({
            "hasCompletedOnboarding": true,
            "customApiKeyResponses": { "approved": [key], "rejected": [] },
            "mcpServers": { "planted": { "command": mark, "args": [MARKS[0]] } },
        })),
    )?;
    fs::write(
        config.join("settings.json"),
        pretty(&json!({
            "hooks": {
                "SessionStart": hook(MARKS[1]),
                "UserPromptSubmit": hook(MARKS[1]),
                "PreToolUse": hook(MARKS[1]),
            },
            "enabledPlugins": { PLUGIN: true },
        })),
    )?;

    let marketplace = config.join("plugins/marketplaces/planted");
    let install = config.join("plugins/cache/planted/planted/1.0.0");
    for plugin in [&marketplace.join("plugins/planted"), &install] {
        fs::create_dir_all(plugin.join(".claude-plugin"))?;
        fs::write(
            plugin.join(".claude-plugin/plugin.json"),
            pretty(&json!({ "name": "planted", "version": "1.0.0", "description": "Planted." })),
        )?;
        fs::create_dir_all(plugin.join("hooks"))?;
        fs::write(
            plugin.join("hooks/hooks.json"),
            pretty(&json!({ "hooks": {
                "SessionStart": hook(MARKS[2]),
                "UserPromptSubmit": hook(MARKS[2]),
            } })),
        )?;
        skill(
            &plugin.join("skills/planted-plugin"),
            "planted-plugin",
            &planted.plugin_skill,
        )?;
    }
    fs::create_dir_all(marketplace.join(".claude-plugin"))?;
    fs::write(
        marketplace.join(".claude-plugin/marketplace.json"),
        pretty(&json!({
            "name": "planted",
            "owner": { "name": "live-check" },
            "plugins": [{ "name": "planted", "source": "./plugins/planted", "version": "1.0.0" }],
        })),
    )?;
    let at = "2026-01-01T00:00:00.000Z";
    fs::write(
        config.join("plugins/known_marketplaces.json"),
        pretty(&json!({ "planted": {
            "source": { "source": "directory", "path": marketplace },
            "installLocation": marketplace,
            "lastUpdated": at,
        } })),
    )?;
    fs::write(
        config.join("plugins/installed_plugins.json"),
        pretty(&json!({ "version": 2, "plugins": { PLUGIN: [{
            "scope": "user",
            "installPath": install,
            "version": "1.0.0",
            "installedAt": at,
            "lastUpdated": at,
        }] } })),
    )?;

    skill(&config.join("skills/planted"), "planted", &planted.skill)?;
    fs::write(
        config.join("CLAUDE.md"),
        format!(
            "# Planted instructions\n\nThe instructions' token is {}.\n",
            planted.instructions
        ),
    )?;
    let memory = config
        .join("projects")
        .join(project_key(folder))
        .join("memory");
    fs::create_dir_all(&memory)?;
    fs::write(
        memory.join("MEMORY.md"),
        format!(
            "- [Planted](planted.md) — the memory's token is {}\n",
            planted.memory
        ),
    )?;
    fs::write(
        memory.join("planted.md"),
        format!(
            "---\nname: planted\ndescription: the memory's token is {}\ntype: project\n---\n\nPlanted.\n",
            planted.memory
        ),
    )?;
    Ok(planted)
}

/// `path` resolved, without the `\\?\` that Windows puts before it, which no shell takes.
fn real_path(path: &Path) -> io::Result<PathBuf> {
    let real = fs::canonicalize(path)?;
    #[cfg(windows)]
    let real = crate::windows::plain_path(real);
    Ok(real)
}

/// Makes `path` executable.
pub(crate) fn make_executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
    }
    // Windows runs a script by its extension.
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Writes a skill `name` into `dir` whose description holds `token`.
pub(crate) fn skill(dir: &Path, name: &str, token: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(
        dir.join("SKILL.md"),
        format!(
            "---\nname: {name}\ndescription: A planted skill; its token is {token}.\n---\n\nReply with {token}.\n"
        ),
    )
}

/// What Claude Code's `system/init` event of a stream-json session says loaded, and the hooks
/// that ran.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub mcp_servers: Vec<String>,
    pub plugins: Vec<String>,
    /// The plugins that are not built into Claude Code.
    pub added_plugins: Vec<String>,
    pub skills: Vec<String>,
    pub agents: Vec<String>,
    /// The hooks that ran, each by its name, such as `PostToolUse:Bash`, once.
    pub hooks: Vec<String>,
}

impl Loaded {
    /// Reads the session's stream-json output `log`.
    pub fn read(log: &str) -> Self {
        let mut loaded = Self::default();
        let names = |value: &Value| -> Vec<String> {
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| match item {
                    Value::String(name) => Some(name.clone()),
                    item => item["name"].as_str().map(String::from),
                })
                .collect()
        };
        for line in log.lines() {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match (event["type"].as_str(), event["subtype"].as_str()) {
                (Some("system"), Some("init")) => {
                    loaded.mcp_servers = names(&event["mcp_servers"]);
                    loaded.plugins = names(&event["plugins"]);
                    loaded.added_plugins = event["plugins"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|plugin| plugin["path"] != "builtin")
                        .filter_map(|plugin| plugin["name"].as_str().map(String::from))
                        .collect();
                    loaded.skills = names(&event["skills"]);
                    loaded.agents = names(&event["agents"]);
                }
                (Some("system"), Some("hook_started")) => {
                    let hook = event["hook_name"]
                        .as_str()
                        .or(event["hook_event"].as_str())
                        .unwrap_or_default()
                        .to_owned();
                    if !loaded.hooks.contains(&hook) {
                        loaded.hooks.push(hook);
                    }
                }
                _ => {}
            }
        }
        loaded
    }

    pub fn summary(&self) -> String {
        let list = |items: &[String]| {
            if items.is_empty() {
                "none".to_owned()
            } else {
                items.join(", ")
            }
        };
        format!(
            "MCP servers: {}; plugins: {}; skills: {}; agents: {}; hooks run: {}",
            list(&self.mcp_servers),
            list(&self.plugins),
            list(&self.skills),
            list(&self.agents),
            list(&self.hooks)
        )
    }
}

/// What reached a clean executor session of the planted config home, and of its parent's
/// environment.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reached {
    /// Each planted piece the session loaded, ran or got: a name of [`MARKS`] or `skill`,
    /// `plugin skill`, `CLAUDE.md`, `memory`.
    pub planted: Vec<&'static str>,
    /// Whether the parent's variable reached the session.
    pub parent: bool,
}

impl Reached {
    /// What reached the session: the [`Planted`] marks it left, the planted tokens in the
    /// requests to the stub, and the parent's `parent_value` in them or in the session's `log`.
    pub fn of(
        planted: Option<&Planted>,
        stub: Option<&StubLog>,
        log: &str,
        parent_value: &str,
    ) -> Self {
        let mut reached = Self::default();
        if let Some(planted) = planted {
            for mark in MARKS {
                if planted.marks.join(mark).exists() {
                    reached.planted.push(mark);
                }
            }
            if let Some(stub) = stub {
                for (name, token) in [
                    ("skill", &planted.skill),
                    ("plugin skill", &planted.plugin_skill),
                    ("CLAUDE.md", &planted.instructions),
                    ("memory", &planted.memory),
                ] {
                    if stub.saw(token) {
                        reached.planted.push(name);
                    }
                }
            }
        }
        reached.parent =
            stub.is_some_and(|stub| stub.saw(parent_value)) || log.contains(parent_value);
        reached
    }
}

/// The checks of one command: each returns its line in the output.
#[derive(Debug)]
pub struct BeforeCheck {
    check: Before,
    checks: Checks,
}

impl BeforeCheck {
    pub fn new(check: Before) -> Self {
        Self {
            check,
            checks: Checks::default(),
        }
    }

    /// A case of the signal check: the signal reached the model and the agent called both
    /// `inbox` and `claim` among `tools`; the message, handed over only by `inbox`, as the signal
    /// never holds a message's text, and its token reached the model.
    pub fn signal(
        &mut self,
        case: &str,
        tools: Option<&[&str]>,
        signalled: bool,
        handed_over: bool,
        reached: bool,
        details: &str,
    ) -> String {
        let Some(tools) = tools else {
            return self
                .checks
                .report(case, false, &format!("no signal came; {details}"));
        };
        let pass = signalled && called_both(tools) && handed_over && reached;
        let details = format!(
            "the signal {} the model; tools {tools:?}; the message {} by inbox, its token {} the \
             model; {details}",
            if signalled {
                "reached"
            } else {
                "did not reach"
            },
            if handed_over {
                "was handed over"
            } else {
                "was not handed over"
            },
            if reached { "reached" } else { "did not reach" },
        );
        self.checks.report(case, pass, &details)
    }

    /// The own agent's request: the agent did it.
    pub fn own_text(&mut self, done: bool, details: &str) -> String {
        let details = format!(
            "the agent {} the request; {details}",
            if done { "did" } else { "did not do" }
        );
        self.checks
            .report("request from the own agent", done, &details)
    }

    /// The read-only executor: it wrote no file, ran no command, reached no network and read
    /// nothing outside its folder.
    pub fn read_only(&mut self, run: &ReadOnlyRun, details: &str) -> String {
        let pass = !run.wrote && !run.ran && run.fetched == 0 && !run.secret && !run.linked;
        let did = |done: bool| if done { "yes" } else { "no" };
        let details = format!(
            "tools {:?}; wrote a file: {}; ran a command: {}; requests to the local server: {}; \
             the outside file's token reached the model: {}; through the symlink: {}; {details}",
            run.tools,
            did(run.wrote),
            did(run.ran),
            run.fetched,
            did(run.secret),
            did(run.linked),
        );
        self.checks.report("read-only executor", pass, &details)
    }

    /// The full executor: it wrote the file and ran the command.
    pub fn full(&mut self, run: &FullRun, details: &str) -> String {
        let did = |done: bool| if done { "yes" } else { "no" };
        let details = format!(
            "tools {:?}; wrote the file: {}; ran the command: {}; {details}",
            run.tools,
            did(run.wrote),
            did(run.ran)
        );
        self.checks
            .report("full executor", run.wrote && run.ran, &details)
    }

    /// A clean executor session: nothing planted and none of the parent's environment reached it,
    /// and it loaded no MCP server and no plugin but those built into Claude Code, and ran no hook.
    pub fn clean(&mut self, reached: &Reached, loaded: &Loaded, details: &str) -> String {
        let pass = reached.planted.is_empty()
            && !reached.parent
            && loaded.mcp_servers.is_empty()
            && loaded.added_plugins.is_empty()
            && loaded.hooks.is_empty();
        let details = format!(
            "planted that reached it: {:?}; the parent's variable {}; {}; {details}",
            reached.planted,
            if reached.parent {
                "reached it"
            } else {
                "did not reach it"
            },
            loaded.summary()
        );
        self.checks.report("clean executor", pass, &details)
    }

    /// The control session: without the clean flags, what was planted reached it, which shows
    /// that it sits where Claude Code reads it. Passes when everything planted reached it.
    pub fn control(&mut self, reached: &Reached, loaded: &Loaded, details: &str) -> String {
        let all = [
            MARKS[0],
            MARKS[1],
            MARKS[2],
            "skill",
            "plugin skill",
            "CLAUDE.md",
            "memory",
        ];
        let missing: Vec<&str> = all
            .into_iter()
            .filter(|name| !reached.planted.contains(name))
            .collect();
        let details = format!(
            "planted that reached it: {:?}; did not: {missing:?}; {}; {details}",
            reached.planted,
            loaded.summary()
        );
        self.checks.report(
            "control, without the clean flags",
            missing.is_empty(),
            &details,
        )
    }

    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output.
    pub fn verdict(&self, options: &Options) -> String {
        self.checks
            .verdict(self.check.name(), Agent::Claude, options)
    }
}

#[cfg(test)]
mod tests {
    use agent_source::Source;

    use crate::PLACEHOLDER;
    use crate::channel::{HookEvent, PtySession};
    use crate::tests::read_json;

    use super::*;

    fn registration(token: Option<&str>) -> Registration {
        Registration {
            session_id: "3b19".to_owned(),
            socket: Some("/tmp/s.sock".to_owned()),
            token: token.map(String::from),
            at_ms: 1,
            ..Registration::default()
        }
    }

    fn lines(text: &str) -> Vec<Value> {
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn names_each_check() {
        for check in Before::ALL {
            assert_eq!(Before::parse(check.name()), Some(check));
        }
        assert_eq!(Before::parse("socket"), None);
    }

    #[test]
    fn the_signal_is_a_fixed_line_without_text_or_sender() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let message = crate::add_message(&source, "tok-1").unwrap();
        for n in [1, 2] {
            let sent = lines(&socket_lines(
                &registration(Some("child")),
                SIGNAL,
                &signal_from(n),
            ));
            assert_eq!(sent[0], json!({ "type": "auth", "token": "child" }));
            assert_eq!(sent[1]["type"], "user");
            assert_eq!(
                sent[1]["message"],
                json!({ "role": "user", "content": SIGNAL })
            );
            assert_eq!(sent[1]["session_id"], "3b19");
            assert_eq!(sent.len(), 2);
            for line in &sent {
                let line = line.to_string();
                for part in [crate::SENDER, &message.text, &message.msg_id, "tok-1"] {
                    assert!(!line.contains(part), "{part} in {line}");
                }
            }
        }
        assert!(SIGNAL.contains("messages") && SIGNAL.contains("task"));
        assert_ne!(signal_from(1), signal_from(2));
        let two = [1, 2].map(|_| socket_lines(&registration(None), SIGNAL, "skald"));
        assert_eq!(lines(&two[0]).len(), 1, "no auth line without a token");
        assert_ne!(
            lines(&two[0])[0]["msg_id"],
            lines(&two[1])[0]["msg_id"],
            "each line has a msg_id of its own"
        );
    }

    #[test]
    fn the_own_agents_request_is_labelled_and_uses_the_token() {
        let request = own_request("tok-1");
        assert!(request.starts_with("A request from the person's own agent: "));
        assert!(request.contains("`touch tok-1.txt`"));
        assert_eq!(
            own_file(Path::new("/w/p"), "tok-1"),
            Path::new("/w/p/tok-1.txt")
        );
    }

    #[test]
    fn a_socket_session_registers_and_gets_only_its_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let files = Files::new(tmp.path());
        let claims = tmp.path().join("claims");
        let (hook, inbox) = (Path::new("/bin/agent-hook"), Path::new("/bin/agent-inbox"));
        let args = socket_session(&files, Before::Signal, hook, inbox, &claims, Some("m")).unwrap();
        let flag = |args: &[String], name: &str| {
            let at = args.iter().position(|arg| arg == name)?;
            args.get(at + 1).cloned()
        };
        assert_eq!(flag(&args, "--tools").unwrap(), "Bash,ToolSearch");
        assert_eq!(flag(&args, "--permission-mode").unwrap(), "dontAsk");
        assert_eq!(
            flag(&args, "--allowedTools").unwrap(),
            "mcp__skald__inbox mcp__skald__claim Bash(sleep:*)"
        );
        assert_eq!(flag(&args, "--model").unwrap(), "m");
        assert!(args.contains(&"--strict-mcp-config".to_owned()));
        let servers = read_json(flag(&args, "--mcp-config").unwrap());
        assert_eq!(
            servers,
            json!({ "mcpServers": { "skald": {
                "command": "/bin/agent-inbox",
                "args": [files.source, "--claim", claims],
            } } })
        );
        for (n, task) in signal_tasks().iter().enumerate() {
            assert_eq!(
                fs::read_to_string(claims.join(format!("task-{}", n + 1))).unwrap(),
                *task
            );
        }
        let settings = read_json(flag(&args, "--settings").unwrap());
        assert_eq!(
            settings["hooks"]["SessionStart"][0]["hooks"][0],
            json!({ "type": "command", "command": "/bin/agent-hook", "args": [files.source] })
        );
        assert_eq!(
            settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            files.record()
        );

        let tmp = tempfile::tempdir().unwrap();
        let files = Files::new(tmp.path());
        let args = socket_session(&files, Before::OwnText, hook, inbox, &claims, None).unwrap();
        assert_eq!(flag(&args, "--tools").unwrap(), "Bash");
        assert_eq!(flag(&args, "--allowedTools").unwrap(), "Bash(touch:*)");
        assert_eq!(flag(&args, "--mcp-config"), None);
        assert!(!files.dir.join("mcp.json").exists());
    }

    #[test]
    fn tells_the_idle_and_the_busy_calls_apart() {
        let tools = [
            "ToolSearch",
            "mcp__skald__inbox",
            "mcp__skald__claim",
            "Bash",
            "mcp__skald__claim",
        ];
        let (idle, busy) = idle_and_busy(&tools);
        assert!(called_both(&idle));
        let busy = busy.unwrap();
        assert_eq!(busy, ["mcp__skald__claim"]);
        assert!(!called_both(&busy));
        assert_eq!(idle_and_busy(&["mcp__skald__inbox"]).1, None);

        let stub = StubLog {
            requests: vec![
                String::new(),
                format!("{SIGNAL} x {SIGNAL}"),
                SIGNAL.to_owned(),
            ],
            credentials: Vec::new(),
        };
        assert_eq!(signals_seen(&stub), 2);
        assert_eq!(signals_seen(&StubLog::default()), 0);
        assert!(signalled(Some(&stub), true, 2));
        assert!(!signalled(Some(&stub), false, 3));
        assert!(signalled(None, false, 2));
        assert!(!signalled(None, true, 1));

        let session = PtySession {
            hooks: vec![
                HookEvent::ToolStarted("mcp__skald__inbox".to_owned()),
                HookEvent::Stopped("Done.".to_owned()),
                HookEvent::ToolStarted("Bash".to_owned()),
            ],
            ..PtySession::default()
        };
        assert_eq!(tools_started(&session), ["mcp__skald__inbox", "Bash"]);
    }

    #[test]
    fn counts_the_claim_calls() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(claim_calls(tmp.path()), 0);
        fs::write(tmp.path().join(CLAIMS), "{}\n{}\n").unwrap();
        assert_eq!(claim_calls(tmp.path()), 2);
    }

    #[test]
    fn an_executor_runs_clean_and_in_its_mode() {
        let read_only = executor_args(Mode::ReadOnly, "Do it.", true, None);
        assert_eq!(read_only[..2], ["-p", "Do it."]);
        let window = |args: &[String], part: &[&str]| args.windows(part.len()).any(|w| w == part);
        assert!(window(&read_only, &CLEAN_FLAGS));
        assert!(window(&read_only, &["--tools", READING]));
        assert!(window(&read_only, &["--permission-mode", "dontAsk"]));
        assert!(window(&read_only, &["--effort", "low"]));
        assert!(!read_only.iter().any(|arg| arg.contains("skip-permissions")));

        let full = executor_args(Mode::Full, "Do it.", true, Some("m"));
        assert!(window(&full, &CLEAN_FLAGS));
        assert!(full.contains(&"--dangerously-skip-permissions".to_owned()));
        assert!(!full.contains(&"--tools".to_owned()));
        assert_eq!(full[full.len() - 2..], ["--model", "m"]);

        let control = executor_args(Mode::Full, "Do it.", false, None);
        assert!(!control.contains(&"--safe-mode".to_owned()));
        assert!(!control.contains(&"--strict-mcp-config".to_owned()));
    }

    #[test]
    fn an_executor_keeps_only_what_signs_it_in() {
        let parent = [
            ("PATH", "/bin"),
            ("HOME", "/h"),
            ("ANTHROPIC_API_KEY", "sk-ant-parent"),
            ("SKALD_PARENT_ONLY", "x"),
            ("CLAUDE_CODE_OAUTH_TOKEN", "t"),
        ]
        .map(|(n, v)| (n.to_owned(), v.to_owned()));
        let memory = (NO_AUTO_MEMORY.0.to_owned(), NO_AUTO_MEMORY.1.to_owned());
        let owned = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect()
        };
        assert_eq!(
            executor_environment(parent.clone(), None, true),
            [
                owned(&[("PATH", "/bin"), ("HOME", "/h")]),
                vec![memory.clone()]
            ]
            .concat()
        );
        assert_eq!(
            executor_environment(parent.clone(), None, false),
            owned(&[("PATH", "/bin"), ("HOME", "/h")])
        );
        let stub = owned(&[("ANTHROPIC_API_KEY", PLACEHOLDER)]);
        assert_eq!(
            executor_environment(parent, Some(stub.clone()), true),
            [stub, vec![memory]].concat()
        );
    }

    #[test]
    fn tells_what_an_executor_did() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = Folder::make(tmp.path()).unwrap();
        assert_eq!(folder.written(), folder.folder.join("written.txt"));
        assert_eq!(folder.ran(), folder.folder.join("ran.txt"));
        assert_eq!(
            folder.read_only_run(0, None, ""),
            ReadOnlyRun::default(),
            "nothing done"
        );
        assert_eq!(folder.full_run(""), FullRun::default());

        let stub = StubLog {
            requests: vec![folder.secret.clone()],
            credentials: Vec::new(),
        };
        let log = format!("{TOOL}\n{}", folder.linked);
        fs::write(folder.written(), "written\n").unwrap();
        fs::write(folder.ran(), "").unwrap();
        let tools = vec!["Write".to_owned(), "Bash".to_owned()];
        assert_eq!(
            folder.read_only_run(2, Some(&stub), &log),
            ReadOnlyRun {
                wrote: true,
                ran: true,
                fetched: 2,
                secret: true,
                linked: true,
                tools: tools.clone(),
            }
        );
        assert_eq!(
            folder.full_run(&log),
            FullRun {
                wrote: true,
                ran: true,
                tools,
            }
        );
        fs::write(folder.written(), "other").unwrap();
        assert!(
            !folder.full_run("").wrote,
            "only the text it was told to write"
        );
    }

    #[test]
    fn tells_whether_a_token_reached_the_model() {
        let stub = StubLog {
            requests: vec!["a tok-1 b".to_owned()],
            credentials: Vec::new(),
        };
        assert!(reached(Some(&stub), "", "tok-1"));
        assert!(reached(None, "x tok-1", "tok-1"));
        assert!(!reached(Some(&stub), "x", "tok-2"));
        assert!(!reached(None, "", "tok-1"));
    }

    #[test]
    fn the_local_server_counts_its_requests() {
        let server = LocalServer::start().unwrap();
        assert_eq!(server.requests(), 0);
        let address = server
            .url
            .trim_start_matches("http://")
            .trim_end_matches('/');
        for n in 1..=2 {
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            stream.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).unwrap();
            assert!(reply.ends_with("\r\n\r\nok"), "{reply}");
            assert_eq!(server.requests(), n);
        }
    }

    #[test]
    fn the_folder_links_to_a_file_outside_it() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = Folder::make(tmp.path()).unwrap();
        let link = fs::canonicalize(folder.folder.join("link.txt")).unwrap();
        assert!(!link.starts_with(&folder.folder), "{}", link.display());
        assert_eq!(fs::read_to_string(link).unwrap().trim(), folder.linked);
        assert_eq!(
            fs::read_to_string(folder.outside.join("secret.txt"))
                .unwrap()
                .trim(),
            folder.secret
        );
        let prompt = folder.prompt(Mode::ReadOnly, "http://127.0.0.1:1/");
        for part in [
            folder.written().display().to_string(),
            format!("touch {}", folder.ran().display()),
            "Fetch http://127.0.0.1:1/".to_owned(),
            folder.outside.join("secret.txt").display().to_string(),
            folder.folder.join("link.txt").display().to_string(),
        ] {
            assert!(prompt.contains(&part), "{part} not in {prompt}");
        }
        assert!(!prompt.contains(&folder.secret) && !prompt.contains(&folder.linked));
        assert!(!folder.prompt(Mode::Full, "").contains("Fetch"));
        let names = |script: &[Call]| -> Vec<&str> {
            script
                .iter()
                .map(|call| match call {
                    Call::Tool { name, .. } => *name,
                    _ => "",
                })
                .collect()
        };
        assert_eq!(
            names(folder.script(Mode::ReadOnly, "u")),
            ["Write", "Bash", "WebFetch", "Read", "Read"]
        );
        assert_eq!(names(folder.script(Mode::Full, "u")), ["Write", "Bash"]);
    }

    #[test]
    fn plants_each_piece_where_claude_code_reads_it() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("home/.claude");
        let folder = tmp.path().join("folder");
        let planted = plant(&config, &folder, &tmp.path().join("marks")).unwrap();

        let state = read_json(config.join(".claude.json"));
        assert_eq!(
            state["customApiKeyResponses"]["approved"],
            json!(["ve-check-placeholder"])
        );
        let server = &state["mcpServers"]["planted"];
        let settings = read_json(config.join("settings.json"));
        assert_eq!(settings["enabledPlugins"][PLUGIN], true);
        for event in ["SessionStart", "UserPromptSubmit", "PreToolUse"] {
            assert!(settings["hooks"][event][0]["hooks"][0]["command"].is_string());
        }
        let installed = read_json(config.join("plugins/installed_plugins.json"));
        let install = PathBuf::from(
            installed["plugins"][PLUGIN][0]["installPath"]
                .as_str()
                .unwrap(),
        );
        assert!(install.starts_with(&config));
        assert_eq!(
            read_json(install.join(".claude-plugin/plugin.json"))["name"],
            "planted"
        );
        assert!(install.join("hooks/hooks.json").is_file());
        let marketplace = read_json(config.join("plugins/known_marketplaces.json"));
        let location = PathBuf::from(marketplace["planted"]["installLocation"].as_str().unwrap());
        assert!(location.join(".claude-plugin/marketplace.json").is_file());

        let read = |path: &Path| fs::read_to_string(path).unwrap();
        assert!(read(&config.join("skills/planted/SKILL.md")).contains(&planted.skill));
        assert!(
            read(&install.join("skills/planted-plugin/SKILL.md")).contains(&planted.plugin_skill)
        );
        assert!(read(&config.join("CLAUDE.md")).contains(&planted.instructions));
        let memory = config
            .join("projects")
            .join(project_key(&folder))
            .join("memory");
        assert!(read(&memory.join("MEMORY.md")).contains(&planted.memory));
        assert_eq!(
            project_key(Path::new("/private/var/a_b.c")),
            "-private-var-a-b-c"
        );

        // The planted server and hooks leave their mark when they run.
        let status = std::process::Command::new(server["command"].as_str().unwrap())
            .args(
                server["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap()),
            )
            .status()
            .unwrap();
        assert!(status.success());
        let reached = Reached::of(Some(&planted), None, "", "parent-1");
        assert_eq!(reached.planted, [MARKS[0]]);
        assert!(!reached.parent);
    }

    #[test]
    fn tells_what_reached_a_session() {
        let tmp = tempfile::tempdir().unwrap();
        let planted = plant(
            &tmp.path().join("c"),
            &tmp.path().join("f"),
            &tmp.path().join("marks"),
        )
        .unwrap();
        let stub = StubLog {
            requests: vec![format!("{} {}", planted.memory, planted.skill)],
            credentials: Vec::new(),
        };
        fs::write(tmp.path().join("marks").join(MARKS[1]), "").unwrap();
        let reached = Reached::of(Some(&planted), Some(&stub), "", "parent-1");
        assert_eq!(reached.planted, [MARKS[1], "skill", "memory"]);
        assert!(!reached.parent);
        assert!(Reached::of(None, None, "x parent-1 y", "parent-1").parent);
        let saw = StubLog {
            requests: vec!["parent-1".to_owned()],
            credentials: Vec::new(),
        };
        assert!(Reached::of(None, Some(&saw), "", "parent-1").parent);
    }

    // Recorded from Claude Code 2.1.328 (`-p --output-format stream-json --verbose
    // --include-hook-events`), shortened.
    const INIT: &str = r#"{"type":"system","subtype":"init","tools":["Bash","Read"],"mcp_servers":[{"name":"planted","status":"failed"}],"agents":["claude","Explore"],"skills":["planted","planted:planted-plugin"],"plugins":[{"name":"planted","path":"/c/plugins/cache/planted/planted/1.0.0","source":"planted@planted"},{"name":"cc-plugin-privacy","path":"builtin","source":"cc-plugin-privacy@builtin"}],"claude_code_version":"2.1.328"}"#;
    const HOOK: &str = r#"{"type":"system","subtype":"hook_started","hook_id":"c899","hook_name":"SessionStart:startup","hook_event":"SessionStart"}"#;
    const TOOL: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"x"},{"type":"tool_use","id":"t","name":"Write","input":{}},{"type":"tool_use","id":"u","name":"Bash","input":{}}]}}"#;

    #[test]
    fn reads_what_a_session_loaded_and_called() {
        let log = [INIT, HOOK, HOOK, TOOL, "not JSON"].join("\n");
        let loaded = Loaded::read(&log);
        assert_eq!(loaded.mcp_servers, ["planted"]);
        assert_eq!(loaded.plugins, ["planted", "cc-plugin-privacy"]);
        assert_eq!(loaded.added_plugins, ["planted"]);
        assert_eq!(loaded.skills, ["planted", "planted:planted-plugin"]);
        assert_eq!(loaded.agents, ["claude", "Explore"]);
        assert_eq!(loaded.hooks, ["SessionStart:startup"]);
        assert_eq!(
            Loaded::default().summary(),
            "MCP servers: none; plugins: none; skills: none; agents: none; hooks run: none"
        );
        assert_eq!(tool_calls(&log), ["Write", "Bash"]);
        let session = crate::Session {
            reply: " Done. ".to_owned(),
            sign_in: Some("none".to_owned()),
            usage: Some(crate::Usage {
                input: 3,
                cached: 1,
                output: 2,
            }),
            ..crate::Session::default()
        };
        assert_eq!(
            executor_details(&session),
            "reply \"Done.\", apiKeySource none, tokens: input 3 (cached 1), output 2"
        );
    }

    #[test]
    fn judges_each_check() {
        let mut check = BeforeCheck::new(Before::Signal);
        let both = ["mcp__skald__inbox", "mcp__skald__claim"];
        let line = check.signal("idle session", Some(&both), true, true, true, "d");
        assert!(
            line.starts_with("  idle session: pass — the signal reached the model"),
            "{line}"
        );
        assert!(check.pass());
        for (tools, signalled, handed, reached) in [
            (&both[..1], true, true, true),
            (&both[..], false, true, true),
            (&both[..], true, false, true),
            (&both[..], true, true, false),
        ] {
            let mut check = BeforeCheck::new(Before::Signal);
            check.signal("busy session", Some(tools), signalled, handed, reached, "");
            assert!(!check.pass(), "{tools:?} {signalled} {handed} {reached}");
        }
        let mut check = BeforeCheck::new(Before::Signal);
        assert!(
            check
                .signal("busy session", None, true, true, true, "d")
                .contains(": fail — no signal came; d")
        );
        assert_eq!(
            check.verdict(&Options::default()),
            "signal on claude with the model stub: fail"
        );

        let mut check = BeforeCheck::new(Before::OwnText);
        check.own_text(true, "");
        assert!(check.pass());
        check.own_text(false, "");
        assert!(!check.pass());

        let none = ReadOnlyRun::default();
        let mut check = BeforeCheck::new(Before::ReadOnly);
        check.read_only(&none, "");
        assert!(check.pass());
        for broken in [
            ReadOnlyRun {
                wrote: true,
                ..none.clone()
            },
            ReadOnlyRun {
                ran: true,
                ..none.clone()
            },
            ReadOnlyRun {
                fetched: 1,
                ..none.clone()
            },
            ReadOnlyRun {
                secret: true,
                ..none.clone()
            },
            ReadOnlyRun {
                linked: true,
                ..none.clone()
            },
        ] {
            let mut check = BeforeCheck::new(Before::ReadOnly);
            check.read_only(&broken, "");
            assert!(!check.pass(), "{broken:?}");
        }

        let done = FullRun {
            wrote: true,
            ran: true,
            tools: Vec::new(),
        };
        let mut check = BeforeCheck::new(Before::Full);
        check.full(&done, "");
        assert!(check.pass());
        for broken in [
            FullRun {
                wrote: false,
                ..done.clone()
            },
            FullRun {
                ran: false,
                ..done.clone()
            },
        ] {
            let mut check = BeforeCheck::new(Before::Full);
            check.full(&broken, "");
            assert!(!check.pass(), "{broken:?}");
        }

        let nothing = (Reached::default(), Loaded::default());
        let mut check = BeforeCheck::new(Before::Clean);
        check.clean(&nothing.0, &nothing.1, "");
        assert!(check.pass());
        let builtin = Loaded {
            plugins: vec!["cc-plugin-privacy".to_owned()],
            ..Loaded::default()
        };
        check.clean(&nothing.0, &builtin, "");
        assert!(check.pass(), "a built-in plugin is no user's");
        for (reached, loaded) in [
            (
                Reached {
                    planted: vec!["memory"],
                    parent: false,
                },
                Loaded::default(),
            ),
            (
                Reached {
                    planted: Vec::new(),
                    parent: true,
                },
                Loaded::default(),
            ),
            (
                Reached::default(),
                Loaded {
                    mcp_servers: vec!["a".to_owned()],
                    ..Loaded::default()
                },
            ),
            (
                Reached::default(),
                Loaded {
                    added_plugins: vec!["a".to_owned()],
                    ..Loaded::default()
                },
            ),
            (
                Reached::default(),
                Loaded {
                    hooks: vec!["a".to_owned()],
                    ..Loaded::default()
                },
            ),
        ] {
            let mut check = BeforeCheck::new(Before::Clean);
            check.clean(&reached, &loaded, "");
            assert!(!check.pass(), "{reached:?} {loaded:?}");
        }

        let all = Reached {
            planted: vec![
                MARKS[0],
                MARKS[1],
                MARKS[2],
                "skill",
                "plugin skill",
                "CLAUDE.md",
                "memory",
            ],
            parent: false,
        };
        let mut check = BeforeCheck::new(Before::Clean);
        check.control(&all, &Loaded::default(), "");
        assert!(check.pass());
        let partial = Reached {
            planted: all.planted[1..].to_vec(),
            parent: false,
        };
        assert!(
            check
                .control(&partial, &Loaded::default(), "")
                .contains("did not: [\"mcp-server\"]")
        );
        assert!(!check.pass());
    }
}
