//! The Codex live checks of "Checks before implementation" in `docs/dev/agent-delivery.md`: what a
//! signal over `codex queue` makes the model do, and what an executor session, `codex exec` with no
//! person, does in each sandbox mode and loads. The binary's `codex_before_run` starts the CLI;
//! everything else is here.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::before::{Before, Folder, Mode, SIGNAL, make_executable, signal_tasks, skill};
use crate::channel::{Files, TERM};
use crate::{
    Agent, Call, Checks, Cue, Kind, Options, PLACEHOLDER, SERVER, STUB_KEY, StubLog,
    codex_stub_provider, model_flag, pretty, queue,
};

/// Whether `check` runs on Codex with `options`: the signal with any; an executor check runs one
/// session on the model stub, so takes neither `--all` nor `--real`; `own-text` does not run.
pub fn takes(check: Before, options: &Options) -> bool {
    match check {
        Before::Signal => true,
        Before::OwnText => false,
        Before::ReadOnly | Before::Full | Before::Clean => !options.all && !options.real,
    }
}

/// The `-c` values that attach the `inbox` server, `inbox <source> --claim <claims>`, to a
/// session, its tools approved, as no person answers.
pub fn signal_servers(inbox: &Path, source: &Path, claims: &Path) -> Vec<String> {
    let server = format!("mcp_servers.{SERVER}");
    vec![
        format!("{server}.command={}", json!(inbox)),
        format!(
            "{server}.args={}",
            json!([source, Path::new("--claim"), claims])
        ),
        format!("{server}.default_tools_approval_mode=\"approve\""),
    ]
}

/// The task the stub `claim` of a signal session hands out: reply with the tokens read.
pub fn signal_task() -> String {
    let [_, reply] = signal_tasks();
    reply
}

/// Writes the one task of a signal session into `claims` and returns the arguments of its terminal
/// UI session of `kind`: those of the `codex queue` check's ([`queue::session`]) with the `inbox`
/// server on `inbox`, the session's source and `claims`.
pub fn signal_session(
    files: &Files,
    kind: Kind,
    inbox: &Path,
    claims: &Path,
    model: Option<&str>,
    stub: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(&files.project)?;
    fs::create_dir_all(claims)?;
    fs::write(claims.join("task-1"), signal_task())?;
    let mut args = queue::session(files, kind, model, stub);
    let prompt = args.pop();
    args.extend(
        signal_servers(inbox, &files.source, claims)
            .into_iter()
            .flat_map(|value| ["-c".to_owned(), value]),
    );
    args.extend(prompt);
    Ok(args)
}

/// What the model stub calls once the signal reached it.
const SIGNALLED: &[Call] = &[Call::Inbox, Call::Claim];

/// The model stub's script of a signal session of `kind`: its prompt's turn, as in the
/// `codex queue` check; from the signal on, `inbox` and `claim`.
pub fn signal_script(kind: Kind) -> (&'static [Call], Cue) {
    (
        queue::script(kind),
        Cue {
            text: SIGNAL,
            then: SIGNALLED,
        },
    )
}

/// The arguments of `codex queue` that queue `text` for the session `thread`.
pub fn queue_args(thread: &str, text: &str) -> [String; 5] {
    ["queue", "--thread", thread, "--message", text].map(String::from)
}

/// The variables a session with `--real` gets on top of `live-check`'s own: the config home
/// `config`, where the check signs it in, and the terminal.
pub fn real_environment(config: &Path) -> Vec<(String, String)> {
    vec![
        (
            "CODEX_HOME".to_owned(),
            config.to_string_lossy().into_owned(),
        ),
        ("TERM".to_owned(), TERM.to_owned()),
    ]
}

/// The variable whose key signs Codex in with `--real`.
pub const REAL_KEY: &str = "OPENAI_API_KEY";

/// The arguments of `codex login` that read the key from stdin.
pub const LOGIN: [&str; 2] = ["login", "--with-api-key"];

/// What a case of the signal check saw.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Signalled {
    /// Whether `codex queue` took the signal.
    pub queued: bool,
    /// Whether the signal reached the model ([`signal_reached`]).
    pub reached: bool,
    /// Whether `inbox` handed the message over.
    pub handed_over: bool,
    /// How many times the stub `claim` was called.
    pub claims: usize,
    /// Whether the message's token reached the model.
    pub token: bool,
}

/// Whether the signal reached the model: on `stub`, if the session ran on one, a request held it;
/// on the real model, `codex queue` took it.
pub fn signal_reached(stub: Option<&StubLog>, queued: bool) -> bool {
    match stub {
        Some(stub) => stub.saw(SIGNAL),
        None => queued,
    }
}

/// The Codex sandbox of an executor in `mode`.
pub fn sandbox(mode: Mode) -> &'static str {
    match mode {
        Mode::ReadOnly => "read-only",
        Mode::Full => "danger-full-access",
    }
}

/// What an executor's turn tries, each with a shell command, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    ReadInside,
    WriteInside,
    WriteOutside,
    Command,
    LocalServer,
    PublicHost,
    ReadOutside,
}

impl Attempt {
    pub const ALL: [Self; 7] = [
        Self::ReadInside,
        Self::WriteInside,
        Self::WriteOutside,
        Self::Command,
        Self::LocalServer,
        Self::PublicHost,
        Self::ReadOutside,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::ReadInside => "read a file in the folder",
            Self::WriteInside => "write a file in the folder",
            Self::WriteOutside => "write a file outside it",
            Self::Command => "run a command",
            Self::LocalServer => "reach the check's server",
            Self::PublicHost => "reach a public host",
            Self::ReadOutside => "read a file outside the folder",
        }
    }
}

/// The public host an executor tries to reach.
pub const PUBLIC: &str = "https://example.com/";

/// What the command of [`Attempt::Command`] prints, and of [`Attempt::PublicHost`] once it reached
/// the host: neither is in the command itself, so either reaches the model only if the command ran.
pub const RAN: &str = "ran-42";
pub const REACHED: &str = "public-2";

/// An executor's folder, with a file of its own, and what lies outside it; and the check's server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executor {
    pub folder: Folder,
    /// The token in `folder/inside.txt`.
    pub inside: String,
    /// The check's local server.
    pub url: String,
}

impl Executor {
    /// Makes the folder and what lies outside it in `dir` ([`Folder::make`]), with `inside.txt`.
    pub fn make(dir: &Path, url: &str) -> io::Result<Self> {
        let folder = Folder::make(dir)?;
        let inside = crate::token();
        fs::write(folder.folder.join("inside.txt"), format!("{inside}\n"))?;
        Ok(Self {
            folder,
            inside,
            url: url.to_owned(),
        })
    }

    /// The file [`Attempt::WriteOutside`] writes.
    pub fn written_outside(&self) -> PathBuf {
        self.folder.outside.join("written.txt")
    }

