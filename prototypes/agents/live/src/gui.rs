//! The GUI live checks: a path on a Claude Code or Codex session in a desktop app or an IDE
//! extension that a developer or reviewer drives. [`prepare`] makes the temporary project with the
//! surface's config and the message source; [`add`] adds the check's message during a busy
//! session; [`result`] takes the token they read from the agent's reply. None shows the token
//! before the reported one.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use agent_source::{Message, Source, State};
use serde_json::{Value, json};

use crate::hook::Hook;
use crate::wake::{self, Wake};
use crate::{
    Agent, Kind, QUICK, SENDER, SERVER, add_message, channel, codex_hook, handed_over,
    handover_delay, inbox_tool, mcp_config, pretty, queue, read, server_tools, shell_quote,
    verdict,
};

/// The file, in a check's directory, that holds what [`result`] needs, the token too.
pub const CHECK: &str = "check.json";

/// An app or IDE extension that runs Claude Code or Codex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Claude Code in the Claude desktop app's Code tab.
    ClaudeDesktop,
    /// Claude Code for VS Code.
    ClaudeVsCode,
    /// The Codex desktop app, `ChatGPT.app`.
    CodexDesktop,
    /// Codex for VS Code.
    CodexVsCode,
}

impl Surface {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "claude-desktop" => Some(Self::ClaudeDesktop),
            "claude-vscode" => Some(Self::ClaudeVsCode),
            "codex-desktop" => Some(Self::CodexDesktop),
            "codex-vscode" => Some(Self::CodexVsCode),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ClaudeDesktop => "claude-desktop",
            Self::ClaudeVsCode => "claude-vscode",
            Self::CodexDesktop => "codex-desktop",
            Self::CodexVsCode => "codex-vscode",
        }
    }

    pub fn agent(self) -> Agent {
        match self {
            Self::ClaudeDesktop | Self::ClaudeVsCode => Agent::Claude,
            Self::CodexDesktop | Self::CodexVsCode => Agent::Codex,
        }
    }

    /// The surface's version line, read from the app's or extension's metadata in `installed`:
    /// the app's or extension's version and, but for the Claude app, which downloads Claude Code
    /// when it starts, the version of the agent it runs; [`result`] adds the Claude Code version
    /// a session ran.
    pub fn version(self, installed: &Installed) -> io::Result<String> {
        let version = match self {
            Self::ClaudeDesktop => {
                format!("Claude {}", bundle_version(&installed.app("Claude.app"))?)
            }
            // The extension's version is that of the Claude Code it bundles.
            Self::ClaudeVsCode => {
                let (version, _) = installed.extension("anthropic.claude-code")?;
                format!("Claude Code for VS Code {version}")
            }
            Self::CodexDesktop => {
                let app = codex_app(installed)?;
                let codex = codex_package(&self.codex_dir(installed)?)?;
                format!("ChatGPT {}, codex {codex}", bundle_version(&app)?)
            }
            Self::CodexVsCode => {
                let (version, _) = installed.extension("openai.chatgpt")?;
                let codex = codex_package(&self.codex_dir(installed)?)?;
                format!("Codex for VS Code {version}, codex {codex}")
            }
        };
        Ok(format!("{}: {version}", self.name()))
    }

    /// The Codex CLI that a Codex surface bundles, in the directory of its `codex-package.json`:
    /// `bin/codex` in the app, `codex` in the extension.
    pub fn codex(self, installed: &Installed) -> io::Result<PathBuf> {
        let dir = self.codex_dir(installed)?;
        ["bin/codex", "codex"]
            .map(|name| dir.join(name))
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| invalid(format!("{} has no bin/codex or codex", dir.display())))
    }

    /// The directory of the Codex CLI that a Codex surface bundles.
    fn codex_dir(self, installed: &Installed) -> io::Result<PathBuf> {
        match self {
            Self::CodexDesktop => Ok(codex_app(installed)?.join("Contents/Resources/codex-cli")),
            Self::CodexVsCode => {
                let (_, dir) = installed.extension("openai.chatgpt")?;
                only_dir(&dir.join("bin"))
            }
            Self::ClaudeDesktop | Self::ClaudeVsCode => {
                Err(invalid(format!("{} bundles no Codex CLI", self.name())))
            }
        }
    }
}

/// The Codex desktop app, once its bundle ID is the Codex app's.
fn codex_app(installed: &Installed) -> io::Result<PathBuf> {
    let app = installed.app("ChatGPT.app");
    let id = plist_string(&app, "CFBundleIdentifier")?;
    if id != CODEX_APP {
        return Err(invalid(format!(
            "{} is {id}, not the Codex app {CODEX_APP}",
            app.display()
        )));
    }
    Ok(app)
}

/// The Codex desktop app's bundle ID.
const CODEX_APP: &str = "com.openai.codex";

/// Where the surfaces are installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The apps' directory, such as `/Applications`.
    pub applications: PathBuf,
    /// The user's home, with VS Code's extensions.
    pub home: PathBuf,
}

impl Installed {
    fn app(&self, name: &str) -> PathBuf {
        self.applications.join(name)
    }

    /// The version and directory of the VS Code extension `id`, as VS Code's list of installed
    /// extensions gives them.
    fn extension(&self, id: &str) -> io::Result<(String, PathBuf)> {
        let extensions = self.home.join(".vscode/extensions");
        let list = read_json(&extensions.join("extensions.json"))?;
        list.as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["identifier"]["id"] == id)
            .and_then(|entry| {
                let version = entry["version"].as_str()?;
                let dir = entry["relativeLocation"].as_str()?;
                Some((version.to_owned(), extensions.join(dir)))
            })
            .ok_or_else(|| invalid(format!("the VS Code extension {id} is not installed")))
    }
}

/// The app's version, `CFBundleShortVersionString`.
fn bundle_version(app: &Path) -> io::Result<String> {
    plist_string(app, "CFBundleShortVersionString")
}

/// The string `key` of the app's XML `Info.plist`.
fn plist_string(app: &Path, key: &str) -> io::Result<String> {
    let path = app.join("Contents/Info.plist");
    let plist = fs::read_to_string(&path)?;
    plist
        .split_once(&format!("<key>{key}</key>"))
        .and_then(|(_, rest)| rest.trim_start().strip_prefix("<string>"))
        .and_then(|rest| rest.split_once("</string>"))
        .map(|(value, _)| value.to_owned())
        .ok_or_else(|| invalid(format!("{} has no {key}", path.display())))
}

/// The version of the Codex CLI an app or extension bundles in `dir`, from its
/// `codex-package.json`.
fn codex_package(dir: &Path) -> io::Result<String> {
    let path = dir.join("codex-package.json");
    read_json(&path)?["version"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| invalid(format!("{} has no version", path.display())))
}

/// The only directory in `dir`, such as an extension's platform in its `bin`.
fn only_dir(dir: &Path) -> io::Result<PathBuf> {
    let dirs: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.is_dir())
        .collect();
    match dirs.as_slice() {
        [only] => Ok(only.clone()),
        _ => Err(invalid(format!(
            "{} holds {} directories, not one",
            dir.display(),
            dirs.len()
        ))),
    }
}

/// The Claude Code versions that sessions in `project` ran, in the order their transcripts in
/// `home`'s `.claude/projects/` give them: the `version` of each entry. None without a transcript.
pub fn session_versions(home: &Path, project: &Path) -> io::Result<Vec<String>> {
    // Claude Code names a project's directory after its real path, with `-` for each character
    // but letters and digits.
    let key: String = fs::canonicalize(project)?
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let dir = home.join(".claude/projects").join(key);
    let mut versions = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(versions);
    };
    let mut transcripts: Vec<PathBuf> = entries
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    transcripts.sort();
    for transcript in transcripts {
        for line in fs::read_to_string(transcript)?.lines() {
            let version = serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|entry| entry["version"].as_str().map(String::from));
            if let Some(version) = version.filter(|version| !versions.contains(version)) {
                versions.push(version);
            }
        }
    }
    Ok(versions)
}

fn read_json(path: &Path) -> io::Result<Value> {
    serde_json::from_str(&fs::read_to_string(path)?).map_err(io::Error::from)
}

fn invalid(message: String) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message)
}

/// A path under a GUI check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiPath {
    Inbox,
    Channel,
    /// A hook: `PostToolUse` and `Stop` on each surface, `UserPromptSubmit` on Codex's.
    Hook(Hook),
    /// `codex queue` into the session's thread, on Codex's surfaces.
    Queue,
    /// A path that wakes an idle session, on Claude Code's surfaces, with the read confirmations.
    Wake(Wake),
    /// `codex queue`, on Codex's surfaces, with the read confirmations.
    Read,
}

