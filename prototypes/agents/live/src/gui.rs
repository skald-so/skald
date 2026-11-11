//! The GUI live checks: a path on a Claude Code or Codex session in a desktop app or an IDE
//! extension that a developer or reviewer drives. [`prepare`] makes the temporary project with the
//! surface's config and the message source with the check's message; [`result`] takes the token
//! they read from the agent's reply. Neither shows the token before the reported one.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use agent_source::{Message, Source, State};
use serde_json::{Value, json};

use crate::hook::Hook;
use crate::{
    Agent, Kind, QUICK, SERVER, add_message, channel, handed_over, inbox_tool, pretty, shell_quote,
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
    /// the app's or extension's version and the version of the agent it runs.
    pub fn version(self, installed: &Installed) -> io::Result<String> {
        let version = match self {
            Self::ClaudeDesktop => {
                let app = bundle_version(&installed.app("Claude.app"))?;
                let code = newest_version(
                    &installed
                        .home
                        .join("Library/Application Support/Claude/claude-code"),
                )?;
                format!("Claude {app}, Claude Code {code}, the newest the app downloaded")
            }
            // The extension's version is that of the Claude Code it bundles.
            Self::ClaudeVsCode => {
                let (version, _) = installed.extension("anthropic.claude-code")?;
                format!("Claude Code for VS Code {version}")
            }
            Self::CodexDesktop => {
                let app = installed.app("ChatGPT.app");
                let id = plist_string(&app, "CFBundleIdentifier")?;
                if id != CODEX_APP {
                    return Err(invalid(format!(
                        "{} is {id}, not the Codex app {CODEX_APP}",
                        app.display()
                    )));
                }
                let codex = codex_package(&app.join("Contents/Resources/codex-cli"))?;
                format!("ChatGPT {}, codex {codex}", bundle_version(&app)?)
            }
            Self::CodexVsCode => {
                let (version, dir) = installed.extension("openai.chatgpt")?;
                let codex = codex_package(&only_dir(&dir.join("bin"))?)?;
                format!("Codex for VS Code {version}, codex {codex}")
            }
        };
        Ok(format!("{}: {version}", self.name()))
    }
}

/// The Codex desktop app's bundle ID.
const CODEX_APP: &str = "com.openai.codex";

/// Where the surfaces are installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The apps' directory, such as `/Applications`.
    pub applications: PathBuf,
    /// The user's home: VS Code's extensions and the Claude app's downloads of Claude Code.
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

/// The newest of the versions, such as `2.1.317`, that name the directories in `dir`.
fn newest_version(dir: &Path) -> io::Result<String> {
    let parse = |name: &str| -> Option<Vec<u64>> {
        name.split('.').map(|part| part.parse().ok()).collect()
    };
    fs::read_dir(dir)?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| Some((parse(&name)?, name)))
        .max()
        .map(|(_, name)| name)
        .ok_or_else(|| invalid(format!("{} holds no version", dir.display())))
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
    /// Claude Code's `PostToolUse` or `Stop` hook.
    Hook(Hook),
}

impl GuiPath {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "inbox" => Some(Self::Inbox),
            "channel" => Some(Self::Channel),
            "post-tool-use" => Some(Self::Hook(Hook::PostToolUse)),
            "stop" => Some(Self::Hook(Hook::Stop)),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Channel => "channel",
            Self::Hook(Hook::PostToolUse) => "post-tool-use",
            Self::Hook(_) => "stop",
        }
    }

    /// Whether the GUI check runs on `surface`: `inbox` on each; Channels and the hooks on Claude
    /// Code's.
    pub fn runs_on(self, surface: Surface) -> bool {
        self == Self::Inbox || surface.agent() == Agent::Claude
    }

    /// The prompt the developer or reviewer types.
    pub fn prompt(self) -> String {
        match self {
            Self::Inbox => Kind::Idle.prompt(),
            Self::Hook(hook) => hook.prompt(Kind::Idle),
            // Only a reply, as for the `Stop` hook: a loaded channel pushes the message itself.
            Self::Channel => Hook::Stop.prompt(Kind::Idle),
        }
    }

    /// Adds the check's message with `token` to `source`: the text of the path's CLI check.
    fn add_message(self, source: &Source, token: &str) -> io::Result<Message> {
        match self {
            Self::Inbox => add_message(source, token),
            Self::Channel | Self::Hook(_) => channel::add_message(source, token),
        }
    }
}