    /// The shell command of `attempt`.
    pub fn command(&self, attempt: Attempt) -> String {
        let read = |path: &Path| format!("{} {}", shell::READ, shell::quote(path));
        let write = |path: &Path| format!("{} {}", shell::WRITE, shell::quote(path));
        match attempt {
            Attempt::ReadInside => read(&self.folder.folder.join("inside.txt")),
            Attempt::WriteInside => write(&self.folder.written()),
            Attempt::WriteOutside => write(&self.written_outside()),
            Attempt::Command => shell::RAN.to_owned(),
            Attempt::LocalServer => format!("{} -s -m 5 {}", shell::CURL, self.url),
            Attempt::PublicHost => format!(
                "{} -s -m 10 -o {} {PUBLIC}{}",
                shell::CURL,
                shell::NULL,
                shell::THEN_REACHED
            ),
            Attempt::ReadOutside => read(&self.folder.outside.join("secret.txt")),
        }
    }

    /// The executor's prompt: each attempt's command in order.
    pub fn prompt(&self) -> String {
        commands_prompt(&Attempt::ALL.map(|attempt| self.command(attempt)))
    }

    /// The model stub's script: each attempt's command in order.
    pub fn script(&self) -> &'static [Call] {
        commands_script(&Attempt::ALL.map(|attempt| self.command(attempt)))
    }

    /// What the executor did, from the folder, the requests the check's server got, `fetched`, and
    /// what reached the model, on its `stub`: each attempt that succeeded.
    pub fn outcomes(&self, stub: Option<&StubLog>, fetched: usize) -> Vec<Attempt> {
        let saw = |text: &str| stub.is_some_and(|stub| stub.saw(text));
        Attempt::ALL
            .into_iter()
            .filter(|attempt| match attempt {
                Attempt::ReadInside => saw(&self.inside),
                Attempt::WriteInside => {
                    fs::read_to_string(self.folder.written()).is_ok_and(|text| text == "written")
                }
                Attempt::WriteOutside => self.written_outside().exists(),
                Attempt::Command => saw(RAN),
                Attempt::LocalServer => fetched > 0,
                Attempt::PublicHost => saw(REACHED),
                Attempt::ReadOutside => saw(&self.folder.secret),
            })
            .collect()
    }
}

/// A prompt that asks for each of `commands` in order.
fn commands_prompt(commands: &[String]) -> String {
    let mut prompt =
        "Run each of these shell commands in order, and go on to the next one if one fails:\n"
            .to_owned();
    for (n, command) in commands.iter().enumerate() {
        prompt.push_str(&format!("{}. `{command}`\n", n + 1));
    }
    prompt.push_str("Then reply with one short line per command saying what happened.");
    prompt
}

/// The model stub's script that runs each of `commands` in order with Codex's shell tool.
fn commands_script(commands: &[String]) -> &'static [Call] {
    Call::script(
        commands
            .iter()
            .map(|command| {
                // Codex waits for the command up to `yield_time_ms`, 10 s by default.
                Call::tool(
                    "exec_command",
                    &json!({ "cmd": command, "yield_time_ms": 30_000 }),
                )
            })
            .collect(),
    )
}

/// The flags that leave an executor none of its config home's MCP servers, hooks, plugins, rules
/// and skills: no `config.toml`, so none of its MCP servers, hooks, plugins and trust; no rules;
/// neither the plugins nor the hooks feature; no skills' instructions.
pub const CLEAN_FLAGS: [&str; 8] = [
    "--ignore-user-config",
    "--ignore-rules",
    "--disable",
    "plugins",
    "--disable",
    "hooks",
    "-c",
    "skills.include_instructions=false",
];

/// A session of the executor checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    /// The `read-only` or full-access executor: the [`CLEAN_FLAGS`], signed in with the stub's key
    /// in its environment.
    Sandbox,
    /// The clean check's control: none of the flags, its hooks run without the trust a person gives
    /// them, and it gets [`PARENT_VAR`].
    Control,
    /// The clean check's session with only the [`CLEAN_FLAGS`].
    Flags,
    /// The clean check's session with the flags and a config home of its own that holds only the
    /// sign-in, [`AUTH`].
    Clean,
}

impl Setup {
    /// The clean check's sessions.
    pub const CLEAN: [Self; 3] = [Self::Control, Self::Flags, Self::Clean];

    pub fn name(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Control => "control",
            Self::Flags => "flags",
            Self::Clean => "clean",
        }
    }

    /// Whether the session signs in from [`AUTH`] in its config home, as a person's sign-in does,
    /// rather than with the stub's key in its environment.
    pub fn signs_in_from_home(self) -> bool {
        self != Self::Sandbox
    }
}

/// The sign-in file of a config home; the clean check's holds the stub's placeholder key.
pub const AUTH: &str = "auth.json";

/// The `-c` values of the model stub at `stub` when the session signs in from [`AUTH`].
fn signed_in_provider(stub: &str) -> Vec<String> {
    let mut provider: Vec<String> = codex_stub_provider(stub)
        .into_iter()
        .filter(|value| !value.contains(".env_key="))
        .collect();
    provider.push("model_providers.stub.requires_openai_auth=true".to_owned());
    provider
}

/// The arguments of an executor session `setup` in `mode` in `folder` with `prompt`, on the model
/// stub at `stub` if given: no person, its events as JSON lines, no session saved, no approval
/// asked, the lowest effort, and the [`CLEAN_FLAGS`] unless it is the control.
pub fn executor_args(
    mode: Mode,
    folder: &Path,
    prompt: &str,
    setup: Setup,
    model: Option<&str>,
    stub: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = ["exec", "--json", "--skip-git-repo-check", "--ephemeral"]
        .map(String::from)
        .to_vec();
    if setup == Setup::Control {
        args.push("--dangerously-bypass-hook-trust".to_owned());
    } else {
        args.extend(CLEAN_FLAGS.map(String::from));
    }
    args.extend([
        "--sandbox".to_owned(),
        sandbox(mode).to_owned(),
        "-C".to_owned(),
        folder.to_string_lossy().into_owned(),
    ]);
    let mut config = vec![
        "approval_policy=\"never\"".to_owned(),
        "model_reasoning_effort=\"low\"".to_owned(),
    ];
    config.extend(
        stub.map(|stub| {
            if setup.signs_in_from_home() {
                signed_in_provider(stub)
            } else {
                codex_stub_provider(stub)
            }
        })
        .unwrap_or_default(),
    );
    args.extend(
        config
            .into_iter()
            .flat_map(|value| ["-c".to_owned(), value]),
    );
    args.extend(model_flag("-m", model));
    args.push(prompt.to_owned());
    args
}

/// The variable the clean check's control session gets, which shows whether a variable of an
/// executor's environment reaches its commands.
pub const PARENT_VAR: &str = crate::before::PARENT_VAR;

/// The whole environment of a session `setup`: that of the model stub, `stub`
/// ([`Agent::environment`]), without the stub's key if it signs in from its config home; for the
/// control, `parent` as [`PARENT_VAR`]; for the clean session, `CODEX_HOME` in `own`.
pub fn executor_environment(
    stub: Vec<(String, String)>,
    setup: Setup,
    parent: &str,
    own: &Path,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = stub
        .into_iter()
        .filter(|(name, _)| !(setup.signs_in_from_home() && name == STUB_KEY))
        .map(|(name, value)| match (setup, name.as_str()) {
            (Setup::Clean, "CODEX_HOME") => (name, own.to_string_lossy().into_owned()),
            _ => (name, value),
        })
        .collect();
    if setup == Setup::Control {
        env.push((PARENT_VAR.to_owned(), parent.to_owned()));
    }
    env
}

