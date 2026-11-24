//! What the setup writes works in a Codex session: its MCP server is `skald mcp`, run with the
//! variables Codex gives it, and its hooks run Skald's hook handler for Codex on Codex's recorded
//! inputs, through a shell as Codex runs a hook's command, so a message reaches the session as the
//! signal by each of them, and a permission prompt reaches the person.

use std::process::Command;

use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::SETUP;
use crate::client::{INBOX, Session};
use crate::codex::{CODEX, CODEX_HOME, Codex, Event, Thread, assert_bare, codex_client, gives_by};
use crate::config_home::Read;
use crate::hooks::{Bob, Hooked, run_hook};
use crate::kit::{RECV, received};
use crate::lines::JSON_FLAG;
use crate::lines::reference;
use crate::talk::{ADA_HOST, delivered, received_until};
use crate::world::run;

/// The length of a device's reference in a default agent name.
const DEVICE_REFERENCE: usize = 6;

/// What Bob says to the session's agent; each is nowhere else.
const BY_POST_TOOL_USE: &str = "codex setup test: ask the session for the test log";
const BY_STOP: &str = "codex setup test: ask the session for the coverage report";
const BY_PROMPT: &str = "codex setup test: ask the session for the release notes";

/// The variables Codex 0.162.1 gives every MCP server it starts, those it has of them
/// (`codex-rs/rmcp-client/src/utils.rs`, `DEFAULT_ENV_VARS`), besides those the server's entry
/// names in `env_vars`.
#[cfg(unix)]
const CODEX_GIVES: &[&str] = &[
    "HOME",
    "LOGNAME",
    "PATH",
    "SHELL",
    "USER",
    "__CF_USER_TEXT_ENCODING",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TZ",
];
#[cfg(windows)]
const CODEX_GIVES: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SHELL",
    "COMSPEC",
    "SYSTEMROOT",
    "WINDIR",
    "SYSTEMDRIVE",
    "USERNAME",
    "USERDOMAIN",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "LOCALAPPDATA",
    "APPDATA",
    "TEMP",
    "TMP",
    "TMPDIR",
    "POWERSHELL",
    "PWSH",
];

/// The `PermissionRequest` input recorded from Codex 0.162.1, as the tests of Codex's permission
/// prompts have it.
const PERMISSION_REQUEST: &str =
    include_str!("../codex_permission/recorded/permission-request.json");

/// What `inbox` answers a session of the machine's device with no messages.
const NO_MESSAGES: &str = "No new messages.";

/// The type of a permission prompt in the person's `skald recv --json`.
const PROMPT_TYPE: &str = "permission_prompt";