impl GuiPath {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "inbox" => Some(Self::Inbox),
            "channel" => Some(Self::Channel),
            "queue" => Some(Self::Queue),
            "read" => Some(Self::Read),
            other => Hook::parse(other)
                .map(Self::Hook)
                .or_else(|| Wake::parse(other).map(Self::Wake)),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Channel => "channel",
            Self::Queue => "queue",
            Self::Read => "read",
            Self::Hook(hook) => hook.name(),
            Self::Wake(wake) => wake.name(),
        }
    }

    /// Whether the GUI check runs on `surface`: `inbox` on each; Channels on Claude Code's; the
    /// hooks where the CLI checks run them; `codex queue` on Codex's.
    pub fn runs_on(self, surface: Surface) -> bool {
        match self {
            Self::Inbox => true,
            Self::Channel => surface.agent() == Agent::Claude,
            Self::Hook(hook) => hook.runs_on(surface.agent()),
            Self::Queue | Self::Read => surface.agent() == Agent::Codex,
            Self::Wake(_) => surface.agent() == Agent::Claude,
        }
    }

    /// Whether the GUI check has a busy case on `surface`: `inbox` on Claude Code's; the hooks the
    /// CLI checks run busy, and `codex queue`.
    pub fn runs_busy_on(self, surface: Surface) -> bool {
        match self {
            Self::Inbox => surface.agent() == Agent::Claude,
            Self::Channel => false,
            Self::Hook(hook) => hook.checks_busy() && self.runs_on(surface),
            Self::Queue | Self::Read | Self::Wake(_) => self.runs_on(surface),
        }
    }

    /// Whether the message comes only once the session has a thread: `codex queue` hands it to
    /// the session's thread, which its first prompt makes.
    pub fn needs_thread(self) -> bool {
        matches!(self, Self::Queue | Self::Read)
    }

    /// Whether the message comes only after the session's first turn, by `gui-add` or, for the
    /// socket, `gui-socket`: the paths that wake an idle session.
    pub fn wakes(self) -> bool {
        matches!(self, Self::Wake(_))
    }

    /// Whether the message carries a secret for its acknowledgement, and the read confirmations
    /// are reported.
    pub fn confirms(self) -> bool {
        matches!(self, Self::Wake(_) | Self::Read)
    }

    /// The prompt the developer or reviewer types: in the busy case, with a shell command during
    /// which the message comes.
    pub fn prompt(self, busy: bool) -> String {
        let kind = if busy { Kind::Busy } else { Kind::Idle };
        match self {
            Self::Inbox => kind.prompt(),
            Self::Hook(hook) => hook.prompt(kind),
            // Only a reply, as for the `Stop` hook: a loaded channel pushes the message itself.
            Self::Channel => Hook::Stop.prompt(Kind::Idle),
            // The loop's prompt schedules the polling.
            Self::Wake(Wake::Loop) => Wake::Loop.prompt(kind).unwrap_or_default(),
            // The first turn, which makes the thread or starts the session; the message's own
            // turn asks for the token.
            Self::Queue | Self::Read | Self::Wake(_) => queue::prompt(kind),
        }
    }

    /// Adds the check's message with `token` to `source`: the text of the path's CLI check.
    fn add_message(self, source: &Source, token: &str) -> io::Result<Message> {
        match self {
            Self::Inbox => add_message(source, token),
            _ if self.confirms() => source.add_with_ack(SENDER, &channel::text(token)),
            _ => channel::add_message(source, token),
        }
    }
}

/// The binaries a project's config runs, next to `live-check`, and what else it needs from the
/// user's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binaries {
    pub inbox: PathBuf,
    pub channel: PathBuf,
    pub hook: PathBuf,
    /// The MCP servers of the user's Codex config, which a Codex project turns off.
    pub user_mcp_servers: Vec<String>,
    /// The plugins and MCP servers of the user's Claude Code config, which a Claude Code project
    /// turns off.
    pub user_claude: UserClaude,
}

/// The names of the plugins and MCP servers of the user's Claude Code config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserClaude {
    pub plugins: Vec<String>,
    pub mcp_servers: Vec<String>,
}

impl UserClaude {
    /// Reads only the names: the plugins that the user's `settings`, `~/.claude/settings.json`,
    /// turn on in `enabledPlugins`, and the servers of `mcpServers` in `config`, `~/.claude.json`.
    pub fn read(settings: &Value, config: &Value) -> Self {
        let names = |value: &Value, on: fn(&Value) -> bool| -> Vec<String> {
            value.as_object().map_or_else(Vec::new, |map| {
                map.iter()
                    .filter(|(_, value)| on(value))
                    .map(|(name, _)| name.clone())
                    .collect()
            })
        };
        Self {
            plugins: names(&settings["enabledPlugins"], |on| on == &Value::Bool(true)),
            mcp_servers: names(&config["mcpServers"], |_| true),
        }
    }

    /// Turns them off in a Claude Code project's `settings`: each plugin in `enabledPlugins`; the
    /// claude.ai connectors, whose names only the account knows, by `disableClaudeAiConnectors`;
    /// and each server's tools, but those of a server named as the check's, by a deny rule on the
    /// name Claude Code gives its tools. Claude Code still starts the servers; the session gets
    /// none of their tools.
    fn turn_off(&self, settings: &mut Value) {
        settings["disableClaudeAiConnectors"] = true.into();
        if !self.plugins.is_empty() {
            let off: serde_json::Map<String, Value> = self
                .plugins
                .iter()
                .map(|plugin| (plugin.clone(), false.into()))
                .collect();
            settings["enabledPlugins"] = off.into();
        }
        let deny: Vec<String> = self
            .mcp_servers
            .iter()
            .map(|name| tools_of(name))
            .filter(|name| name != SERVER)
            .map(|name| format!("mcp__{name}"))
            .collect();
        if !deny.is_empty() {
            settings["permissions"]["deny"] = deny.into();
        }
    }
}