/// Deletes the sign-in, [`AUTH`], from the config home `config`, so that a work directory kept
/// for its logs holds no key; nothing if there is none.
pub fn forget_sign_in(config: &Path) -> io::Result<()> {
    match fs::remove_file(config.join(AUTH)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}

/// Makes `own` a config home that holds only the sign-in of the config home `config`.
pub fn sign_in_only(config: &Path, own: &Path) -> io::Result<()> {
    fs::create_dir_all(own)?;
    fs::copy(config.join(AUTH), own.join(AUTH)).map(|_| ())
}

/// What the clean check plants in a config home, each piece with what shows it reached a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planted {
    /// The token in the skill's description.
    pub skill: String,
    /// The token in the plugin's skill's description.
    pub plugin_skill: String,
    /// The token in `AGENTS.md`.
    pub instructions: String,
    /// Where the planted MCP server and hooks leave a file named after themselves when they run.
    pub marks: PathBuf,
    /// The file the rule forbids the session to make.
    pub ruled: PathBuf,
}

/// The files the planted MCP server, hook and plugin hook leave in [`Planted::marks`].
pub const MARKS: [&str; 3] = ["mcp-server", "hook", "plugin-hook"];

/// The other planted pieces, as [`Reached::planted`] names them.
pub const PIECES: [&str; 4] = ["rule", "skill", "plugin skill", "AGENTS.md"];

/// The plugin's ID, `<plugin>@<marketplace>`, as `config.toml` names it.
pub const PLUGIN: &str = "planted@planted";

/// The hooks of `hooks.json`, of the config home or of a plugin, that run `command` on each
/// event the clean check's session goes through.
fn hooks(command: &str) -> Value {
    let hook = json!([{ "hooks": [{ "type": "command", "command": command }] }]);
    json!({ "hooks": {
        "SessionStart": hook, "UserPromptSubmit": hook, "PreToolUse": hook, "Stop": hook,
    } })
}

/// Plants, in the config home `config`: the sign-in, [`AUTH`], with the stub's placeholder key; an
/// MCP server, a local marketplace in `marketplace` and its plugin, enabled, with a skill and a
/// hook, in `config.toml`; a hook in `hooks.json`; a rule that forbids making [`Planted::ruled`]; a
/// skill; and `AGENTS.md`. The MCP server and the hooks run `mark <name>`, which leaves a file in
/// `marks`.
pub fn plant(config: &Path, marks: &Path, marketplace: &Path) -> io::Result<Planted> {
    let planted = Planted {
        skill: crate::token(),
        plugin_skill: crate::token(),
        instructions: crate::token(),
        marks: marks.to_owned(),
        ruled: marks.join("ruled"),
    };
    fs::create_dir_all(marks)?;
    fs::create_dir_all(config)?;
    fs::write(
        config.join(AUTH),
        pretty(&json!({ "auth_mode": "apikey", "OPENAI_API_KEY": PLACEHOLDER })),
    )?;
    let mark = config.join(shell::MARK);
    fs::write(&mark, shell::mark_script(marks))?;
    make_executable(&mark)?;
    let command = |name: &str| format!("{} {name}", shell::program(&mark));

    // A `-c` value and a `config.toml` value are TOML; a JSON string is a valid TOML string.
    fs::write(
        config.join("config.toml"),
        format!(
            "[mcp_servers.planted]\ncommand = {}\nargs = [\"{}\"]\n\n\
             [marketplaces.planted]\nsource_type = \"local\"\nsource = {}\n\n\
             [plugins.\"{PLUGIN}\"]\nenabled = true\n",
            json!(mark),
            MARKS[0],
            json!(marketplace),
        ),
    )?;
    fs::write(
        config.join("hooks.json"),
        pretty(&hooks(&command(MARKS[1]))),
    )?;

    let source = marketplace.join("plugins/planted");
    let install = config.join("plugins/cache/planted/planted/1.0.0");
    for plugin in [&source, &install] {
        fs::create_dir_all(plugin.join(".codex-plugin"))?;
        fs::write(
            plugin.join(".codex-plugin/plugin.json"),
            pretty(&json!({
                "name": "planted", "version": "1.0.0", "description": "Planted.",
                "skills": "./skills/", "hooks": "./hooks/hooks.json",
            })),
        )?;
        fs::create_dir_all(plugin.join("hooks"))?;
        fs::write(
            plugin.join("hooks/hooks.json"),
            pretty(&hooks(&command(MARKS[2]))),
        )?;
        skill(
            &plugin.join("skills/planted-plugin"),
            "planted-plugin",
            &planted.plugin_skill,
        )?;
    }
    fs::create_dir_all(marketplace.join(".agents/plugins"))?;
    fs::write(
        marketplace.join(".agents/plugins/marketplace.json"),
        pretty(&json!({
            "name": "planted",
            "plugins": [{
                "name": "planted",
                "source": { "source": "local", "path": "./plugins/planted" },
                "policy": { "installation": "AVAILABLE", "authentication": "ON_USE" },
                "category": "Productivity",
            }],
        })),
    )?;

    fs::create_dir_all(config.join("rules"))?;
    fs::write(
        config.join("rules/planted.rules"),
        format!(
            "prefix_rule(pattern=[{}], decision=\"forbidden\")\n",
            ruled_pattern(&planted.ruled)
        ),
    )?;
    skill(&config.join("skills/planted"), "planted", &planted.skill)?;
    fs::write(
        config.join("AGENTS.md"),
        format!(
            "# Planted instructions\n\nThe instructions' token is {}.\n",
            planted.instructions
        ),
    )?;
    Ok(planted)
}

/// The commands of the clean check's session: make the file the rule forbids, then print
/// [`PARENT_VAR`].
pub fn clean_commands(planted: &Planted) -> [String; 2] {
    [
        format!(
            "{} {}",
            shell::MAKE_FILE.join(" "),
            shell::quote(&planted.ruled)
        ),
        shell::print_parent(PARENT_VAR),
    ]
}

/// The words of the command the rule forbids, that which makes `ruled`, as Starlark strings.
fn ruled_pattern(ruled: &Path) -> String {
    let mut words: Vec<String> = shell::MAKE_FILE
        .map(|word| json!(word).to_string())
        .to_vec();
    words.push(json!(ruled).to_string());
    words.join(", ")
}

/// What the executor checks' commands are written in: POSIX `sh`, in which Codex runs commands on
/// macOS and Linux.
#[cfg(not(windows))]
mod shell {
    use std::path::Path;

    use crate::shell_quote;

    pub const READ: &str = "cat";
    pub const WRITE: &str = "printf written >";
    /// Prints [`super::RAN`].
    pub const RAN: &str = "echo ran-$((20+22))";
    pub const CURL: &str = "curl";
    pub const NULL: &str = "/dev/null";
    /// Prints [`super::REACHED`] if `curl` reached the host.
    pub const THEN_REACHED: &str = " && echo public-$((1+1))";
    /// The words of a command that makes a file, before its path.
    pub const MAKE_FILE: [&str; 1] = ["touch"];
    /// The script the planted MCP server and hooks run.
    pub const MARK: &str = "mark.sh";

