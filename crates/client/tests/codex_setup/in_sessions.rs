//! What the setup writes works in a Codex session: its MCP server is `skald mcp`, and its hooks
//! run Skald's hook handler for Codex on Codex's recorded inputs, through a shell as Codex runs a
//! hook's command, so a message reaches the session as the signal by each of them.

use std::process::Command;

use skald_proto::MessageId;

use crate::SETUP;
use crate::client::{INBOX, Session};
use crate::codex::{CODEX, CODEX_HOME, Codex, Event, Thread, assert_bare, codex_client, gives_by};
use crate::config_home::Read;
use crate::hooks::{Hooked, api_sends, run_hook};
use crate::lines::reference;
use crate::talk::ADA_HOST;

/// The length of a device's reference in a default agent name.
const DEVICE_REFERENCE: usize = 6;

/// What api says to the session's agent; each is nowhere else.
const BY_POST_TOOL_USE: &str = "codex setup test: api asks the session for the test log";
const BY_STOP: &str = "codex setup test: api asks the session for the coverage report";
const BY_PROMPT: &str = "codex setup test: api asks the session for the release notes";

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
        use std::os::windows::process::CommandExt;
        let root = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let cmd = std::path::Path::new(&root).join("System32").join("cmd.exe");
        let mut command = like(template, cmd);
        command.raw_arg(format!("/d /s /c \"{line}\""));
        command
    }
}

/// Ada's machine alone in `clienta`, as the tests of Codex's hooks have it. A Codex session in
/// `~/work/clientA/session` has its config home in `CODEX_HOME`, `~/.codex`, where the setup sets
/// Skald up. The server it wrote starts there as `skald mcp`, initialized with the client name
/// Codex gives, and lists `inbox`. The hooks it wrote run on Codex's recorded inputs: `SessionStart`
/// and `UserPromptSubmit` print nothing. api sends the session's agent, the default one of
/// `skald mcp`, a message: its `PostToolUse` gives the signal; another: its `Stop` gives it; a third:
/// its `UserPromptSubmit` gives it. No signal holds a message's text or reference.
#[test]
fn the_server_and_hooks_it_writes_run_skald_in_a_codex_session() {
    let codex = Codex::new(5_209_001_001, "ada-codex-setup");
    let solo = &codex.web.solo;
    let dir = solo.dir("session");
    let session = Thread::new(codex.machine(), &dir, None);
    let home = session.home.to_string_lossy().into_owned();
    let settings = [(CODEX_HOME, home.as_str())];

    let set_up = codex.helped.run_with(&settings, &[SETUP, CODEX]);
    assert!(set_up.success, "the setup failed: {set_up:?}");

    let read = Read::home(&session.home);
    let template = codex.helped.command_in(&dir, &settings, &[]);
    let (program, args) = read.server_command();
    let mut server = like(&template, &program);
    server.args(&args);
    let client = codex_client();
    let mut mcp = Session::start(&mut server, &client);
    let tools = mcp.tools();
    let listed = (tools["tools"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .any(|tool| tool["name"].as_str() == Some(INBOX));
    assert!(listed, "no {INBOX} tool: {tools}");

    let hook = |event: Event| -> Hooked {
        let line = read.hook_command(event.name());
        run_hook(&mut shell(&template, &line), &session.input(event))
    };
    hook(Event::SessionStart).assert_silent("SessionStart");
    hook(Event::UserPromptSubmit).assert_silent("the turn's start");

    let device = reference(MessageId::from_bytes(*solo.talk.ada.device.as_bytes()));
    let agent = solo.short(&format!(
        "{client}-{ADA_HOST}-{}",
        &device[..DEVICE_REFERENCE]
    ));
    for (text, event) in [
        (BY_POST_TOOL_USE, Event::PostToolUse),
        (BY_STOP, Event::Stop),
        (BY_PROMPT, Event::UserPromptSubmit),
    ] {
        let msg_id = api_sends(solo, &agent, text);
        let signal = gives_by(event, || hook(event));
        assert_bare(&signal, &[text], &[msg_id], &[]);
    }
    drop(mcp);
}