/// The name of the MCP server `name` in its tools' names, `mcp__<name>__<tool>`: Claude Code
/// 2.1.322 writes each character but a letter, a digit, `_` and `-` as `_`.
fn tools_of(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl Binaries {
    /// The shell form of the hook handler's command on `agent`'s `source`: both CLIs run it
    /// through the shell.
    fn hook_command(&self, agent: Agent, source: &Path) -> String {
        let source = source.to_string_lossy();
        let mut args = Vec::new();
        if agent == Agent::Codex {
            args.push("--codex");
        }
        args.push(&source);
        self.hook_with(&args)
    }

    /// The shell form of the hook handler's command with `args`.
    fn hook_with(&self, args: &[&str]) -> String {
        std::iter::once(self.hook.to_string_lossy().as_ref())
            .chain(args.iter().copied())
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The model of Codex's sessions: the API key of the test machine has no access to Codex's default
/// model.
pub const CODEX_MODEL: &str = "gpt-5.5";

/// The rule that allows the busy case's shell command.
const BUSY_SHELL: &str = "Bash(sleep:*)";

/// Claude Code's lowest effort: `maxEffortLevel` caps the effort the surface would pick, and the
/// lowest across settings files wins.
const LOW: &str = "low";

/// Writes the config that attaches `path`, on the source in `source`, to `surface`'s sessions in
/// `project`. Only files in `project` change the surface's config: the surfaces take no flags.
pub fn write_config(
    project: &Path,
    surface: Surface,
    path: GuiPath,
    binaries: &Binaries,
    source: &Path,
) -> io::Result<()> {
    if !path.runs_on(surface) {
        return Err(invalid(format!(
            "{} does not run on {}",
            path.name(),
            surface.name()
        )));
    }
    fs::create_dir_all(project)?;
    match surface.agent() {
        Agent::Claude => {
            let server = |command: &Path| mcp_config(command, source);
            let mut settings = match path {
                GuiPath::Inbox => {
                    fs::write(project.join(".mcp.json"), pretty(&server(&binaries.inbox)))?;
                    // Without them, Claude Code asks a person to approve the server and each call.
                    json!({
                        "enabledMcpjsonServers": [SERVER],
                        "permissions": { "allow": [inbox_tool(), BUSY_SHELL] },
                        "maxEffortLevel": LOW,
                    })
                }
                GuiPath::Channel => {
                    fs::write(
                        project.join(".mcp.json"),
                        pretty(&server(&binaries.channel)),
                    )?;
                    json!({ "enabledMcpjsonServers": [SERVER], "maxEffortLevel": LOW })
                }
                GuiPath::Hook(hook) => {
                    let command = binaries.hook_command(Agent::Claude, source);
                    let handlers =
                        json!([{ "hooks": [{ "type": "command", "command": command }] }]);
                    json!({
                        "hooks": { hook.event(): handlers },
                        // Without it, Claude Code asks a person to approve the commands.
                        "permissions": { "allow": [format!("Bash({QUICK})"), BUSY_SHELL] },
                        "maxEffortLevel": LOW,
                    })
                }
                GuiPath::Wake(wake) => {
                    fs::write(project.join(".mcp.json"), pretty(&server(&binaries.inbox)))?;
                    if wake == Wake::Mod {
                        wake::write_mod_config(project, &binaries.hook, source)?;
                    }
                    let handler = |args: &[&str]| json!({ "type": "command", "command": binaries.hook_with(args) });
                    let confirm = handler(&["--confirm", &source.to_string_lossy()]);
                    let mut allow = server_tools();
                    allow.push(BUSY_SHELL.to_owned());
                    json!({
                        "hooks": wake::hooks(wake, &handler, source, &[("Stop", confirm)]),
                        "enabledMcpjsonServers": [SERVER],
                        "permissions": { "allow": allow },
                        "maxEffortLevel": LOW,
                    })
                }
                GuiPath::Queue | GuiPath::Read => unreachable!("checked above"),
            };
            binaries.user_claude.turn_off(&mut settings);
            fs::create_dir_all(project.join(".claude"))?;
            fs::write(
                project.join(".claude/settings.local.json"),
                pretty(&settings),
            )
        }
        Agent::Codex => {
            // TOML; a JSON string or array of strings is valid TOML.
            let mut config = format!(
                "model = {}\nmodel_reasoning_effort = \"low\"\n",
                json!(CODEX_MODEL)
            );
            if path == GuiPath::Read {
                // A dotted key, before any table: it runs once a person trusts it.
                let source = source.to_string_lossy();
                let command = binaries.hook_with(&["--confirm", "--codex", &source]);
                config.push_str(&format!("{}\n", codex_hook("Stop", &command)));
            }
            match path {
                GuiPath::Inbox | GuiPath::Read => config.push_str(&format!(
                    "\n[mcp_servers.{SERVER}]\ncommand = {}\nargs = {}\n\
                     # Without it, Codex asks a person to approve each call.\n\
                     default_tools_approval_mode = \"approve\"\n",
                    json!(binaries.inbox),
                    json!([source]),
                )),
                // A dotted key, as `-c` takes it: it runs once a person trusts it in Review hooks.
                GuiPath::Hook(hook) => {
                    let command = binaries.hook_command(Agent::Codex, source);
                    config.push_str(&format!("{}\n", codex_hook(hook.event(), &command)));
                }
                // The message comes from outside the session.
                GuiPath::Queue => {}
                GuiPath::Channel | GuiPath::Wake(_) => unreachable!("checked above"),
            }
            // The session gets no tools of the user's MCP servers and plugins: one of the servers
            // asked macOS for automation access in Codex for VS Code. That extension still starts
            // the servers to list their status, which ignores `enabled = false`.
            config.push_str("\n[features]\nplugins = false\n");
            for name in &binaries.user_mcp_servers {
                if name != SERVER {
                    config.push_str(&format!(
                        "\n[mcp_servers.{}]\nenabled = false\n",
                        json!(name)
                    ));
                }
            }
            fs::create_dir_all(project.join(".codex"))?;
            fs::write(project.join(".codex/config.toml"), config)
        }
    }
}

/// A prepared GUI check: what the developer or reviewer needs to drive the surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The directory to open in the surface.
    pub project: PathBuf,
    pub prompt: String,
}

/// Prepares the GUI check of `path` on `surface`, whose version line is `version`, in `work`: the
/// project with the surface's config, the source with a message that holds `token`, and the
/// [`CHECK`] file for [`add`] and [`result`]. In the `busy` case, and for a path that
/// [needs the thread](GuiPath::needs_thread), the message waits for [`add`].
pub fn prepare(
    work: &Path,
    surface: Surface,
    path: GuiPath,
    binaries: &Binaries,
    version: &str,
    token: &str,
    busy: bool,
) -> io::Result<Prepared> {
    let project = work.join("project");
    let source = work.join("source");
    write_config(&project, surface, path, binaries, &source)?;
    fs::create_dir_all(&source)?;
    let check = json!({
        "surface": surface.name(),
        "path": path.name(),
        "version": version,
        "token": token,
        "busy": busy,
    });
    fs::write(work.join(CHECK), pretty(&check))?;
    if !busy && !path.needs_thread() && !path.wakes() {
        add(work)?;
    }
    Ok(Prepared {
        project,
        prompt: path.prompt(busy),
    })
}

/// Writes the settings of the VS Code profile, `work/vscode`, that a check of `path` on Claude Code
/// for VS Code needs, and returns the profile: for the mod, Claude Code's variable that loads the
/// plugin directory `plugin`; for a `bypass` session, the extension's bypass mode. `None` for any
/// other check, which needs no settings of its own.
pub fn vscode_profile(
    work: &Path,
    surface: Surface,
    path: GuiPath,
    plugin: &Path,
    bypass: bool,
) -> io::Result<Option<PathBuf>> {
    let loads_mod = path == GuiPath::Wake(Wake::Mod);
    if surface != Surface::ClaudeVsCode || !(loads_mod || bypass) {
        return Ok(None);
    }
    let mut settings = serde_json::Map::new();
    if loads_mod {
        settings.insert(
            "claudeCode.environmentVariables".to_owned(),
            json!([{ "name": "CLAUDE_CODE_PLUGIN_DIRS", "value": plugin }]),
        );
    }
    if bypass {
        settings.insert(
            "claudeCode.allowDangerouslySkipPermissions".to_owned(),
            true.into(),
        );
        settings.insert(
            "claudeCode.initialPermissionMode".to_owned(),
            "bypassPermissions".into(),
        );
    }
    let profile = work.join("vscode");
    fs::create_dir_all(profile.join("User"))?;
    fs::write(
        profile.join("User/settings.json"),
        pretty(&Value::Object(settings)),
    )?;
    Ok(Some(profile))
}

/// The [`CHECK`] file of the check prepared in `work`, and its string `key`.
fn read_check(work: &Path) -> io::Result<(Value, impl Fn(&str) -> io::Result<String>)> {
    let check = read_json(&work.join(CHECK))?;
    let copy = check.clone();
    let text = move |key: &str| {
        copy[key]
            .as_str()
            .map(String::from)
            .ok_or_else(|| invalid(format!("{CHECK} has no {key}")))
    };
    Ok((check, text))
}

/// Adds the message of the check prepared in `work`, with its token, once.
pub fn add(work: &Path) -> io::Result<Message> {
    let (mut check, text) = read_check(work)?;
    if check.get("msg_id").is_some() {
        return Err(invalid("the check's message is already added".to_owned()));
    }
    let path = GuiPath::parse(&text("path")?)
        .ok_or_else(|| invalid(format!("{CHECK} has no known path")))?;
    let message = path.add_message(&Source::open(work.join("source"))?, &text("token")?)?;
    check["msg_id"] = message.msg_id.clone().into();
    fs::write(work.join(CHECK), pretty(&check))?;
    Ok(message)
}

/// A prepared check of a path that [needs the session's thread](GuiPath::needs_thread).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub surface: Surface,
    /// The project, where the session runs.
    pub project: PathBuf,
    /// The prompt that started the session's thread.
    pub prompt: String,
}

/// The check prepared in `work`, if its path needs the session's thread.
pub fn pending(work: &Path) -> io::Result<Pending> {
    let (check, text) = read_check(work)?;
    let path = GuiPath::parse(&text("path")?)
        .filter(|path| path.needs_thread())
        .ok_or_else(|| invalid(format!("{CHECK} is not of a path that needs the thread")))?;
    let surface = Surface::parse(&text("surface")?)
        .ok_or_else(|| invalid(format!("{CHECK} has no known surface")))?;
    Ok(Pending {
        surface,
        project: work.join("project"),
        prompt: path.prompt(check["busy"] == true),
    })
}

/// Records `thread`, the session's thread that `gui-queue` found, in the check prepared in `work`,
/// for [`result`] to find the session's rollout.
pub fn set_thread(work: &Path, thread: &str) -> io::Result<()> {
    let (mut check, _) = read_check(work)?;
    check["thread"] = thread.into();
    fs::write(work.join(CHECK), pretty(&check))
}

/// The message of the check prepared in `work`, if it was added: its source and `msg_id`.
pub fn added(work: &Path) -> io::Result<Option<(Source, String)>> {
    let (check, _) = read_check(work)?;
    let Some(msg_id) = check["msg_id"].as_str() else {
        return Ok(None);
    };
    Ok(Some((
        Source::open(work.join("source"))?,
        msg_id.to_owned(),
    )))
}

/// The session's thread in a `thread/list` result, newest first: the newest whose first message is
/// `prompt`.
pub fn session_thread(list: &Value, prompt: &str) -> Option<String> {
    list["data"]
        .as_array()?
        .iter()
        .find(|thread| thread["preview"] == prompt)
        .and_then(|thread| thread["id"].as_str().map(String::from))
}

/// The outcome of a GUI check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The surface's version line, as [`prepare`] read it.
    pub version: String,
    /// The check's line: the reported token and the message's state.
    pub details: String,
    /// For a path with the read confirmations, the line that tells them.
    pub confirmations: Option<String>,
    /// The last line: the verdict.
    pub verdict: String,
    pub pass: bool,
}