    pub fn quote(path: &Path) -> String {
        shell_quote(&path.to_string_lossy())
    }

    /// `program` as the first word of a command.
    pub fn program(program: &Path) -> String {
        quote(program)
    }

    /// A command that prints `parent: ` and the variable `var`, or `unset`.
    pub fn print_parent(var: &str) -> String {
        format!("echo \"parent: ${{{var}:-unset}}\"")
    }

    /// The [`MARK`] script: it makes the file named by its argument in `marks`.
    pub fn mark_script(marks: &Path) -> String {
        format!("#!/bin/sh\ntouch {}/\"$1\"\n", quote(marks))
    }
}

/// On Windows, PowerShell, in which Codex runs commands there.
#[cfg(windows)]
mod shell {
    use std::path::Path;

    pub const READ: &str = "Get-Content -Raw";
    pub const WRITE: &str = "Set-Content -NoNewline -Value written -Path";
    pub const RAN: &str = "Write-Output \"ran-$(20+22)\"";
    pub const CURL: &str = "curl.exe";
    pub const NULL: &str = "NUL";
    pub const THEN_REACHED: &str = "; if ($LASTEXITCODE -eq 0) { Write-Output \"public-$(1+1)\" }";
    pub const MAKE_FILE: [&str; 4] = ["New-Item", "-ItemType", "File", "-Path"];
    pub const MARK: &str = "mark.cmd";

    pub fn quote(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', "''"))
    }

    /// `program` as the first word of a command that `cmd` or PowerShell runs: a path with a space
    /// in it would need each one's own quoting.
    pub fn program(program: &Path) -> String {
        program.to_string_lossy().into_owned()
    }

    pub fn print_parent(var: &str) -> String {
        format!("Write-Output \"parent: $(if ($env:{var}) {{ $env:{var} }} else {{ 'unset' }})\"")
    }

    pub fn mark_script(marks: &Path) -> String {
        format!("@echo off\r\ntype nul > \"{}\\%1\"\r\n", marks.display())
    }
}

/// The clean check's prompt: its [`clean_commands`].
pub fn clean_prompt(planted: &Planted) -> String {
    commands_prompt(&clean_commands(planted))
}

/// The model stub's script of the clean check: its [`clean_commands`].
pub fn clean_script(planted: &Planted) -> &'static [Call] {
    commands_script(&clean_commands(planted))
}

/// What reached a session of the clean check from the planted config home and its environment.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reached {
    /// Each planted piece the session loaded, ran or got: a name of [`MARKS`] or [`PIECES`].
    pub planted: Vec<&'static str>,
    /// Whether [`PARENT_VAR`] reached the session's command.
    pub parent: bool,
    /// Whether the session ran the command that prints it.
    pub ran: bool,
}

impl Reached {
    /// What reached the session: the marks the planted pieces left; the rule, if the session ran
    /// its commands and the file the rule forbids is missing; the planted tokens in the requests
    /// to the `stub`; and `parent`, the variable's value, in the command's output there.
    pub fn of(planted: &Planted, stub: Option<&StubLog>, parent: &str) -> Self {
        let saw = |text: &str| stub.is_some_and(|stub| stub.saw(text));
        let parent = saw(&format!("parent: {parent}"));
        let ran = parent || saw("parent: unset");
        let mut reached = Self {
            planted: Vec::new(),
            parent,
            ran,
        };
        for mark in MARKS {
            if planted.marks.join(mark).exists() {
                reached.planted.push(mark);
            }
        }
        for (piece, here) in PIECES.into_iter().zip([
            ran && !planted.ruled.exists(),
            saw(&planted.skill),
            saw(&planted.plugin_skill),
            saw(&planted.instructions),
        ]) {
            if here {
                reached.planted.push(piece);
            }
        }
        reached
    }
}

/// The Codex checks of one command: each returns its line in the output.
#[derive(Debug)]
pub struct CodexCheck {
    check: Before,
    checks: Checks,
}

impl CodexCheck {
    pub fn new(check: Before) -> Self {
        Self {
            check,
            checks: Checks::default(),
        }
    }

    /// A case of the signal check: `codex queue` took the signal, it reached the model, the agent
    /// called `inbox`, which handed the message over, and `claim`, and the message's token reached
    /// the model, only through `inbox`, as the signal holds none. `None` if no signal went.
    pub fn signal(&mut self, case: &str, seen: Option<&Signalled>, details: &str) -> String {
        let Some(seen) = seen else {
            return self
                .checks
                .report(case, false, &format!("no signal went; {details}"));
        };
        let pass = seen.queued && seen.reached && seen.handed_over && seen.claims > 0 && seen.token;
        let yes = |done: bool| if done { "yes" } else { "no" };
        let details = format!(
            "codex queue took the signal: {}; it reached the model: {}; inbox handed the message \
             over: {}; claim calls: {}; the token reached the model: {}; {details}",
            yes(seen.queued),
            yes(seen.reached),
            yes(seen.handed_over),
            seen.claims,
            yes(seen.token),
        );
        self.checks.report(case, pass, &details)
    }

    /// An executor in `mode` and the attempts that succeeded, `done`: in `read-only`, only reading
    /// in its folder; with full access, also writing, running a command, reaching the check's
    /// server and reading outside the folder. Reaching a public host depends on the machine's
    /// network, so it only shows.
    pub fn attempts(&mut self, mode: Mode, done: &[Attempt], details: &str) -> String {
        let needed: &[Attempt] = match mode {
            Mode::ReadOnly => &[Attempt::ReadInside],
            Mode::Full => &[
                Attempt::ReadInside,
                Attempt::WriteInside,
                Attempt::WriteOutside,
                Attempt::Command,
                Attempt::LocalServer,
                Attempt::ReadOutside,
            ],
        };
        let pass = match mode {
            Mode::ReadOnly => done == needed,
            Mode::Full => needed.iter().all(|attempt| done.contains(attempt)),
        };
        let outcomes: Vec<String> = Attempt::ALL
            .iter()
            .map(|attempt| {
                let did = if done.contains(attempt) { "yes" } else { "no" };
                format!("{}: {did}", attempt.name())
            })
            .collect();
        let case = format!("{} executor", sandbox(mode));
        self.checks
            .report(&case, pass, &format!("{}; {details}", outcomes.join("; ")))
    }

    /// The session with only the clean flags: it signed in and ran its commands; what reached it
    /// only shows.
    pub fn flags(&mut self, reached: &Reached, details: &str) -> String {
        let details = format!("{}; {details}", reached_summary(reached));
        self.checks
            .report("with only the clean flags", reached.ran, &details)
    }

    /// The clean executor: it signed in and ran its commands, and nothing planted and not the
    /// variable reached it.
    pub fn clean(&mut self, reached: &Reached, details: &str) -> String {
        let pass = reached.ran && reached.planted.is_empty() && !reached.parent;
        let details = format!("{}; {details}", reached_summary(reached));
        self.checks.report(
            "clean executor, in a config home of its own",
            pass,
            &details,
        )
    }