/// The binaries a project's config runs, next to `live-check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binaries {
    pub inbox: PathBuf,
    pub channel: PathBuf,
    pub hook: PathBuf,
}

/// The model of Codex's sessions: the API key of the test machine has no access to Codex's default
/// model.
pub const CODEX_MODEL: &str = "gpt-5.5";

/// Writes the config that attaches `path`, on the source in `source`, to `surface`'s sessions in
/// `project`. Only files in `project` change the surface's config: the surfaces take no flags.
pub fn write_config(
    project: &Path,
    surface: Surface,
    path: GuiPath,
    binaries: &Binaries,
    source: &Path,
) -> io::Result<()> {
    fs::create_dir_all(project)?;
    match surface.agent() {
        Agent::Claude => {
            let server = |command: &Path| json!({ "mcpServers": { SERVER: { "command": command, "args": [source] } } });
            let settings = match path {
                GuiPath::Inbox => {
                    fs::write(project.join(".mcp.json"), pretty(&server(&binaries.inbox)))?;
                    // Without them, Claude Code asks a person to approve the server and each call.
                    json!({
                        "enabledMcpjsonServers": [SERVER],
                        "permissions": { "allow": [inbox_tool()] },
                    })
                }
                GuiPath::Channel => {
                    fs::write(
                        project.join(".mcp.json"),
                        pretty(&server(&binaries.channel)),
                    )?;
                    json!({ "enabledMcpjsonServers": [SERVER] })
                }
                GuiPath::Hook(hook) => {
                    // The shell form: Claude Code runs it through the shell.
                    let command = [&binaries.hook, source]
                        .map(|arg| shell_quote(&arg.to_string_lossy()))
                        .join(" ");
                    let handlers =
                        json!([{ "hooks": [{ "type": "command", "command": command }] }]);
                    json!({
                        "hooks": { hook.event(): handlers },
                        // Without it, Claude Code asks a person to approve the command.
                        "permissions": { "allow": [format!("Bash({QUICK})")] },
                    })
                }
            };
            fs::create_dir_all(project.join(".claude"))?;
            fs::write(
                project.join(".claude/settings.local.json"),
                pretty(&settings),
            )
        }
        Agent::Codex => {
            // TOML; a JSON string or array of strings is valid TOML.
            let config = format!(
                "model = {}\nmodel_reasoning_effort = \"low\"\n\n[mcp_servers.{SERVER}]\n\
                 command = {}\nargs = {}\n\
                 # Without it, Codex asks a person to approve each call.\n\
                 default_tools_approval_mode = \"approve\"\n",
                json!(CODEX_MODEL),
                json!(binaries.inbox),
                json!([source]),
            );
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
/// [`CHECK`] file for [`result`].
pub fn prepare(
    work: &Path,
    surface: Surface,
    path: GuiPath,
    binaries: &Binaries,
    version: &str,
    token: &str,
) -> io::Result<Prepared> {
    let project = work.join("project");
    let source = work.join("source");
    write_config(&project, surface, path, binaries, &source)?;
    let message = path.add_message(&Source::open(&source)?, token)?;
    let check = json!({
        "surface": surface.name(),
        "path": path.name(),
        "version": version,
        "msg_id": message.msg_id,
        "token": token,
    });
    fs::write(work.join(CHECK), pretty(&check))?;
    Ok(Prepared {
        project,
        prompt: path.prompt(),
    })
}

/// The outcome of a GUI check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The surface's version line, as [`prepare`] read it.
    pub version: String,
    /// The check's line: the reported token and the message's state.
    pub details: String,
    /// The last line: the verdict.
    pub verdict: String,
    pub pass: bool,
}

/// The outcome of the GUI check prepared in `work`, given the token the developer or reviewer read
/// from the agent's reply: it passes when the token matches and the message is handed over.
pub fn result(work: &Path, reported: &str) -> io::Result<Outcome> {
    let check = read_json(&work.join(CHECK))?;
    let text = |key: &str| {
        check[key]
            .as_str()
            .map(String::from)
            .ok_or_else(|| invalid(format!("{CHECK} has no {key}")))
    };
    let token = text("token")?;
    let state = Source::open(work.join("source"))?.state(&text("msg_id")?)?;
    let matches = reported.trim() == token;
    let pass = matches && handed_over(state);
    let reported = if matches {
        "matches".to_owned()
    } else {
        format!("differs from the check token {token}")
    };
    let message = match state {
        Some(State::HandedOver { .. }) => "handed over".to_owned(),
        other => format!("not handed over (state {other:?})"),
    };
    Ok(Outcome {
        version: text("version")?,
        details: format!("  the reported token {reported}; the message is {message}"),
        verdict: format!(
            "{} on {} with the real model: {}",
            text("path")?,
            text("surface")?,
            verdict(pass)
        ),
        pass,
    })
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
    const PATHS: [GuiPath; 4] = [
        GuiPath::Inbox,
        GuiPath::Channel,
        GuiPath::Hook(Hook::PostToolUse),
        GuiPath::Hook(Hook::Stop),
    ];

    /// The metadata recorded on the test machine, in `tests/surfaces/`, where the surfaces keep
    /// it: the apps' `Info.plist`, the bundled Codex CLIs' `codex-package.json` and VS Code's list
    /// of extensions, each shortened; the Claude app's downloads of Claude Code as empty
    /// directories, with one more that names no version.
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
        let downloads = installed
            .home
            .join("Library/Application Support/Claude/claude-code");
        for version in ["2.1.309", "2.1.317", "2.1.99", "latest"] {
            fs::create_dir_all(downloads.join(version)).unwrap();
        }
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
                "claude-desktop: Claude 4.23118.0, Claude Code 2.1.317, the newest the app downloaded",
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
        let downloads = installed
            .home
            .join("Library/Application Support/Claude/claude-code");
        fs::remove_dir_all(&downloads).unwrap();
        fs::create_dir_all(downloads.join("latest")).unwrap();
        let error = Surface::ClaudeDesktop.version(&installed).unwrap_err();
        assert!(error.to_string().contains("holds no version"), "{error}");
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
        assert_eq!(GuiPath::parse("user-prompt-submit"), None);
        assert_eq!(GuiPath::parse("queue"), None);
    }

    #[test]
    fn runs_inbox_on_each_surface_and_the_rest_on_claude_code() {
        for surface in SURFACES {
            let claude = surface.agent() == Agent::Claude;
            assert_eq!(
                claude,
                matches!(surface, Surface::ClaudeDesktop | Surface::ClaudeVsCode)
            );
            for path in PATHS {
                assert_eq!(
                    path.runs_on(surface),
                    path == GuiPath::Inbox || claude,
                    "{path:?} on {surface:?}"
                );
            }
        }
    }

    fn binaries() -> Binaries {
        Binaries {
            inbox: PathBuf::from("/bin/agent-inbox"),
            channel: PathBuf::from("/bin/agent-channel"),
            hook: PathBuf::from("/bin/agent-hook"),
        }
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
            pretty(&json!({
                "hooks": { event: [{ "hooks": [{
                    "type": "command",
                    "command": "'/bin/agent-hook' '/work/source'",
                }] }] },
                "permissions": { "allow": ["Bash(true)"] },
            }))
        };
        let codex = "model = \"gpt-5.5\"\nmodel_reasoning_effort = \"low\"\n\n\
                     [mcp_servers.skald]\ncommand = \"/bin/agent-inbox\"\n\
                     args = [\"/work/source\"]\n\
                     # Without it, Codex asks a person to approve each call.\n\
                     default_tools_approval_mode = \"approve\"\n";
        for surface in SURFACES {
            for path in PATHS.into_iter().filter(|path| path.runs_on(surface)) {
                let dir = tempfile::tempdir().unwrap();
                let project = dir.path().join("project");
                write_config(&project, surface, path, &binaries(), source).unwrap();
                let expected = match (surface.agent(), path) {
                    (Agent::Codex, _) => vec![(".codex/config.toml", codex.to_owned())],
                    (Agent::Claude, GuiPath::Inbox) => vec![
                        (".mcp.json", server("/bin/agent-inbox")),
                        (
                            ".claude/settings.local.json",
                            pretty(&json!({
                                "enabledMcpjsonServers": ["skald"],
                                "permissions": { "allow": ["mcp__skald__inbox"] },
                            })),
                        ),
                    ],
                    (Agent::Claude, GuiPath::Channel) => vec![
                        (".mcp.json", server("/bin/agent-channel")),
                        (
                            ".claude/settings.local.json",
                            pretty(&json!({ "enabledMcpjsonServers": ["skald"] })),
                        ),
                    ],
                    (Agent::Claude, GuiPath::Hook(Hook::PostToolUse)) => {
                        vec![(".claude/settings.local.json", hook("PostToolUse"))]
                    }
                    (Agent::Claude, GuiPath::Hook(_)) => {
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
    fn prompts_as_the_cli_checks() {
        assert_eq!(GuiPath::Inbox.prompt(), Kind::Idle.prompt());
        assert_eq!(
            GuiPath::Hook(Hook::PostToolUse).prompt(),
            Hook::PostToolUse.prompt(Kind::Idle)
        );
        assert_eq!(
            GuiPath::Hook(Hook::Stop).prompt(),
            Hook::Stop.prompt(Kind::Idle)
        );
        assert_eq!(GuiPath::Channel.prompt(), Hook::Stop.prompt(Kind::Idle));
    }

    /// Prepares `path`'s check on `surface` in `work` with `token`.
    fn prepared(work: &Path, surface: Surface, path: GuiPath, token: &str) -> Prepared {
        prepare(work, surface, path, &binaries(), "a version line", token).unwrap()
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
            let prepared = prepared(work.path(), surface, path, &token);
            assert_eq!(prepared.project, work.path().join("project"));
            assert_eq!(prepared.prompt, path.prompt());
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
        }
    }

    #[test]
    fn passes_only_when_the_reported_token_matches_and_the_message_is_handed_over() {
        let work = tempfile::tempdir().unwrap();
        let token = token();
        prepared(work.path(), Surface::ClaudeVsCode, GuiPath::Inbox, &token);

        let waiting = result(work.path(), &token).unwrap();
        assert!(!waiting.pass);
        assert_eq!(
            waiting.details,
            "  the reported token matches; the message is not handed over (state Some(Waiting))"
        );
        assert_eq!(waiting.version, "a version line");
        assert_eq!(
            waiting.verdict,
            "inbox on claude-vscode with the real model: fail"
        );

        hand_over(work.path());
        let wrong = result(work.path(), "NONE").unwrap();
        assert!(!wrong.pass);
        assert_eq!(
            wrong.details,
            format!(
                "  the reported token differs from the check token {token}; the message is \
                 handed over"
            )
        );

        let read = result(work.path(), &format!(" {token}\n")).unwrap();
        assert!(read.pass);
        assert_eq!(
            read.details,
            "  the reported token matches; the message is handed over"
        );
        assert_eq!(
            read.verdict,
            "inbox on claude-vscode with the real model: pass"
        );
    }

    #[test]
    fn result_fails_without_a_prepared_check() {
        let work = tempfile::tempdir().unwrap();
        assert!(result(work.path(), "tok").is_err());
        fs::write(work.path().join(CHECK), "{}").unwrap();
        let error = result(work.path(), "tok").unwrap_err();
        assert!(error.to_string().contains("has no token"), "{error}");
    }
}