/// The outcome of the GUI check prepared in `work`, given the token the developer or reviewer read
/// from the agent's reply: it passes when the token matches and the message is handed over. On
/// Claude Code's surfaces the version line adds the Claude Code version the session ran, from its
/// transcript in `home`.
pub fn result(work: &Path, reported: &str, home: &Path) -> io::Result<Outcome> {
    let (check, text) = read_check(work)?;
    let token = text("token")?;
    let surface = Surface::parse(&text("surface")?)
        .ok_or_else(|| invalid(format!("{CHECK} has no known surface")))?;
    let source = Source::open(work.join("source"))?;
    let message = match check["msg_id"].as_str() {
        Some(msg_id) => Some((msg_id.to_owned(), source.state(msg_id)?)),
        None => None,
    };
    let matches = reported.trim() == token;
    let pass = matches
        && message
            .as_ref()
            .is_some_and(|(_, state)| handed_over(*state));
    let reported = if matches {
        "matches".to_owned()
    } else {
        format!("differs from the check token {token}")
    };
    let message = match message {
        None => "never added".to_owned(),
        Some((msg_id, state @ Some(State::HandedOver { .. }))) => {
            let mut text = "handed over".to_owned();
            if check["busy"] == true {
                let added_at_ms = source.handed(&msg_id)?.added_at_ms;
                let delay = handover_delay(state, added_at_ms);
                text.push_str(&format!(" {delay} after it was added"));
            }
            text
        }
        Some((_, other)) => format!("not handed over (state {other:?})"),
    };
    let confirmations = match (GuiPath::parse(&text("path")?), check["msg_id"].as_str()) {
        (Some(path), Some(msg_id)) if path.confirms() => {
            Some(confirmations(&source, msg_id, surface, &check, home)?)
        }
        _ => None,
    };
    let mut version = text("version")?;
    if surface.agent() == Agent::Claude {
        let ran = session_versions(home, &work.join("project"))?;
        if ran.is_empty() {
            version.push_str(", no session transcript");
        } else {
            version.push_str(&format!(
                ", the session ran Claude Code {}",
                ran.join(" and ")
            ));
        }
    }
    Ok(Outcome {
        version,
        details: format!("  the reported token {reported}; the message is {message}"),
        confirmations,
        verdict: format!(
            "{} on {} with the real model: {}",
            text("path")?,
            surface.name(),
            verdict(pass)
        ),
        pass,
    })
}

/// The deep link that opens the session of the check prepared in `work` on its surface, as an OS
/// notification would carry it: the session that its `SessionStart` hook registered or the thread
/// that `gui-queue` found. The Claude desktop app's link takes the app's own ID of the session,
/// which its record in `home` maps to Claude Code's. Claude Code for VS Code's link also carries a
/// prompt, which the extension puts in the prompt box without sending: the `inbox` check's, never
/// the message, so the message comes through `inbox`. Codex for VS Code has none.
pub fn link(work: &Path, home: &Path) -> io::Result<String> {
    let (check, text) = read_check(work)?;
    let surface = Surface::parse(&text("surface")?)
        .ok_or_else(|| invalid(format!("{CHECK} has no known surface")))?;
    let session = match surface.agent() {
        Agent::Claude => Source::open(work.join("source"))?
            .registration()?
            .map(|registration| registration.session_id),
        Agent::Codex => check["thread"].as_str().map(String::from),
    }
    .ok_or_else(|| invalid("the check has no session yet".to_owned()))?;
    Ok(match surface {
        Surface::ClaudeDesktop => {
            let session = desktop_session(home, &session)?
                .ok_or_else(|| invalid("the Claude app has no record of the session".to_owned()))?;
            format!("claude://code/continue?session={}", encode(&session))
        }
        Surface::ClaudeVsCode => format!(
            "vscode://anthropic.claude-code/open?session={}&prompt={}",
            encode(&session),
            encode(&Kind::Idle.prompt())
        ),
        Surface::CodexDesktop => format!("codex://threads/{}", encode(&session)),
        Surface::CodexVsCode => {
            return Err(invalid("Codex for VS Code has no deep link".to_owned()));
        }
    })
}

/// The Claude desktop app's ID of the Claude Code session `cli_session`: the `sessionId` of its
/// record, `claude-code-sessions/<account>/<organization>/local_<id>.json` in the app's data in
/// `home`, whose `cliSessionId` it is. Reads only those two keys.
fn desktop_session(home: &Path, cli_session: &str) -> io::Result<Option<String>> {
    let root = home.join("Library/Application Support/Claude/claude-code-sessions");
    let entries = |dir: &Path| -> Vec<PathBuf> {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| Some(entry.ok()?.path()))
                    .collect()
            })
            .unwrap_or_default()
    };
    for account in entries(&root) {
        for organization in entries(&account) {
            for record in entries(&organization) {
                let name = record.file_name().unwrap_or_default().to_string_lossy();
                if !(name.starts_with("local_") && name.ends_with(".json")) {
                    continue;
                }
                let Ok(record) = read_json(&record) else {
                    continue;
                };
                if record["cliSessionId"] == cli_session
                    && let Some(id) = record["sessionId"].as_str()
                {
                    return Ok(Some(id.to_owned()));
                }
            }
        }
    }
    Ok(None)
}