    /// The line of the clean check's session `setup`.
    pub fn session(&mut self, setup: Setup, reached: &Reached, details: &str) -> String {
        match setup {
            Setup::Control => self.control(reached, details),
            Setup::Flags => self.flags(reached, details),
            Setup::Clean | Setup::Sandbox => self.clean(reached, details),
        }
    }

    /// The control session, without the clean flags and with the variable: everything planted and
    /// the variable reached it, which shows that each sits where Codex reads it.
    pub fn control(&mut self, reached: &Reached, details: &str) -> String {
        let missing: Vec<&str> = MARKS
            .into_iter()
            .chain(PIECES)
            .filter(|piece| !reached.planted.contains(piece))
            .collect();
        let pass = reached.parent && missing.is_empty();
        let details = format!(
            "{}; did not: {missing:?}; {details}",
            reached_summary(reached)
        );
        self.checks
            .report("control, without the clean flags", pass, &details)
    }

    /// The last line of the output, with `options`' model, and whether every check passed.
    pub fn ended(&self, options: &Options) -> (String, bool) {
        let line = self
            .checks
            .verdict(self.check.name(), Agent::Codex, options);
        (line, self.checks.pass())
    }
}

/// What reached a session of the clean check, for its line.
fn reached_summary(reached: &Reached) -> String {
    format!(
        "planted that reached it: {:?}; the variable {}; ran its commands: {}",
        reached.planted,
        if reached.parent {
            "reached it"
        } else {
            "did not reach it"
        },
        if reached.ran { "yes" } else { "no" },
    )
}

#[cfg(test)]
mod tests {
    use crate::shell_quote;
    use crate::tests::read_json;

    use super::*;

    /// Whether every check of `check` passed.
    fn passed(check: &CodexCheck) -> bool {
        check.ended(&Options::default()).1
    }