/// `program`, run as Codex starts the MCP server `server`, an entry of `config.toml`, with
/// `template`'s settings: only those of [`CODEX_GIVES`] and of the entry's `env_vars`.
fn as_server(template: &Command, server: &Value, program: &str) -> Command {
    let named = (server["env_vars"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str);
    let given: Vec<&str> = CODEX_GIVES.iter().copied().chain(named).collect();
    let mut command = like(template, program);
    command.env_clear();
    for (name, value) in template.get_envs() {
        let name_text = name.to_string_lossy();
        let kept = (given.iter()).any(|given| given.eq_ignore_ascii_case(&name_text));
        if let (true, Some(value)) = (kept, value) {
            command.env(name, value);
        }
    }
    command
}

/// `program`, run as `template` is: with its settings, in its working directory.
fn like(template: &Command, program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    for (name, value) in template.get_envs() {
        if let Some(value) = value {
            command.env(name, value);
        }
    }
    if let Some(dir) = template.get_current_dir() {
        command.current_dir(dir);
    }
    command
}

/// `line`, a hook's command, run by the shell Codex runs it in on this OS, as `template` is.
fn shell(template: &Command, line: &str) -> Command {
    #[cfg(unix)]
    {
        let mut command = like(template, "/bin/sh");
        command.arg("-c").arg(line);
        command
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let powershell =
            std::path::Path::new(&root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = like(template, powershell);
        // The fixture clears the environment. PowerShell needs PATHEXT to run the native hook.
        command
            .env("PATHEXT", ".EXE")
            .args(["-NoProfile", "-Command", line]);
        command
    }
}

#[cfg(windows)]
#[test]
fn the_windows_hook_runs_from_a_path_with_spaces_and_an_apostrophe() {
    use std::fs;

    use crate::common::CLIENT;
    use crate::machine::{TestDir, bare_skald};

    let scratch = TestDir::new("codex-setup-powershell");
    let bin = scratch.join("Skald's O’Brien tools");
    fs::create_dir(&bin).expect("local installation directory");
    let exe = bin.join("skald.exe");
    fs::copy(CLIENT, &exe).expect("local Skald executable");

    let home = scratch.join("config");
    let mut template = bare_skald();
    template
        .env(CODEX_HOME, &home)
        .env("SKALD_STATE_DIR", scratch.join("state"));
    let set_up = like(&template, exe)
        .args([SETUP, CODEX])
        .output()
        .expect("set up local Codex");
    assert!(set_up.status.success(), "{set_up:?}");

    let line = Read::home(&home).hook_command("SessionStart");
    let input = json!({
        "hook_event_name": "SessionStart", "session_id": "fixture", "cwd": scratch.to_string_lossy()
    });
    run_hook(&mut shell(&template, &line), &input.to_string())
        .assert_silent("PowerShell SessionStart");

    let malformed = run_hook(&mut shell(&template, &line), "not JSON");
    assert_eq!(
        malformed.code,
        Some(1),
        "Skald did not reject malformed hook input: {malformed:?}"
    );
    assert!(
        malformed.stdout.is_empty() && malformed.stderr.is_empty(),
        "Skald printed output for malformed hook input: {malformed:?}"
    );
}

/// Ada's machine in `clienta`, as the tests of Codex's hooks have it. A Codex session in
/// `~/work/clientA/session` has its config home in `CODEX_HOME`, `~/.codex`, where the setup sets
/// Skald up. The server it wrote starts there as `skald mcp`, with only the variables Codex gives
/// it, initialized with the client name Codex gives; it lists `inbox`, which answers for the
/// machine's device. The hooks it wrote run on Codex's recorded inputs: `SessionStart` and
/// `UserPromptSubmit` print nothing. Bob sends the session's agent, the default one of
/// `skald mcp`, a message: its `PostToolUse` gives the signal; another: its `Stop` gives it; a third:
/// its `UserPromptSubmit` gives it. No signal holds a message's text or reference. The session
/// stops on a permission prompt: its `PermissionRequest` prints nothing, and the person's
/// `skald recv --json` shows the session waiting.
#[test]
fn the_server_and_hooks_it_writes_run_skald_in_a_codex_session() {
    let codex = Codex::new(5_209_001_001, "ada-codex-setup");
    let solo = &codex.web.solo;
    let bob = Bob::join(solo, 5_209_001_002, "bob-codex-setup");
    let dir = solo.dir("session");
    let session = Thread::new(codex.machine(), &dir, None);
    let home = session.home.to_string_lossy().into_owned();
    let settings = [(CODEX_HOME, home.as_str())];

    let set_up = codex.helped.run_with(&settings, &[SETUP, CODEX]);
    assert!(set_up.success, "the setup failed: {set_up:?}");

    let read = Read::home(&session.home);
    let template = codex.helped.command_in(&dir, &settings, &[]);
    let (program, args) = read.server_command();
    let entry = read.server().expect("the server's entry");
    let mut server = as_server(&template, entry, &program);
    server.args(&args);
    let client = codex_client();
    let mut mcp = Session::start(&mut server, &client);
    let tools = mcp.tools();
    let listed = (tools["tools"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .any(|tool| tool["name"].as_str() == Some(INBOX));
    assert!(listed, "no {INBOX} tool: {tools}");
    let answer = mcp.call(INBOX, json!({}));
    assert!(
        !answer.error && answer.text == NO_MESSAGES,
        "{}",
        answer.text
    );

    let hook = |event: Event| -> Hooked {
        let line = read.hook_command(event.name());
        run_hook(&mut shell(&template, &line), &session.input(event))
    };
    hook(Event::SessionStart).assert_silent("SessionStart");
    hook(Event::UserPromptSubmit).assert_silent("the turn's start");

    let device = reference(MessageId::from_bytes(*solo.talk.ada.device.as_bytes()));
    let agent = format!("{client}-{ADA_HOST}-{}", &device[..DEVICE_REFERENCE]);
    for (text, event) in [
        (BY_POST_TOOL_USE, Event::PostToolUse),
        (BY_STOP, Event::Stop),
        (BY_PROMPT, Event::UserPromptSubmit),
    ] {
        let msg_id = bob.sends(solo, &agent, text);
        // These hooks replay recorded input. Confirm the separate device took in the message.
        received(codex.machine());
        received_until(&bob.person.machine, |shown| {
            delivered(shown, msg_id, solo.talk.ada.device) > 0
        });
        let signal = gives_by(event, || hook(event));
        assert_bare(&signal, &[text], &[msg_id], &[]);
    }

    let mut input: Value = serde_json::from_str(PERMISSION_REQUEST).expect("JSON");
    input["session_id"] = json!(session.id);
    input["cwd"] = json!(dir.to_string_lossy());
    input["transcript_path"] = json!(session.rollout().to_string_lossy());
    let line = read.hook_command("PermissionRequest");
    run_hook(&mut shell(&template, &line), &input.to_string()).assert_silent("PermissionRequest");
    let shown = run(codex.machine(), &[RECV, JSON_FLAG]);
    assert!(shown.success, "{shown:?}");
    let waits = (shown.stdout.lines())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|record| record["type"] == PROMPT_TYPE && record["session"] == json!(session.id));
    assert!(waits, "the person sees no prompt of the session: {shown:?}");
    drop(mcp);
}