/// `text` percent-encoded for a URL's path segment or query value: all but the unreserved
/// characters.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The read confirmations of the message `msg_id` in `source`, a check's on `surface` whose
/// [`CHECK`] file is `check`: reads the session's file, as the client would, then tells each
/// signal and how long after the handover it came. The file is the transcript the `SessionStart`
/// hook registered or, for a thread `gui-queue` found, its rollout in `home`'s `.codex`.
fn confirmations(
    source: &Source,
    msg_id: &str,
    surface: Surface,
    check: &Value,
    home: &Path,
) -> io::Result<String> {
    if let (None, Some(thread)) = (source.registration()?, check["thread"].as_str())
        && let Some(rollout) = agent_transcript::codex_rollout(&home.join(".codex"), thread)?
    {
        source.register(&agent_source::Registration {
            session_id: thread.to_owned(),
            transcript_path: Some(rollout.to_string_lossy().into_owned()),
            ..agent_source::Registration::default()
        })?;
    }
    read::read_session_file(surface.agent(), source)?;
    let handed = match source.state(msg_id)? {
        Some(State::HandedOver { at_ms }) => Some(at_ms),
        _ => None,
    };
    Ok(format!(
        "  read confirmations after the handover: {}",
        read::signals(&source.records(msg_id)?, handed)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::read_json as json_at;
    use crate::{SENDER, token};

    const SURFACES: [Surface; 4] = [
        Surface::ClaudeDesktop,
        Surface::ClaudeVsCode,
        Surface::CodexDesktop,
        Surface::CodexVsCode,
    ];
    const PATHS: [GuiPath; 11] = [
        GuiPath::Inbox,
        GuiPath::Channel,
        GuiPath::Hook(Hook::PostToolUse),
        GuiPath::Hook(Hook::Stop),
        GuiPath::Hook(Hook::UserPromptSubmit),
        GuiPath::Queue,
        GuiPath::Wake(Wake::Socket),
        GuiPath::Wake(Wake::Rewake),
        GuiPath::Wake(Wake::Mod),
        GuiPath::Wake(Wake::Loop),
        GuiPath::Read,
    ];

    /// The metadata recorded on the test machine, in `tests/surfaces/`, where the surfaces keep
    /// it: the apps' `Info.plist`, the bundled Codex CLIs' `codex-package.json` and VS Code's list
    /// of extensions, each shortened.
    fn recorded() -> (tempfile::TempDir, Installed) {
        let root = tempfile::tempdir().unwrap();
        let surfaces = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/surfaces");
        let installed = Installed {
            applications: root.path().join("Applications"),
            home: root.path().join("home"),
        };
        let copy = |from: &str, to: PathBuf| {
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(surfaces.join(from), to).unwrap();
        };
        let apps = &installed.applications;
        copy("Claude.plist", apps.join("Claude.app/Contents/Info.plist"));
        copy(
            "ChatGPT.plist",
            apps.join("ChatGPT.app/Contents/Info.plist"),
        );
        copy(
            "codex-package-app.json",
            apps.join("ChatGPT.app/Contents/Resources/codex-cli/codex-package.json"),
        );
        let extensions = installed.home.join(".vscode/extensions");
        copy("extensions.json", extensions.join("extensions.json"));
        let codex = extensions.join("openai.chatgpt-26.1021.22027-darwin-arm64");
        copy(
            "codex-package-extension.json",
            codex.join("bin/macos-aarch64/codex-package.json"),
        );
        // A file next to the platform's directory, as the extension's `bin` may hold.
        fs::write(codex.join("bin/README"), "").unwrap();
        (root, installed)
    }

    #[test]
    fn reads_each_surface_version_from_recorded_metadata() {
        let (_root, installed) = recorded();
        let versions: Vec<String> = SURFACES
            .iter()
            .map(|surface| surface.version(&installed).unwrap())
            .collect();
        assert_eq!(
            versions,
            [
                "claude-desktop: Claude 4.23118.0",
                "claude-vscode: Claude Code for VS Code 2.1.322",
                "codex-desktop: ChatGPT 26.1027.51404, codex 0.166.0-alpha.5.1",
                "codex-vscode: Codex for VS Code 26.1021.22027, codex 0.164.0-alpha.11.1",
            ]
        );
    }

    #[test]
    fn refuses_an_app_that_is_not_the_codex_app() {
        let (_root, installed) = recorded();
        let plist = installed.app("ChatGPT.app").join("Contents/Info.plist");
        let text = fs::read_to_string(&plist).unwrap();
        fs::write(&plist, text.replace(CODEX_APP, "com.openai.chat")).unwrap();
        let error = Surface::CodexDesktop.version(&installed).unwrap_err();
        assert!(error.to_string().contains("not the Codex app"), "{error}");
        assert!(Surface::CodexDesktop.codex(&installed).is_err());
    }

    #[test]
    fn fails_without_the_surface() {
        let missing = Installed {
            applications: PathBuf::from("/nonexistent/Applications"),
            home: PathBuf::from("/nonexistent/home"),
        };
        for surface in SURFACES {
            assert!(surface.version(&missing).is_err(), "{surface:?}");
        }
        let (_root, installed) = recorded();
        let list = installed.home.join(".vscode/extensions/extensions.json");
        fs::write(&list, "[]").unwrap();
        let error = Surface::ClaudeVsCode.version(&installed).unwrap_err();
        assert!(error.to_string().contains("not installed"), "{error}");
    }

    #[test]
    fn fails_on_metadata_without_a_version() {
        let (_root, installed) = recorded();
        let plist = installed.app("Claude.app").join("Contents/Info.plist");
        fs::write(&plist, "<plist><dict></dict></plist>").unwrap();
        let error = Surface::ClaudeDesktop.version(&installed).unwrap_err();
        assert!(
            error.to_string().contains("CFBundleShortVersionString"),
            "{error}"
        );
        let platform = installed
            .home
            .join(".vscode/extensions/openai.chatgpt-26.1021.22027-darwin-arm64/bin/macos-aarch64");
        fs::write(platform.join("codex-package.json"), "{}").unwrap();
        let error = Surface::CodexVsCode.version(&installed).unwrap_err();
        assert!(error.to_string().contains("has no version"), "{error}");
        fs::create_dir_all(platform.with_file_name("linux-x86_64")).unwrap();
        let error = Surface::CodexVsCode.version(&installed).unwrap_err();
        assert!(error.to_string().contains("2 directories"), "{error}");
    }

    #[test]
    fn parses_surfaces_and_paths() {
        for surface in SURFACES {
            assert_eq!(Surface::parse(surface.name()), Some(surface));
        }
        for path in PATHS {
            assert_eq!(GuiPath::parse(path.name()), Some(path));
        }
        assert_eq!(Surface::parse("claude"), None);
        assert_eq!(GuiPath::parse("app-server"), None);
        assert_eq!(GuiPath::parse("pre-tool-use"), None);
    }

    #[test]
    fn runs_each_path_where_its_surface_can_take_it() {
        for surface in SURFACES {
            let claude = surface.agent() == Agent::Claude;
            assert_eq!(
                claude,
                matches!(surface, Surface::ClaudeDesktop | Surface::ClaudeVsCode)
            );
            for path in PATHS {
                let (runs, busy) = match path {
                    GuiPath::Inbox => (true, claude),
                    GuiPath::Channel => (claude, false),
                    GuiPath::Hook(Hook::UserPromptSubmit) => (!claude, false),
                    GuiPath::Hook(_) => (true, true),
                    GuiPath::Queue | GuiPath::Read => (!claude, !claude),
                    GuiPath::Wake(_) => (claude, claude),
                };
                assert_eq!(path.runs_on(surface), runs, "{path:?} on {surface:?}");
                assert_eq!(path.runs_busy_on(surface), busy, "{path:?} on {surface:?}");
                assert_eq!(
                    path.needs_thread(),
                    matches!(path, GuiPath::Queue | GuiPath::Read),
                    "{path:?}"
                );
                assert_eq!(path.wakes(), matches!(path, GuiPath::Wake(_)), "{path:?}");
                assert_eq!(
                    path.confirms(),
                    matches!(path, GuiPath::Wake(_) | GuiPath::Read),
                    "{path:?}"
                );
            }
        }
    }

    fn binaries() -> Binaries {
        Binaries {
            inbox: PathBuf::from("/bin/agent-inbox"),
            channel: PathBuf::from("/bin/agent-channel"),
            hook: PathBuf::from("/bin/agent-hook"),
            user_mcp_servers: vec![
                "alpha".to_owned(),
                "skald".to_owned(),
                "beta-two".to_owned(),
            ],
            user_claude: UserClaude {
                plugins: vec!["one@market".to_owned(), "two@market".to_owned()],
                mcp_servers: vec!["gamma".to_owned(), "skald".to_owned()],
            },
        }
    }

    /// `settings` with the user's Claude Code plugins and servers of [`binaries`] turned off.
    fn claude_off(mut settings: Value) -> String {
        settings["disableClaudeAiConnectors"] = true.into();
        settings["enabledPlugins"] = json!({ "one@market": false, "two@market": false });
        settings["permissions"]["deny"] = json!(["mcp__gamma"]);
        pretty(&settings)
    }

    #[test]
    fn reads_only_the_names_of_the_users_claude_plugins_and_servers() {
        let settings = json!({
            "enabledPlugins": { "on@m": true, "off@m": false, "odd@m": "yes" },
            "model": "opus",
        });
        let config = json!({ "mcpServers": { "exa": { "command": "x" }, "local": {} } });
        assert_eq!(
            UserClaude::read(&settings, &config),
            UserClaude {
                plugins: vec!["on@m".to_owned()],
                mcp_servers: vec!["exa".to_owned(), "local".to_owned()],
            }
        );
        assert_eq!(
            UserClaude::read(&json!({}), &json!(null)),
            UserClaude::default()
        );
        // With nothing else to turn off, only the claude.ai connectors are.
        let connectors = json!({ "maxEffortLevel": "low", "disableClaudeAiConnectors": true });
        let mut settings = json!({ "maxEffortLevel": "low" });
        UserClaude::default().turn_off(&mut settings);
        assert_eq!(settings, connectors);
        let mut settings = json!({ "maxEffortLevel": "low" });
        UserClaude {
            plugins: Vec::new(),
            mcp_servers: vec!["skald".to_owned()],
        }
        .turn_off(&mut settings);
        assert_eq!(settings, connectors);
        // A rule names a server as its tools do.
        let mut settings = json!({});
        UserClaude {
            plugins: Vec::new(),
            mcp_servers: vec![
                "my server.v2".to_owned(),
                "a_b-c9".to_owned(),
                "sk.ald".to_owned(),
            ],
        }
        .turn_off(&mut settings);
        assert_eq!(
            settings["permissions"]["deny"],
            json!(["mcp__my_server_v2", "mcp__a_b-c9", "mcp__sk_ald"])
        );
    }

    /// The files under `dir`, as paths relative to it, sorted.
    fn files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut dirs = vec![dir.to_owned()];
        while let Some(next) = dirs.pop() {
            for entry in fs::read_dir(next).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    files.push(path.strip_prefix(dir).unwrap().to_owned());
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn writes_the_config_each_surface_reads() {
        let source = Path::new("/work/source");
        let server = |command: &str| {
            pretty(&json!({ "mcpServers": { "skald": { "command": command, "args": [source] } } }))
        };
        let hook = |event: &str| {
            claude_off(json!({
                "hooks": { event: [{ "hooks": [{
                    "type": "command",
                    "command": "'/bin/agent-hook' '/work/source'",
                }] }] },
                "permissions": { "allow": ["Bash(true)", "Bash(sleep:*)"] },
                "maxEffortLevel": "low",
            }))
        };
        let codex_model = "model = \"gpt-5.5\"\nmodel_reasoning_effort = \"low\"\n";
        // The user's MCP servers and plugins turned off, but for a server named as the check's.
        let codex_off = "\n[features]\nplugins = false\n\n[mcp_servers.\"alpha\"]\nenabled = false\n\n\
                         [mcp_servers.\"beta-two\"]\nenabled = false\n";
        let codex_inbox = format!(
            "{codex_model}\n[mcp_servers.skald]\ncommand = \"/bin/agent-inbox\"\n\
             args = [\"/work/source\"]\n\
             # Without it, Codex asks a person to approve each call.\n\
             default_tools_approval_mode = \"approve\"\n{codex_off}"
        );
        // A dotted key at the top level, before any table.
        let codex_hook = |event: &str| {
            format!(
                "{codex_model}hooks.{event}=[{{hooks=[{{type=\"command\",\
                 command=\"'/bin/agent-hook' '--codex' '/work/source'\"}}]}}]\n{codex_off}"
            )
        };
        for surface in SURFACES {
            for path in PATHS.into_iter().filter(|path| path.runs_on(surface)) {
                let dir = tempfile::tempdir().unwrap();
                let project = dir.path().join("project");
                write_config(&project, surface, path, &binaries(), source).unwrap();
                let expected = match (surface.agent(), path) {
                    (Agent::Codex, GuiPath::Inbox) => {
                        vec![(".codex/config.toml", codex_inbox.clone())]
                    }
                    (Agent::Codex, GuiPath::Hook(hook)) => {
                        vec![(".codex/config.toml", codex_hook(hook.event()))]
                    }
                    (Agent::Codex, GuiPath::Read) => vec![(
                        ".codex/config.toml",
                        codex_inbox.replacen(
                            codex_model,
                            &format!(
                                "{codex_model}hooks.Stop=[{{hooks=[{{type=\"command\",\
                                 command=\"'/bin/agent-hook' '--confirm' '--codex' \
                                 '/work/source'\"}}]}}]\n"
                            ),
                            1,
                        ),
                    )],
                    (Agent::Codex, _) => {
                        vec![(".codex/config.toml", format!("{codex_model}{codex_off}"))]
                    }
                    (Agent::Claude, GuiPath::Inbox) => vec![
                        (".mcp.json", server("/bin/agent-inbox")),
                        (
                            ".claude/settings.local.json",
                            claude_off(json!({
                                "enabledMcpjsonServers": ["skald"],
                                "permissions": { "allow": ["mcp__skald__inbox", "Bash(sleep:*)"] },
                                "maxEffortLevel": "low",
                            })),
                        ),
                    ],
                    (Agent::Claude, GuiPath::Channel) => vec![
                        (".mcp.json", server("/bin/agent-channel")),
                        (
                            ".claude/settings.local.json",
                            claude_off(json!({
                                "enabledMcpjsonServers": ["skald"],
                                "maxEffortLevel": "low",
                            })),
                        ),
                    ],
                    (Agent::Claude, GuiPath::Hook(Hook::PostToolUse)) => {
                        vec![(".claude/settings.local.json", hook("PostToolUse"))]
                    }
                    (Agent::Claude, GuiPath::Wake(wake)) => {
                        let handler = |args: &[&str]| {
                            let words: Vec<String> = std::iter::once("/bin/agent-hook")
                                .chain(args.iter().copied())
                                .map(|word| format!("'{word}'"))
                                .collect();
                            json!({ "type": "command", "command": words.join(" ") })
                        };
                        let confirm = handler(&["--confirm", "/work/source"]);
                        let settings = json!({
                            "hooks": wake::hooks(wake, &handler, source, &[("Stop", confirm)]),
                            "enabledMcpjsonServers": ["skald"],
                            "permissions": { "allow": [
                                "mcp__skald__inbox", "mcp__skald__ack", "mcp__skald__reply", "Bash(sleep:*)"
                            ] },
                            "maxEffortLevel": "low",
                        });
                        let settings = claude_off(settings);
                        let mut files = vec![
                            (".mcp.json", server("/bin/agent-inbox")),
                            (".claude/settings.local.json", settings),
                        ];
                        if wake == Wake::Mod {
                            files.push((
                                ".skald/mod.json",
                                pretty(&json!({ "handler": "/bin/agent-hook", "source": source })),
                            ));
                        }
                        files
                    }
                    (Agent::Claude, _) => {
                        vec![(".claude/settings.local.json", hook("Stop"))]
                    }
                };
                let mut names: Vec<PathBuf> =
                    expected.iter().map(|(name, _)| name.into()).collect();
                names.sort();
                assert_eq!(files(&project), names, "{path:?} on {surface:?}");
                for (name, text) in expected {
                    let written = fs::read_to_string(project.join(name)).unwrap();
                    assert_eq!(written, text, "{name}: {path:?} on {surface:?}");
                }
            }
        }
    }

    #[test]
    fn writes_no_config_for_a_path_its_surface_cannot_take() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        for (surface, path) in [
            (Surface::ClaudeVsCode, GuiPath::Queue),
            (
                Surface::ClaudeDesktop,
                GuiPath::Hook(Hook::UserPromptSubmit),
            ),
            (Surface::CodexDesktop, GuiPath::Channel),
            (Surface::CodexVsCode, GuiPath::Wake(Wake::Rewake)),
            (Surface::ClaudeDesktop, GuiPath::Read),
        ] {
            let error =
                write_config(&project, surface, path, &binaries(), Path::new("/s")).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("{} does not run on {}", path.name(), surface.name())
            );
            assert!(!project.exists(), "{path:?} on {surface:?}");
        }
    }

    #[test]
    fn prompts_as_the_cli_checks() {
        for (busy, kind) in [(false, Kind::Idle), (true, Kind::Busy)] {
            assert_eq!(GuiPath::Inbox.prompt(busy), kind.prompt());
            for hook in [Hook::PostToolUse, Hook::Stop] {
                assert_eq!(GuiPath::Hook(hook).prompt(busy), hook.prompt(kind));
            }
        }
        assert_eq!(
            GuiPath::Channel.prompt(false),
            Hook::Stop.prompt(Kind::Idle)
        );
        assert_eq!(
            GuiPath::Hook(Hook::UserPromptSubmit).prompt(false),
            Hook::UserPromptSubmit.prompt(Kind::Idle)
        );
        for (busy, kind) in [(false, Kind::Idle), (true, Kind::Busy)] {
            for path in [GuiPath::Queue, GuiPath::Read, GuiPath::Wake(Wake::Socket)] {
                assert_eq!(path.prompt(busy), queue::prompt(kind));
            }
            assert_eq!(
                GuiPath::Wake(Wake::Loop).prompt(busy),
                format!("/loop 1m {}", crate::LOOP_PROMPT)
            );
        }
    }

    #[test]
    fn a_wake_check_adds_its_message_with_a_secret_only_when_asked() {
        for path in [GuiPath::Wake(Wake::Socket), GuiPath::Wake(Wake::Mod)] {
            let work = tempfile::tempdir().unwrap();
            let token = token();
            prepared(work.path(), Surface::ClaudeDesktop, path, &token, false);
            assert!(
                added(work.path()).unwrap().is_none(),
                "added before the first turn"
            );
            let message = add(work.path()).unwrap();
            assert_eq!(message.text, channel::text(&token));
            assert_eq!(message.ack.as_ref().map(String::len), Some(32));
            let (source, msg_id) = added(work.path()).unwrap().unwrap();
            assert_eq!(msg_id, message.msg_id);
            assert_eq!(source.state(&msg_id).unwrap(), Some(State::Waiting));
        }
    }

    #[test]
    fn writes_a_vs_code_profile_only_for_the_mod_and_a_bypass_session() {
        let plugin = Path::new("/p/mod");
        let written = |surface, path, bypass| {
            let work = tempfile::tempdir().unwrap();
            let profile = vscode_profile(work.path(), surface, path, plugin, bypass).unwrap();
            profile.map(|profile| {
                assert_eq!(profile, work.path().join("vscode"));
                json_at(profile.join("User/settings.json"))
            })
        };
        assert_eq!(
            written(Surface::ClaudeVsCode, GuiPath::Wake(Wake::Mod), false),
            Some(json!({ "claudeCode.environmentVariables": [
                { "name": "CLAUDE_CODE_PLUGIN_DIRS", "value": "/p/mod" }
            ] }))
        );
        assert_eq!(
            written(Surface::ClaudeVsCode, GuiPath::Wake(Wake::Socket), true),
            Some(json!({
                "claudeCode.allowDangerouslySkipPermissions": true,
                "claudeCode.initialPermissionMode": "bypassPermissions",
            }))
        );
        for (surface, path, bypass) in [
            (Surface::ClaudeVsCode, GuiPath::Wake(Wake::Socket), false),
            (Surface::ClaudeDesktop, GuiPath::Wake(Wake::Mod), false),
            (Surface::ClaudeVsCode, GuiPath::Inbox, false),
        ] {
            assert_eq!(
                written(surface, path, bypass),
                None,
                "{path:?} on {surface:?}"
            );
        }
    }

    #[test]
    fn tells_the_read_confirmations_from_the_session_file_and_the_records() {
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let token = token();
        prepared(
            work.path(),
            Surface::CodexDesktop,
            GuiPath::Read,
            &token,
            false,
        );
        let message = add(work.path()).unwrap();
        hand_over(work.path());
        let source = Source::open(work.path().join("source")).unwrap();
        source.record(&message.msg_id, "queue-taken", None).unwrap();
        let thread = "01a1ce22-0000-7000-8000-000000000001";
        set_thread(work.path(), thread).unwrap();
        let day = home.path().join(".codex/sessions/2026/11/12");
        fs::create_dir_all(&day).unwrap();
        fs::write(
            day.join(format!("rollout-2026-11-12T14-52-10-{thread}.jsonl")),
            format!(
                "{}\n{}",
                format_args!(
                    r#"{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"text":"{}"}}]}}}}"#,
                    message.msg_id
                ),
                r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[]}}"#
            ),
        )
        .unwrap();
        let outcome = result(work.path(), &token, home.path()).unwrap();
        assert!(outcome.pass);
        let line = outcome.confirmations.unwrap();
        let signals = line
            .strip_prefix("  read confirmations after the handover: ")
            .unwrap();
        let names: Vec<&str> = signals
            .split(", ")
            .map(|signal| signal.split(' ').next().unwrap())
            .collect();
        assert_eq!(names, ["queue-taken", "session-file"], "{line}");
        // Each tells how long after the handover it came.
        for signal in signals.split(", ") {
            let words: Vec<&str> = signal.split(' ').collect();
            assert!(words[1].parse::<f64>().is_ok() && words[2] == "s", "{line}");
        }
        assert_eq!(
            source.registration().unwrap().unwrap().session_id,
            thread,
            "the rollout is registered"
        );
        // A check of another path tells none.
        let other = tempfile::tempdir().unwrap();
        prepared(
            other.path(),
            Surface::CodexDesktop,
            GuiPath::Queue,
            &token,
            false,
        );
        assert_eq!(add(other.path()).unwrap().ack, None, "a secret for no ack");
        assert_eq!(
            result(other.path(), &token, home.path())
                .unwrap()
                .confirmations,
            None
        );
    }

    /// Prepares `path`'s check on `surface` in `work` with `token`, in the `busy` case or not.
    fn prepared(work: &Path, surface: Surface, path: GuiPath, token: &str, busy: bool) -> Prepared {
        prepare(
            work,
            surface,
            path,
            &binaries(),
            "a version line",
            token,
            busy,
        )
        .unwrap()
    }

    /// A home without transcripts.
    fn no_home() -> &'static Path {
        Path::new("/nonexistent/home")
    }

    /// Marks every waiting message of the check in `work` handed over, as a path does.
    fn hand_over(work: &Path) {
        let source = Source::open(work.join("source")).unwrap();
        let ids: Vec<String> = source
            .take()
            .unwrap()
            .into_iter()
            .map(|message| message.msg_id)
            .collect();
        source.handed_over(&ids).unwrap();
    }

    #[test]
    fn prepares_the_project_and_the_message() {
        for (surface, path) in [
            (Surface::ClaudeVsCode, GuiPath::Inbox),
            (Surface::ClaudeDesktop, GuiPath::Hook(Hook::Stop)),
            (Surface::ClaudeDesktop, GuiPath::Channel),
            (Surface::CodexDesktop, GuiPath::Inbox),
        ] {
            let work = tempfile::tempdir().unwrap();
            let token = token();
            let prepared = prepared(work.path(), surface, path, &token, false);
            assert_eq!(prepared.project, work.path().join("project"));
            assert_eq!(prepared.prompt, path.prompt(false));
            assert!(!files(&prepared.project).is_empty());
            let source = Source::open(work.path().join("source")).unwrap();
            let taken = source.take().unwrap();
            assert_eq!(taken.len(), 1);
            let expected = match path {
                GuiPath::Inbox => format!("The check token is {token}."),
                _ => channel::text(&token),
            };
            assert_eq!(taken[0].text, expected);
            assert_eq!(taken[0].sender, SENDER);
            let check = json_at(work.path().join(CHECK));
            assert_eq!(check["msg_id"], taken[0].msg_id);
            assert_eq!(check["surface"], surface.name());
            assert_eq!(check["path"], path.name());
            assert!(add(work.path()).is_err(), "added twice");
        }
    }

    #[test]
    fn adds_the_busy_case_message_only_when_asked() {
        let work = tempfile::tempdir().unwrap();
        let token = token();
        let path = GuiPath::Hook(Hook::Stop);
        let prepared = prepared(work.path(), Surface::ClaudeVsCode, path, &token, true);
        assert_eq!(prepared.prompt, path.prompt(true));
        let source = Source::open(work.path().join("source")).unwrap();
        assert!(source.take().unwrap().is_empty());
        let never = result(work.path(), &token, no_home()).unwrap();
        assert!(!never.pass);
        assert_eq!(
            never.details,
            "  the reported token matches; the message is never added"
        );

        let message = add(work.path()).unwrap();
        assert_eq!(message.text, channel::text(&token));
        assert_eq!(json_at(work.path().join(CHECK))["msg_id"], message.msg_id);
        assert!(add(work.path()).is_err(), "added twice");
        let taken = source.take().unwrap();
        assert_eq!(taken, std::slice::from_ref(&message));
        source.handed_over(&[message.msg_id]).unwrap();
        let read = result(work.path(), &token, no_home()).unwrap();
        assert!(read.pass);
        let delay = read
            .details
            .strip_prefix("  the reported token matches; the message is handed over ")
            .and_then(|rest| rest.strip_suffix(" s after it was added"))
            .expect("the handover's delay");
        assert!(delay.parse::<f64>().is_ok(), "{delay}");
    }

    #[test]
    fn a_queue_check_adds_its_message_only_when_asked_and_knows_its_thread_prompt() {
        for busy in [false, true] {
            let work = tempfile::tempdir().unwrap();
            let token = token();
            let prepared = prepared(
                work.path(),
                Surface::CodexVsCode,
                GuiPath::Queue,
                &token,
                busy,
            );
            assert_eq!(prepared.prompt, GuiPath::Queue.prompt(busy));
            let source = Source::open(work.path().join("source")).unwrap();
            assert!(source.take().unwrap().is_empty(), "added before the thread");
            assert_eq!(
                pending(work.path()).unwrap(),
                Pending {
                    surface: Surface::CodexVsCode,
                    project: work.path().join("project"),
                    prompt: queue::prompt(if busy { Kind::Busy } else { Kind::Idle }),
                }
            );
            assert_eq!(add(work.path()).unwrap().text, channel::text(&token));
        }
    }

    #[test]
    fn only_a_queue_check_waits_for_the_thread() {
        let work = tempfile::tempdir().unwrap();
        prepared(
            work.path(),
            Surface::CodexDesktop,
            GuiPath::Hook(Hook::Stop),
            &token(),
            true,
        );
        let error = pending(work.path()).unwrap_err();
        assert!(
            error.to_string().contains("not of a path that needs"),
            "{error}"
        );
        assert!(pending(&work.path().join("none")).is_err());
        fs::write(
            work.path().join(CHECK),
            r#"{"path": "queue", "surface": "codex-tv"}"#,
        )
        .unwrap();
        let error = pending(work.path()).unwrap_err();
        assert!(error.to_string().contains("no known surface"), "{error}");
    }

    #[test]
    fn finds_the_newest_thread_that_the_check_prompt_started() {
        let exchange = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/surfaces/thread-list.jsonl"),
        )
        .unwrap();
        let list = exchange
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find_map(|line| {
                line["server"]["result"]
                    .get("data")
                    .map(|_| line["server"]["result"].clone())
            })
            .unwrap();
        assert_eq!(
            session_thread(&list, &queue::prompt(Kind::Busy)).as_deref(),
            Some("01a1ce21-961d-7a10-b19d-afc3cbde7c0d")
        );
        let mut twice = list.clone();
        let first = twice["data"][1].clone();
        twice["data"].as_array_mut().unwrap().push(first);
        twice["data"][2]["id"] = "older".into();
        assert_eq!(
            session_thread(&twice, &queue::prompt(Kind::Busy)).as_deref(),
            Some("01a1ce21-961d-7a10-b19d-afc3cbde7c0d")
        );
        assert_eq!(
            session_thread(&list, &queue::prompt(Kind::Idle)).as_deref(),
            Some("01a1ce22-0000-7000-8000-000000000001")
        );
        assert_eq!(session_thread(&list, "Another prompt."), None);
        assert_eq!(session_thread(&json!({}), "Reply with only OK."), None);
    }

    #[test]
    fn finds_the_codex_cli_each_codex_surface_bundles() {
        let (_root, installed) = recorded();
        let app = installed
            .applications
            .join("ChatGPT.app/Contents/Resources/codex-cli");
        let extension = installed
            .home
            .join(".vscode/extensions/openai.chatgpt-26.1021.22027-darwin-arm64/bin/macos-aarch64");
        for surface in [Surface::CodexDesktop, Surface::CodexVsCode] {
            let error = surface.codex(&installed).unwrap_err();
            assert!(
                error.to_string().contains("has no bin/codex or codex"),
                "{error}"
            );
        }
        // As the surfaces lay them out: the app's under `bin`, the extension's next to its package.
        fs::create_dir_all(app.join("bin")).unwrap();
        fs::write(app.join("bin/codex"), "").unwrap();
        fs::write(extension.join("codex"), "").unwrap();
        assert_eq!(
            Surface::CodexDesktop.codex(&installed).unwrap(),
            app.join("bin/codex")
        );
        assert_eq!(
            Surface::CodexVsCode.codex(&installed).unwrap(),
            extension.join("codex")
        );
        fs::create_dir_all(extension.join("bin")).unwrap();
        fs::write(extension.join("bin/codex"), "").unwrap();
        assert_eq!(
            Surface::CodexVsCode.codex(&installed).unwrap(),
            extension.join("bin/codex")
        );
        for surface in [Surface::ClaudeDesktop, Surface::ClaudeVsCode] {
            let error = surface.codex(&installed).unwrap_err();
            assert!(
                error.to_string().contains("bundles no Codex CLI"),
                "{error}"
            );
        }
    }

    #[test]
    fn reads_the_claude_code_version_a_session_ran() {
        let home = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let token = token();
        prepared(
            work.path(),
            Surface::ClaudeDesktop,
            GuiPath::Inbox,
            &token,
            false,
        );
        let project = fs::canonicalize(work.path().join("project")).unwrap();
        let key: String = project.to_string_lossy().replace(['/', '_', '.'], "-");
        let transcripts = home.path().join(".claude/projects").join(key);
        fs::create_dir_all(&transcripts).unwrap();
        let recorded = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/surfaces/claude-desktop-transcript.jsonl");
        fs::copy(&recorded, transcripts.join("a.jsonl")).unwrap();
        fs::write(transcripts.join("b.txt"), r#"{"version":"9.9.9"}"#).unwrap();

        let outcome = result(work.path(), &token, home.path()).unwrap();
        assert_eq!(
            outcome.version,
            "a version line, the session ran Claude Code 2.1.317"
        );
        fs::write(transcripts.join("b.jsonl"), r#"{"version":"2.1.321"}"#).unwrap();
        assert_eq!(
            session_versions(home.path(), &work.path().join("project")).unwrap(),
            ["2.1.317", "2.1.321"]
        );
        let none = result(work.path(), &token, no_home()).unwrap();
        assert_eq!(none.version, "a version line, no session transcript");

        let codex = tempfile::tempdir().unwrap();
        prepared(
            codex.path(),
            Surface::CodexDesktop,
            GuiPath::Inbox,
            &token,
            false,
        );
        let outcome = result(codex.path(), &token, home.path()).unwrap();
        assert_eq!(outcome.version, "a version line");
    }

    #[test]
    fn passes_only_when_the_reported_token_matches_and_the_message_is_handed_over() {
        let work = tempfile::tempdir().unwrap();
        let token = token();
        prepared(
            work.path(),
            Surface::CodexVsCode,
            GuiPath::Inbox,
            &token,
            false,
        );

        let waiting = result(work.path(), &token, no_home()).unwrap();
        assert!(!waiting.pass);
        assert_eq!(
            waiting.details,
            "  the reported token matches; the message is not handed over (state Some(Waiting))"
        );
        assert_eq!(waiting.version, "a version line");
        assert_eq!(
            waiting.verdict,
            "inbox on codex-vscode with the real model: fail"
        );

        hand_over(work.path());
        let wrong = result(work.path(), "NONE", no_home()).unwrap();
        assert!(!wrong.pass);
        assert_eq!(
            wrong.details,
            format!(
                "  the reported token differs from the check token {token}; the message is \
                 handed over"
            )
        );

        let read = result(work.path(), &format!(" {token}\n"), no_home()).unwrap();
        assert!(read.pass);
        assert_eq!(
            read.details,
            "  the reported token matches; the message is handed over"
        );
        assert_eq!(
            read.verdict,
            "inbox on codex-vscode with the real model: pass"
        );
    }

    #[test]
    fn links_to_the_checks_session_on_each_surface_that_has_a_link() {
        let home = tempfile::tempdir().unwrap();
        let link = |work: &Path| link(work, home.path());
        let error = |work: &Path| link(work).unwrap_err().to_string();
        let register = |work: &Path, session: &str| {
            Source::open(work.join("source"))
                .unwrap()
                .register(&agent_source::Registration {
                    session_id: session.to_owned(),
                    ..agent_source::Registration::default()
                })
                .unwrap();
        };
        let token = token();

        let desktop = tempfile::tempdir().unwrap();
        prepared(
            desktop.path(),
            Surface::ClaudeDesktop,
            GuiPath::Wake(Wake::Socket),
            &token,
            false,
        );
        assert_eq!(error(desktop.path()), "the check has no session yet");
        register(desktop.path(), "cli-1");
        assert_eq!(
            error(desktop.path()),
            "the Claude app has no record of the session"
        );
        let records = home
            .path()
            .join("Library/Application Support/Claude/claude-code-sessions/acct/org");
        fs::create_dir_all(&records).unwrap();
        fs::write(records.join("notes.txt"), "cli-1").unwrap();
        fs::write(records.join("local_bad.json"), "{").unwrap();
        // Only the app's session records count.
        for decoy in ["decoy.json", "local_decoy.txt"] {
            fs::write(
                records.join(decoy),
                r#"{"cliSessionId":"cli-3","sessionId":"local_decoy"}"#,
            )
            .unwrap();
        }
        assert_eq!(desktop_session(home.path(), "cli-3").unwrap(), None);
        fs::write(
            records.join("local_other.json"),
            r#"{"cliSessionId":"cli-2","sessionId":"local_other"}"#,
        )
        .unwrap();
        fs::write(
            records.join("local_s 1.json"),
            r#"{"cliSessionId":"cli-1","sessionId":"local_s 1/é","title":"t"}"#,
        )
        .unwrap();
        assert_eq!(
            link(desktop.path()).unwrap(),
            "claude://code/continue?session=local_s%201%2F%C3%A9"
        );

        let vscode = tempfile::tempdir().unwrap();
        prepared(
            vscode.path(),
            Surface::ClaudeVsCode,
            GuiPath::Wake(Wake::Rewake),
            &token,
            false,
        );
        register(vscode.path(), "abc-1");
        add(vscode.path()).unwrap();
        let vscode_link = link(vscode.path()).unwrap();
        assert_eq!(
            vscode_link,
            format!(
                "vscode://anthropic.claude-code/open?session=abc-1&prompt={}",
                encode(&Kind::Idle.prompt())
            )
        );
        assert!(!vscode_link.contains(&token));

        let codex = tempfile::tempdir().unwrap();
        prepared(
            codex.path(),
            Surface::CodexDesktop,
            GuiPath::Read,
            &token,
            false,
        );
        assert_eq!(error(codex.path()), "the check has no session yet");
        set_thread(codex.path(), "019a-thread").unwrap();
        assert_eq!(link(codex.path()).unwrap(), "codex://threads/019a-thread");

        let codex_vscode = tempfile::tempdir().unwrap();
        prepared(
            codex_vscode.path(),
            Surface::CodexVsCode,
            GuiPath::Read,
            &token,
            false,
        );
        set_thread(codex_vscode.path(), "019a-thread").unwrap();
        assert_eq!(
            error(codex_vscode.path()),
            "Codex for VS Code has no deep link"
        );
        assert!(link(Path::new("/nonexistent/check")).is_err());
        assert_eq!(
            desktop_session(Path::new("/nonexistent/home"), "cli-1").unwrap(),
            None
        );
    }

    #[test]
    fn percent_encodes_all_but_the_unreserved_characters() {
        assert_eq!(encode("Az09-._~"), "Az09-._~");
        assert_eq!(encode("a b/?&=%\n"), "a%20b%2F%3F%26%3D%25%0A");
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[test]
    fn result_fails_without_a_prepared_check() {
        let work = tempfile::tempdir().unwrap();
        assert!(result(work.path(), "tok", no_home()).is_err());
        fs::write(work.path().join(CHECK), "{}").unwrap();
        let error = result(work.path(), "tok", no_home()).unwrap_err();
        assert!(error.to_string().contains("has no token"), "{error}");
    }
}