    fn options(all: bool, real: bool) -> Options<'static> {
        Options {
            all,
            real,
            ..Options::default()
        }
    }

    #[test]
    fn each_check_takes_its_options() {
        for (all, real) in [(false, false), (true, false), (false, true), (true, true)] {
            assert!(takes(Before::Signal, &options(all, real)));
            assert!(!takes(Before::OwnText, &options(all, real)));
            for check in [Before::ReadOnly, Before::Full, Before::Clean] {
                assert_eq!(
                    takes(check, &options(all, real)),
                    !all && !real,
                    "{check:?}"
                );
            }
        }
    }

    #[test]
    fn a_signal_session_has_inbox_and_claim_and_one_task() {
        let tmp = tempfile::tempdir().unwrap();
        let files = Files::new(tmp.path());
        let claims = tmp.path().join("claims");
        let inbox = Path::new("/bin/agent-inbox");
        let args =
            signal_session(&files, Kind::Busy, inbox, &claims, None, Some("http://s")).unwrap();
        let queue = queue::session(&files, Kind::Busy, None, Some("http://s"));
        // The `codex queue` check's session, with the server before the prompt.
        assert_eq!(args[..queue.len() - 1], queue[..queue.len() - 1]);
        assert_eq!(args.last(), queue.last());
        let added: Vec<&str> = args[queue.len() - 1..args.len() - 1]
            .iter()
            .map(String::as_str)
            .collect();
        let source = files.source.to_string_lossy();
        let claims_dir = claims.to_string_lossy();
        assert_eq!(
            added,
            [
                "-c",
                r#"mcp_servers.skald.command="/bin/agent-inbox""#,
                "-c",
                &format!(r#"mcp_servers.skald.args=["{source}","--claim","{claims_dir}"]"#),
                "-c",
                r#"mcp_servers.skald.default_tools_approval_mode="approve""#,
            ]
        );
        assert!(files.project.is_dir());
        assert_eq!(
            fs::read_to_string(claims.join("task-1")).unwrap(),
            signal_task()
        );
        assert!(signal_task().contains("check tokens"));
    }

    #[test]
    fn the_stub_calls_inbox_and_claim_once_the_signal_comes() {
        for kind in [Kind::Idle, Kind::Busy] {
            let (script, cue) = signal_script(kind);
            assert_eq!(script, queue::script(kind));
            assert_eq!(cue.text, SIGNAL);
            assert_eq!(cue.then, [Call::Inbox, Call::Claim]);
        }
        // The signal says messages and a task wait, and names neither a sender nor a token.
        assert!(SIGNAL.contains("messages") && SIGNAL.contains("task"));
        assert!(!SIGNAL.contains(crate::SENDER) && !SIGNAL.contains("tok-"));
        assert_eq!(
            queue_args("019a", SIGNAL),
            ["queue", "--thread", "019a", "--message", SIGNAL]
        );
        let stub = StubLog {
            requests: vec![format!("x {SIGNAL} y")],
            credentials: Vec::new(),
        };
        assert!(signal_reached(Some(&stub), false));
        assert!(!signal_reached(Some(&StubLog::default()), true));
        assert!(signal_reached(None, true));
        assert!(!signal_reached(None, false));
    }

    #[test]
    fn real_sessions_sign_in_in_a_config_home_of_their_own() {
        assert_eq!(
            real_environment(Path::new("/w/a/home/.codex")),
            [
                ("CODEX_HOME".to_owned(), "/w/a/home/.codex".to_owned()),
                ("TERM".to_owned(), TERM.to_owned()),
            ]
        );
        assert_eq!(LOGIN, ["login", "--with-api-key"]);
        assert!(Agent::Codex.replaces_sign_in(REAL_KEY));
    }

    #[test]
    fn an_executor_runs_in_its_sandbox_and_clean() {
        let folder = Path::new("/w/folder");
        let window = |args: &[String], part: &[&str]| args.windows(part.len()).any(|w| w == part);
        let read_only = executor_args(
            Mode::ReadOnly,
            folder,
            "Do it.",
            Setup::Sandbox,
            None,
            Some("http://s"),
        );
        assert_eq!(
            read_only[..4],
            ["exec", "--json", "--skip-git-repo-check", "--ephemeral"]
        );
        assert!(window(&read_only, &CLEAN_FLAGS));
        assert!(window(
            &read_only,
            &["--sandbox", "read-only", "-C", "/w/folder"]
        ));
        assert!(window(&read_only, &["-c", "approval_policy=\"never\""]));
        assert!(window(&read_only, &["-c", "model_provider=\"stub\""]));
        assert!(window(
            &read_only,
            &["-c", "model_providers.stub.env_key=\"SKALD_STUB_API_KEY\""]
        ));
        assert!(!read_only.contains(&"--dangerously-bypass-hook-trust".to_owned()));
        assert_eq!(read_only.last().unwrap(), "Do it.");

        let full = executor_args(
            Mode::Full,
            folder,
            "Do it.",
            Setup::Sandbox,
            Some("m"),
            None,
        );
        assert!(window(&full, &["--sandbox", "danger-full-access"]));
        assert!(window(&full, &["-m", "m", "Do it."]));
        assert!(!full.iter().any(|arg| arg.contains("model_provider")));

        let control = executor_args(
            Mode::Full,
            folder,
            "Do it.",
            Setup::Control,
            None,
            Some("http://s"),
        );
        assert!(control.contains(&"--dangerously-bypass-hook-trust".to_owned()));
        for flag in CLEAN_FLAGS.into_iter().filter(|flag| *flag != "-c") {
            assert!(!control.contains(&flag.to_owned()), "{flag}");
        }
        // The clean check's sessions sign in from their config home, not with the stub's key.
        for setup in Setup::CLEAN {
            let args = executor_args(Mode::Full, folder, "Do it.", setup, None, Some("http://s"));
            assert!(window(
                &args,
                &["-c", "model_providers.stub.requires_openai_auth=true"]
            ));
            assert!(!args.iter().any(|arg| arg.contains("env_key")), "{args:?}");
            assert_eq!(
                window(&args, &CLEAN_FLAGS),
                setup != Setup::Control,
                "{setup:?}"
            );
        }

        let pair = |name: &str, value: &str| (name.to_owned(), value.to_owned());
        let stub = vec![
            pair("PATH", "/bin"),
            pair("CODEX_HOME", "/h/.codex"),
            pair("SKALD_STUB_API_KEY", PLACEHOLDER),
        ];
        let own = Path::new("/own/.codex");
        let env = |setup| executor_environment(stub.clone(), setup, "tok-1", own);
        assert_eq!(env(Setup::Sandbox), stub);
        assert_eq!(
            env(Setup::Control),
            [
                pair("PATH", "/bin"),
                pair("CODEX_HOME", "/h/.codex"),
                pair(PARENT_VAR, "tok-1")
            ]
        );
        assert_eq!(
            env(Setup::Flags),
            [pair("PATH", "/bin"), pair("CODEX_HOME", "/h/.codex")]
        );
        assert_eq!(
            env(Setup::Clean),
            [pair("PATH", "/bin"), pair("CODEX_HOME", "/own/.codex")]
        );
    }

    #[test]
    fn plants_each_piece_where_codex_reads_it() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("home/.codex");
        let marketplace = tmp.path().join("marketplace");
        let planted = plant(&config, &tmp.path().join("marks"), &marketplace).unwrap();
        let read = |path: &Path| fs::read_to_string(path).unwrap();

        let toml = read(&config.join("config.toml"));
        let mark = config.join("mark.sh");
        for line in [
            "[mcp_servers.planted]".to_owned(),
            format!("command = {}", json!(mark)),
            "args = [\"mcp-server\"]".to_owned(),
            "[marketplaces.planted]".to_owned(),
            "source_type = \"local\"".to_owned(),
            format!("source = {}", json!(marketplace)),
            "[plugins.\"planted@planted\"]".to_owned(),
            "enabled = true".to_owned(),
        ] {
            assert!(toml.lines().any(|l| l == line), "{line} not in {toml}");
        }
        let hooks = read_json(config.join("hooks.json"));
        for event in ["SessionStart", "UserPromptSubmit", "PreToolUse", "Stop"] {
            assert_eq!(
                hooks["hooks"][event][0]["hooks"][0]["command"],
                format!("{} hook", shell_quote(&mark.to_string_lossy()))
            );
        }
        let install = config.join("plugins/cache/planted/planted/1.0.0");
        for plugin in [&install, &marketplace.join("plugins/planted")] {
            let manifest = read_json(plugin.join(".codex-plugin/plugin.json"));
            assert_eq!(manifest["name"], "planted");
            assert_eq!(manifest["skills"], "./skills/");
            let hooks = read_json(plugin.join("hooks/hooks.json"));
            assert!(
                hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                    .as_str()
                    .unwrap()
                    .ends_with(" plugin-hook")
            );
            assert!(
                read(&plugin.join("skills/planted-plugin/SKILL.md"))
                    .contains(&planted.plugin_skill)
            );
        }
        let listed = read_json(marketplace.join(".agents/plugins/marketplace.json"));
        assert_eq!(listed["plugins"][0]["source"]["path"], "./plugins/planted");
        assert_eq!(
            read(&config.join("rules/planted.rules")),
            format!(
                "prefix_rule(pattern=[\"touch\", {}], decision=\"forbidden\")\n",
                json!(planted.ruled)
            )
        );
        assert!(read(&config.join("skills/planted/SKILL.md")).contains(&planted.skill));
        assert!(read(&config.join("AGENTS.md")).contains(&planted.instructions));
        assert_eq!(read_json(config.join(AUTH))["OPENAI_API_KEY"], PLACEHOLDER);
        let own = tmp.path().join("own/.codex");
        sign_in_only(&config, &own).unwrap();
        let held: Vec<_> = fs::read_dir(&own)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(held, [AUTH]);
        assert_eq!(read(&own.join(AUTH)), read(&config.join(AUTH)));
        forget_sign_in(&own).unwrap();
        assert!(!own.join(AUTH).exists() && own.is_dir());
        forget_sign_in(&own).unwrap();
        // A sign-in it cannot delete is an error, so the check does not keep it.
        fs::create_dir(own.join(AUTH)).unwrap();
        assert!(forget_sign_in(&own).is_err());

        // The planted server and hooks leave their mark when they run.
        assert!(
            std::process::Command::new(&mark)
                .arg(MARKS[0])
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            Reached::of(&planted, None, "tok-p"),
            Reached {
                planted: vec![MARKS[0]],
                parent: false,
                ran: false,
            }
        );
    }

    #[test]
    fn the_clean_session_makes_the_ruled_file_and_prints_the_variable() {
        let tmp = tempfile::tempdir().unwrap();
        let planted = plant(
            &tmp.path().join("c"),
            &tmp.path().join("marks"),
            &tmp.path().join("m"),
        )
        .unwrap();
        let [touch, echo] = clean_commands(&planted);
        assert_eq!(
            touch,
            format!("touch {}", shell_quote(&planted.ruled.to_string_lossy()))
        );
        assert_eq!(echo, "echo \"parent: ${SKALD_LIVE_CHECK_PARENT:-unset}\"");
        let prompt = clean_prompt(&planted);
        assert!(
            prompt.contains(&format!("1. `{touch}`")) && prompt.contains(&format!("2. `{echo}`"))
        );
        let names: Vec<_> = clean_script(&planted)
            .iter()
            .map(|call| match call {
                Call::Tool { name, input } => (
                    *name,
                    serde_json::from_str::<Value>(input).unwrap()["cmd"].clone(),
                ),
                _ => ("", Value::Null),
            })
            .collect();
        assert_eq!(
            names,
            [
                ("exec_command", json!(touch)),
                ("exec_command", json!(echo))
            ]
        );
    }

    /// A request to the stub that holds the outputs of Codex's shell tool `outputs`, each with its
    /// exit code, as Codex 0.162.1 sent them.
    fn recorded(outputs: &[(i32, &str)]) -> StubLog {
        let input: Vec<Value> = outputs
            .iter()
            .enumerate()
            .map(|(n, (code, output))| {
                json!({
                    "type": "function_call_output",
                    "call_id": format!("call_stub_{n}"),
                    "output": format!(
                        "Chunk ID: 7d77bf\nWall time: 0.0000 seconds\nProcess exited with code \
                         {code}\nOriginal token count: 5\nOutput:\n{output}"
                    ),
                })
            })
            .collect();
        StubLog {
            requests: vec![json!({ "input": input }).to_string()],
            credentials: Vec::new(),
        }
    }

    #[test]
    fn reads_each_attempt_s_outcome_from_a_recorded_read_only_run() {
        let tmp = tempfile::tempdir().unwrap();
        let executor = Executor::make(tmp.path(), "http://127.0.0.1:1/").unwrap();
        // macOS: writing is not permitted and curl cannot connect; commands and reads run.
        let denied = format!(
            "zsh:1: operation not permitted: {}\n",
            executor.folder.written().display()
        );
        let stub = recorded(&[
            (0, &format!("{}\n", executor.inside)),
            (1, &denied),
            (1, &denied),
            (0, "ran-42\n"),
            (7, ""),
            (6, ""),
            (0, &format!("{}\n", executor.folder.secret)),
        ]);
        let done = executor.outcomes(Some(&stub), 0);
        assert_eq!(
            done,
            [Attempt::ReadInside, Attempt::Command, Attempt::ReadOutside]
        );
        let mut check = CodexCheck::new(Before::ReadOnly);
        let line = check.attempts(Mode::ReadOnly, &done, "d");
        assert!(!passed(&check), "{line}");
        assert!(line.starts_with("  read-only executor: fail — "), "{line}");
        for outcome in [
            "read a file in the folder: yes",
            "write a file in the folder: no",
            "write a file outside it: no",
            "run a command: yes",
            "reach the check's server: no",
            "reach a public host: no",
            "read a file outside the folder: yes",
        ] {
            assert!(line.contains(outcome), "{outcome}: {line}");
        }
        // Nothing reached the model: no attempt succeeded but what the folder shows.
        assert_eq!(executor.outcomes(None, 0), []);
        // In a Linux container bubblewrap cannot start, so no command runs, not even the read.
        let bwrap = "bwrap: Can't mount proc on /proc: Operation not permitted\n";
        let linux = recorded(&[(1, bwrap); 7]);
        assert_eq!(executor.outcomes(Some(&linux), 0), []);

        // A read-only sandbox that kept the executor to reading its folder passes.
        let mut kept = CodexCheck::new(Before::ReadOnly);
        kept.attempts(Mode::ReadOnly, &[Attempt::ReadInside], "d");
        assert!(passed(&kept));
        let mut nothing = CodexCheck::new(Before::ReadOnly);
        nothing.attempts(Mode::ReadOnly, &[], "d");
        assert!(
            !passed(&nothing),
            "a session that did nothing shows no sandbox"
        );
    }

    #[test]
    fn reads_each_attempt_s_outcome_from_a_recorded_full_access_run() {
        let tmp = tempfile::tempdir().unwrap();
        let executor = Executor::make(tmp.path(), "http://127.0.0.1:1/").unwrap();
        fs::write(executor.folder.written(), "written").unwrap();
        fs::write(executor.written_outside(), "written").unwrap();
        let stub = recorded(&[
            (0, &format!("{}\n", executor.inside)),
            (0, ""),
            (0, ""),
            (0, "ran-42\n"),
            (0, "served\n"),
            (0, "public-2\n"),
            (0, &format!("{}\n", executor.folder.secret)),
        ]);
        let done = executor.outcomes(Some(&stub), 1);
        assert_eq!(done, Attempt::ALL);
        let mut check = CodexCheck::new(Before::Full);
        let line = check.attempts(Mode::Full, &done, "d");
        assert!(passed(&check), "{line}");
        assert!(
            line.starts_with("  danger-full-access executor: pass — "),
            "{line}"
        );

        // The public host depends on the machine's network: without it the check still passes.
        let offline: Vec<Attempt> = done
            .iter()
            .copied()
            .filter(|a| *a != Attempt::PublicHost)
            .collect();
        let mut check = CodexCheck::new(Before::Full);
        check.attempts(Mode::Full, &offline, "d");
        assert!(passed(&check));
        // A text written by something else is no write of the executor's.
        fs::write(executor.folder.written(), "other").unwrap();
        assert!(
            !executor
                .outcomes(Some(&stub), 1)
                .contains(&Attempt::WriteInside)
        );
        let mut check = CodexCheck::new(Before::Full);
        check.attempts(Mode::Full, &[Attempt::ReadInside, Attempt::Command], "d");
        assert!(!passed(&check));
    }

    #[test]
    fn finds_a_planted_piece_in_the_stub_s_requests_or_not() {
        let tmp = tempfile::tempdir().unwrap();
        let planted = plant(
            &tmp.path().join("home/.codex"),
            &tmp.path().join("marks"),
            &tmp.path().join("m"),
        )
        .unwrap();
        let instructions = format!(
            "# AGENTS.md instructions for /w\n\n<INSTRUCTIONS>\nThe instructions' token is {}.\n",
            planted.instructions
        );

        // The control: each mark left, the skills listed, the instructions sent, the rule kept the
        // file from being made, and the variable printed.
        for mark in MARKS {
            fs::write(planted.marks.join(mark), "").unwrap();
        }
        let mut control = recorded(&[
            (1, "execution of `touch` is forbidden by a rule\n"),
            (0, "parent: tok-p\n"),
        ]);
        control.requests.push(format!(
            "- planted: A planted skill; its token is {}.\n- planted:planted-plugin: A planted \
             skill; its token is {}.\n{instructions}",
            planted.skill, planted.plugin_skill
        ));
        let reached = Reached::of(&planted, Some(&control), "tok-p");
        assert_eq!(
            reached,
            Reached {
                planted: MARKS.into_iter().chain(PIECES).collect(),
                parent: true,
                ran: true,
            }
        );
        let mut check = CodexCheck::new(Before::Clean);
        let line = check.session(Setup::Control, &reached, "d");
        assert!(
            line.starts_with("  control, without the clean flags: pass — "),
            "{line}"
        );
        assert!(passed(&check));

        // Only the clean flags: the rule did not keep the file from being made, and only the
        // instructions reached the model.
        for mark in MARKS {
            fs::remove_file(planted.marks.join(mark)).unwrap();
        }
        fs::write(&planted.ruled, "").unwrap();
        let mut flags = recorded(&[(0, ""), (0, "parent: unset\n")]);
        flags.requests.push(instructions);
        let reached = Reached::of(&planted, Some(&flags), "tok-p");
        assert_eq!(
            reached,
            Reached {
                planted: vec!["AGENTS.md"],
                parent: false,
                ran: true,
            }
        );
        let line = check.session(Setup::Flags, &reached, "d");
        assert!(
            line.starts_with("  with only the clean flags: pass — "),
            "{line}"
        );
        assert!(
            line.contains("planted that reached it: [\"AGENTS.md\"]"),
            "{line}"
        );
        let line = check.session(Setup::Clean, &reached, "d");
        assert!(line.contains(": fail — "), "{line}");
        assert!(!passed(&check));

        // The clean session: none of it.
        let clean = recorded(&[(0, ""), (0, "parent: unset\n")]);
        let reached = Reached::of(&planted, Some(&clean), "tok-p");
        assert_eq!(
            reached,
            Reached {
                planted: Vec::new(),
                parent: false,
                ran: true,
            }
        );
        let mut check = CodexCheck::new(Before::Clean);
        let line = check.session(Setup::Clean, &reached, "d");
        assert!(
            line.starts_with("  clean executor, in a config home of its own: pass — "),
            "{line}"
        );
        // A session that never signed in or ran shows nothing: it fails, as does one that got the
        // variable.
        for reached in [
            Reached::default(),
            Reached {
                parent: true,
                ran: true,
                ..Reached::default()
            },
        ] {
            let mut check = CodexCheck::new(Before::Clean);
            check.session(Setup::Clean, &reached, "d");
            assert!(!passed(&check), "{reached:?}");
            let mut check = CodexCheck::new(Before::Clean);
            check.session(Setup::Control, &reached, "d");
            assert!(!passed(&check), "{reached:?}");
        }
        let mut check = CodexCheck::new(Before::Clean);
        check.session(Setup::Flags, &Reached::default(), "d");
        assert!(!passed(&check));
    }

    #[test]
    fn a_signal_passes_only_when_each_step_came() {
        let all = Signalled {
            queued: true,
            reached: true,
            handed_over: true,
            claims: 1,
            token: true,
        };
        let mut check = CodexCheck::new(Before::Signal);
        let line = check.signal("idle session", Some(&all), "d");
        assert!(passed(&check));
        assert_eq!(
            line,
            "  idle session: pass — codex queue took the signal: yes; it reached the model: yes; \
             inbox handed the message over: yes; claim calls: 1; the token reached the model: \
             yes; d"
        );
        for broken in [
            Signalled {
                queued: false,
                ..all
            },
            Signalled {
                reached: false,
                ..all
            },
            Signalled {
                handed_over: false,
                ..all
            },
            Signalled { claims: 0, ..all },
            Signalled {
                token: false,
                ..all
            },
        ] {
            let mut check = CodexCheck::new(Before::Signal);
            check.signal("idle session", Some(&broken), "d");
            assert!(!passed(&check), "{broken:?}");
        }
        let mut check = CodexCheck::new(Before::Signal);
        let line = check.signal("busy session", None, "d");
        assert_eq!(line, "  busy session: fail — no signal went; d");
        assert!(!passed(&check));
    }

    #[test]
    fn each_attempt_s_command_does_what_its_outcome_reads() {
        let tmp = tempfile::tempdir().unwrap();
        let server = crate::before::LocalServer::start().unwrap();
        let executor = Executor::make(tmp.path(), &server.url).unwrap();
        // Run with no sandbox, each command but the public host's succeeds, and the outcomes read
        // from its output name exactly its attempt; the writes and the server show in the end.
        let offline: Vec<Attempt> = Attempt::ALL
            .into_iter()
            .filter(|attempt| *attempt != Attempt::PublicHost)
            .collect();
        let mut outputs = Vec::new();
        for attempt in &offline {
            let ran = std::process::Command::new("sh")
                .arg("-c")
                .arg(executor.command(*attempt))
                .current_dir(&executor.folder.folder)
                .output()
                .unwrap();
            assert!(ran.status.success(), "{attempt:?}: {ran:?}");
            let output = String::from_utf8(ran.stdout).unwrap();
            let alone = recorded(&[(0, &output)]);
            let mut done = executor.outcomes(Some(&alone), 0);
            done.retain(|done| !matches!(done, Attempt::WriteInside | Attempt::WriteOutside));
            let expected: &[Attempt] = match attempt {
                Attempt::WriteInside | Attempt::WriteOutside | Attempt::LocalServer => &[],
                _ => std::slice::from_ref(attempt),
            };
            assert_eq!(done, expected, "{attempt:?} printed {output:?}");
            outputs.push(output);
        }
        let outputs: Vec<(i32, &str)> = outputs.iter().map(|o| (0, o.as_str())).collect();
        assert_eq!(
            executor.outcomes(Some(&recorded(&outputs)), server.requests()),
            offline
        );
        assert!(server.requests() > 0);
        // The writes land where their outcomes look: one in the folder, one outside it.
        assert!(
            executor
                .folder
                .written()
                .starts_with(&executor.folder.folder)
        );
        assert!(
            !executor
                .written_outside()
                .starts_with(&executor.folder.folder)
        );
        assert_eq!(
            executor.command(Attempt::PublicHost),
            "curl -s -m 10 -o /dev/null https://example.com/ && echo public-$((1+1))"
        );
        for (attempt, command) in [
            (Attempt::Command, "echo ran-$((20+22))".to_owned()),
            (Attempt::LocalServer, format!("curl -s -m 5 {}", server.url)),
        ] {
            assert_eq!(executor.command(attempt), command);
        }
        // Neither marker is in a command: either reaches the model only from a command's output.
        for attempt in Attempt::ALL {
            let command = executor.command(attempt);
            for marker in [RAN, REACHED, &executor.inside, &executor.folder.secret] {
                assert!(!command.contains(marker), "{marker} in {command}");
            }
        }
    }

    #[test]
    fn the_executor_s_prompt_and_script_run_each_attempt_s_command_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let executor = Executor::make(tmp.path(), "http://127.0.0.1:1/").unwrap();
        let commands = Attempt::ALL.map(|attempt| executor.command(attempt));
        let prompt = executor.prompt();
        assert!(
            prompt.starts_with("Run each of these shell commands in order"),
            "{prompt}"
        );
        let mut at = 0;
        for (n, command) in commands.iter().enumerate() {
            let line = format!("{}. `{command}`\n", n + 1);
            let found = prompt[at..]
                .find(&line)
                .unwrap_or_else(|| panic!("{line} in {prompt}"));
            at += found + line.len();
        }
        let script: Vec<(&str, Value)> = executor
            .script()
            .iter()
            .map(|call| match call {
                Call::Tool { name, input } => (*name, serde_json::from_str(input).unwrap()),
                other => panic!("{other:?}"),
            })
            .collect();
        let expected: Vec<(&str, Value)> = commands
            .iter()
            .map(|command| {
                (
                    "exec_command",
                    json!({ "cmd": command, "yield_time_ms": 30_000 }),
                )
            })
            .collect();
        assert_eq!(script, expected);
    }

    #[test]
    fn names_each_clean_session_and_gives_the_verdict_line() {
        assert_eq!(Setup::CLEAN.map(Setup::name), ["control", "flags", "clean"]);
        assert_eq!(Setup::Sandbox.name(), "sandbox");
        assert!(!Setup::Sandbox.signs_in_from_home());
        assert!(Setup::CLEAN.iter().all(|setup| setup.signs_in_from_home()));

        let mut check = CodexCheck::new(Before::Clean);
        let ran = Reached {
            ran: true,
            ..Reached::default()
        };
        check.session(Setup::Clean, &ran, "d");
        assert_eq!(
            check.ended(&Options::default()),
            (
                "executor-clean on codex with the model stub: pass".to_owned(),
                true
            )
        );
        let mut check = CodexCheck::new(Before::ReadOnly);
        check.attempts(Mode::ReadOnly, &[], "d");
        assert_eq!(
            check.ended(&options(false, true)),
            (
                "executor-read-only on codex with the real model: fail".to_owned(),
                false
            )
        );
    }
}
