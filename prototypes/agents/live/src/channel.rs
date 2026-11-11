//! The Channels path's live check on Claude Code: how a session is set up, the pseudo-terminal
//! screen and its dialogs, the hook events that show the agent's turns, and the checks' verdicts.
//! Claude Code loads a channel only in an interactive session, so the binary runs it in a
//! pseudo-terminal.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agent_source::{Message, Source, State};
use serde_json::{Value, json};

use crate::{
    Agent, Call, Checks, Options, PLACEHOLDER, SENDER, SERVER, SHELL, StubLog, Usage, handed_over,
    handover_delay, pretty, shell_quote,
};

/// The channel server's binary, next to `live-check`.
pub const CHANNEL_BIN: &str = "agent-channel";

/// The terminal the pseudo-terminal emulates.
pub const TERM: &str = "xterm-256color";

/// Messages added in a row to the idle session with `--all`.
pub const IN_A_ROW: usize = 3;

/// A session's case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// No prompt: the session waits for one when the messages arrive.
    Idle,
    /// The prompt has the agent run `SHELL`; the message arrives while it runs.
    Busy,
    /// Idle, but without the flag that loads the channel.
    Unloaded,
}

impl Case {
    /// The session's prompt, if any.
    pub fn prompt(self) -> Option<String> {
        match self {
            Self::Idle | Self::Unloaded => None,
            Self::Busy => Some(format!(
                "Run the shell command `{SHELL}`. Then reply with only the check token from the \
                 {SERVER} channel message, or NONE if there is none."
            )),
        }
    }

    /// The model stub's tool calls, in order, before it replies `DONE`.
    pub fn script(self) -> &'static [Call] {
        match self {
            Self::Busy => &[Call::Shell],
            Self::Idle | Self::Unloaded => &[],
        }
    }

    /// Whether the session loads the channel.
    pub fn loads_channel(self) -> bool {
        self != Self::Unloaded
    }
}

/// The text of a check's message with `token`.
pub fn text(token: &str) -> String {
    format!("The check token is {token}. Reply with only the check token.")
}

/// Adds a check's message with `token` to `source`.
pub fn add_message(source: &Source, token: &str) -> io::Result<Message> {
    source.add(SENDER, &text(token))
}

/// Where a session's files go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Files {
    /// The session's directory, outside the project.
    pub dir: PathBuf,
    /// The working directory: empty, so Claude Code's trust dialog finds nothing to warn about.
    pub project: PathBuf,
    pub source: PathBuf,
    /// The hook events, as the `Stop` and `PreToolUse` hooks write them.
    pub hooks: PathBuf,
    /// The pseudo-terminal's output.
    pub screen: PathBuf,
}

impl Files {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_owned(),
            project: dir.join("project"),
            source: dir.join("source"),
            hooks: dir.join("hooks.json"),
            screen: dir.join("screen.log"),
        }
    }

    /// The shell command of a hook that only records what the agent does: it appends the hook's
    /// input to `hooks`.
    pub fn record(&self) -> String {
        format!("cat >> {}", shell_quote(&self.hooks.to_string_lossy()))
    }
}

/// Writes the session's MCP and settings files into `files.dir` and returns the arguments of an
/// interactive `case` session. Only these files and the arguments change Claude Code's config:
/// no project or user settings, only the channel server, no skills, the lowest effort, and no tool
/// but the busy session's `sleep`, without asking.
pub fn session(
    files: &Files,
    channel: &Path,
    case: Case,
    model: Option<&str>,
) -> io::Result<Vec<String>> {
    fs::create_dir_all(&files.project)?;
    let servers = files.dir.join("mcp.json");
    let server = json!({ "command": channel, "args": [files.source] });
    fs::write(
        &servers,
        pretty(&json!({ "mcpServers": { SERVER: server } })),
    )?;
    // Each event's input goes to one file.
    let hook = json!([{ "hooks": [{ "type": "command", "command": files.record() }] }]);
    let settings = files.dir.join("settings.json");
    // Prompt suggestions would cost a model request after each turn.
    fs::write(
        &settings,
        pretty(&json!({
            "promptSuggestionEnabled": false,
            "hooks": { "Stop": hook, "PreToolUse": hook },
        })),
    )?;

    let mut args: Vec<String> = case.prompt().into_iter().collect();
    let tools = match case {
        Case::Busy => "Bash",
        Case::Idle | Case::Unloaded => "",
    };
    args.extend(
        [
            "--setting-sources",
            "local",
            "--strict-mcp-config",
            "--mcp-config",
            &servers.to_string_lossy(),
            "--settings",
            &settings.to_string_lossy(),
            "--disable-slash-commands",
            "--tools",
            tools,
            "--permission-mode",
            "dontAsk",
            "--effort",
            "low",
        ]
        .map(String::from),
    );
    if case == Case::Busy {
        args.extend(["--allowedTools".to_owned(), format!("Bash({}:*)", "sleep")]);
    }
    if let Some(model) = model {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    if case.loads_channel() {
        args.extend([
            "--dangerously-load-development-channels".to_owned(),
            format!("server:{SERVER}"),
        ]);
    }
    Ok(args)
}

/// The variables the session gets on top of its environment, which off the stub is `live-check`'s
/// but for what [`Agent::command`] drops: the terminal, and, on the real model, the mark of a
/// session started by another, so Claude Code keeps no transcript or prompt history of it in the
/// user's config home.
pub fn extra_environment(real: bool) -> Vec<(String, String)> {
    let mut env = vec![("TERM", TERM)];
    if real {
        env.push(("CLAUDE_CODE_CHILD_SESSION", "1"));
    }
    env.into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

/// The CLI's whole environment on the model stub at `stub`: that of the `inbox` check, the
/// terminal, and a config home in which onboarding is done and the placeholder key approved, so
/// no dialog but the trust and channel ones opens.
pub fn stub_environment(home: &Path, stub: &str, path: &str) -> io::Result<Vec<(String, String)>> {
    let mut env = Agent::Claude.environment(home, stub, path)?;
    let config = home.join(".claude");
    // Claude Code keeps the last 20 characters of an approved key.
    let key = &PLACEHOLDER[PLACEHOLDER.len() - 20..];
    fs::write(
        config.join(".claude.json"),
        pretty(&json!({
            "hasCompletedOnboarding": true,
            "theme": "dark",
            "customApiKeyResponses": { "approved": [key], "rejected": [] },
        })),
    )?;
    env.extend(extra_environment(false));
    Ok(env)
}

/// A dialog Claude Code opens before the session starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    /// Whether the user trusts the working directory; its default is "No, exit".
    Trust,
    /// The warning about loading a development channel.
    DevelopmentChannels,
}

/// The keys that move the selection down and confirm it.
pub const DOWN: &[u8] = b"\x1b[B";
pub const ENTER: &[u8] = b"\r";

/// The text of the pseudo-terminal's screen without escape sequences or white space, which the
/// terminal UI replaces with cursor moves; and the keys that answer its dialogs.
#[derive(Debug, Default)]
pub struct Screen {
    text: Vec<u8>,
    escape: Escape,
    /// Where the text to search for the next dialog starts.
    from: usize,
    trust: Answer,
    channels: Answer,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Escape {
    #[default]
    None,
    /// After ESC.
    Start,
    /// In a control sequence, `ESC [`, until its final byte.
    Csi,
    /// In a string, such as `ESC ]`, until BEL or `ESC \`.
    String,
    /// ESC inside a string: `\` ends it.
    StringEnd,
    /// After `ESC (` and the like: one more byte.
    Charset,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Answer {
    #[default]
    No,
    /// The selection moved to the answer.
    Moved,
    Done,
}

const TRUST: &str = "Yes,Itrustthisfolder";
const TRUST_SELECTED: &str = "❯Yes,Itrustthisfolder";
const CHANNELS: &str = "Iamusingthisforlocaldevelopment";

impl Screen {
    /// Adds the terminal's output `bytes` and returns the keys to send, if a dialog waits.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<(Dialog, &'static [u8])> {
        for &byte in bytes {
            self.escape = match (self.escape, byte) {
                (Escape::None, 0x1b) => Escape::Start,
                (Escape::None, byte) => {
                    if !byte.is_ascii_whitespace() && !byte.is_ascii_control() {
                        self.text.push(byte);
                    }
                    Escape::None
                }
                (Escape::Start, b'[') => Escape::Csi,
                (Escape::Start, b']' | b'P' | b'_' | b'^' | b'X') => Escape::String,
                (Escape::Start, b'(' | b')' | b'*' | b'+' | b'#' | b'%') => Escape::Charset,
                (Escape::Start | Escape::Charset, _) => Escape::None,
                (Escape::Csi, 0x40..=0x7e) => Escape::None,
                (Escape::Csi, _) => Escape::Csi,
                (Escape::String, 0x07) => Escape::None,
                (Escape::String, 0x1b) => Escape::StringEnd,
                (Escape::String, _) => Escape::String,
                (Escape::StringEnd, b'\\') => Escape::None,
                (Escape::StringEnd, _) => Escape::String,
            };
        }
        self.answer()
    }

    fn answer(&mut self) -> Option<(Dialog, &'static [u8])> {
        let text = &self.text[self.from..];
        if self.trust != Answer::Done && contains(text, TRUST) {
            if self.trust == Answer::Moved && contains(text, TRUST_SELECTED) {
                self.trust = Answer::Done;
                self.from = self.text.len();
                return Some((Dialog::Trust, ENTER));
            }
            if self.trust == Answer::No {
                self.trust = Answer::Moved;
                self.from = self.text.len();
                return Some((Dialog::Trust, DOWN));
            }
        }
        if self.channels == Answer::No && contains(text, CHANNELS) {
            self.channels = Answer::Done;
            self.from = self.text.len();
            return Some((Dialog::DevelopmentChannels, ENTER));
        }
        None
    }

    /// Whether the screen showed `text`, white space aside.
    pub fn showed(&self, text: &str) -> bool {
        let text: String = text.split_whitespace().collect();
        contains(&self.text, &text)
    }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// What a hook says the agent did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent {
    /// `PreToolUse`: the agent starts a tool.
    ToolStarted(String),
    /// `Stop`: the agent's turn ended with this reply.
    Stopped(String),
}

/// The hook events in the hooks file's `text`: the inputs of the hooks, one JSON object each. A
/// partly written last object is left for later.
pub fn hook_events(text: &str) -> Vec<HookEvent> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .map_while(Result::ok)
        .filter_map(|input| {
            let text = |key: &str| input[key].as_str().unwrap_or_default().to_owned();
            match input["hook_event_name"].as_str() {
                Some("PreToolUse") => Some(HookEvent::ToolStarted(text("tool_name"))),
                Some("Stop") => Some(HookEvent::Stopped(text("last_assistant_message"))),
                _ => None,
            }
        })
        .collect()
}

/// What a session in the pseudo-terminal did: a session of the Channels check or of the
/// `codex queue` check.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PtySession {
    /// When the session started, in milliseconds since the Unix epoch.
    pub started_ms: u64,
    pub hooks: Vec<HookEvent>,
    /// What the model stub saw; `None` on the real model.
    pub stub: Option<StubLog>,
    /// When each token was first seen to reach the model, in milliseconds since the Unix epoch.
    pub reached_ms: Vec<(String, u64)>,
    /// The tokens the session used, if the CLI says.
    pub usage: Option<Usage>,
    /// Whether the screen showed that the channel loaded; `None` for a session of another check.
    pub channel_listening: Option<bool>,
}

/// What Claude Code shows below its banner once a development channel loads.
pub const LISTENING: &str = "inject directly in this session";

impl PtySession {
    pub fn new(started_ms: u64) -> Self {
        Self {
            started_ms,
            ..Self::default()
        }
    }

    /// The replies of the turns that ended.
    pub fn replies(&self) -> Vec<&str> {
        self.hooks
            .iter()
            .filter_map(|event| match event {
                HookEvent::Stopped(reply) => Some(reply.as_str()),
                HookEvent::ToolStarted(_) => None,
            })
            .collect()
    }

    /// Whether the agent started a shell command.
    pub fn shell_started(&self) -> bool {
        self.hooks
            .iter()
            .any(|event| *event == HookEvent::ToolStarted("Bash".to_owned()))
    }

    /// Whether `token` reached the model: in a request to the stub, or in a reply of the real
    /// model.
    pub fn got(&self, token: &str) -> bool {
        match &self.stub {
            Some(stub) => stub.saw(token),
            None => self.replies().iter().any(|reply| reply.contains(token)),
        }
    }

    /// Records, at `now_ms`, the tokens of `tokens` that reached the model for the first time.
    pub fn note(&mut self, tokens: &[&str], now_ms: u64) {
        for token in tokens {
            if self.got(token) && !self.reached_ms.iter().any(|(seen, _)| seen == token) {
                self.reached_ms.push(((*token).to_owned(), now_ms));
            }
        }
    }

    /// When `token` was first seen to reach the model.
    pub fn reached_at(&self, token: &str) -> Option<u64> {
        self.reached_ms
            .iter()
            .find(|(seen, _)| seen == token)
            .map(|(_, at)| *at)
    }

    /// Whether `token` reached the model in the turn that ran the shell command: on the stub, in
    /// a request that returns the command's result after the agent's last message; on the real
    /// model, in the first reply.
    pub fn in_first_turn(&self, token: &str) -> Option<bool> {
        match &self.stub {
            Some(stub) => {
                let body = stub.requests.iter().find(|body| body.contains(token))?;
                let body: Value = serde_json::from_str(body).ok()?;
                let messages = body["messages"].as_array()?;
                Some(
                    messages
                        .iter()
                        .rev()
                        .take_while(|message| message["role"] != "assistant")
                        .filter_map(|message| message["content"].as_array())
                        .flatten()
                        .any(|block| block["type"] == "tool_result"),
                )
            }
            None => {
                let first = self.replies().into_iter().next()?;
                self.got(token).then(|| first.contains(token))
            }
        }
    }

    /// Whether the stub got no credential but the placeholder.
    pub(crate) fn clean(&self) -> bool {
        self.stub
            .as_ref()
            .is_none_or(|stub| stub.failure().is_none())
    }

    pub fn summary(&self) -> String {
        let mut summary = format!(
            "replies {:?}",
            self.replies().iter().map(|r| r.trim()).collect::<Vec<_>>()
        );
        if self.channel_listening == Some(false) {
            summary.push_str(", the channel did not load");
        }
        match (&self.stub, self.usage) {
            (Some(stub), _) => {
                summary.push_str(&format!(", requests to the stub: {}", stub.requests.len()));
                if let Some(failure) = stub.failure() {
                    summary.push_str(&format!(", {failure}"));
                }
            }
            // On the stub, the CLI's token counts are the stub's.
            (None, Some(usage)) => summary.push_str(&format!(", {}", usage.summary())),
            (None, None) => {}
        }
        summary
    }
}

/// How long after `from_ms` the token reached the model, if it did.
pub(crate) fn reached_delay(session: &PtySession, token: &str, from_ms: u64) -> String {
    match session.reached_at(token) {
        Some(at_ms) => crate::seconds(at_ms.saturating_sub(from_ms)),
        None => "never".to_owned(),
    }
}

/// A message of a check: its token, when it was added and where it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub token: String,
    pub added_at_ms: u64,
    pub state: Option<State>,
}

/// The Channels path's checks: each returns its line in the output.
#[derive(Debug, Default)]
pub struct ChannelCheck {
    checks: Checks,
}

impl ChannelCheck {
    /// The idle session, whose message waited before it started.
    pub fn idle(&mut self, session: &PtySession, sent: &Sent) -> String {
        let pass = session.clean()
            && session.channel_listening == Some(true)
            && session.got(&sent.token)
            && handed_over(sent.state);
        let details = format!(
            "{}; handed over {} and reached the model {} after the session started",
            session.summary(),
            handover_delay(sent.state, session.started_ms),
            reached_delay(session, &sent.token, session.started_ms)
        );
        self.checks.report("idle session", pass, &details)
    }

    /// The messages added in a row to the idle session after its first turn.
    pub fn in_a_row(&mut self, session: &PtySession, sent: &[Sent]) -> String {
        let reached = sent.iter().filter(|s| session.got(&s.token)).count();
        let pass = session.clean()
            && !sent.is_empty()
            && reached == sent.len()
            && sent.iter().all(|s| handed_over(s.state));
        let delays: Vec<String> = sent
            .iter()
            .map(|s| {
                format!(
                    "{}/{}",
                    handover_delay(s.state, s.added_at_ms),
                    reached_delay(session, &s.token, s.added_at_ms)
                )
            })
            .collect();
        let details = format!(
            "{reached} of {} reached the model; turns ended: {}; handed over/reached the model \
             after adding: {}",
            sent.len(),
            session.replies().len(),
            delays.join(", ")
        );
        self.checks.report("messages in a row", pass, &details)
    }

    /// The other of two idle sessions: its own message reaches it, the first session's does not,
    /// and its own does not reach the first session.
    pub fn other(
        &mut self,
        session: &PtySession,
        own: &Sent,
        first: &PtySession,
        first_token: &str,
    ) -> String {
        let pass = session.clean()
            && session.got(&own.token)
            && handed_over(own.state)
            && !session.got(first_token)
            && !first.got(&own.token);
        let details = format!(
            "{}; its own message {} it, the first session's {} it, its own {} the first session",
            session.summary(),
            reached(session.got(&own.token)),
            reached(session.got(first_token)),
            reached(first.got(&own.token))
        );
        self.checks
            .report("the other of two idle sessions", pass, &details)
    }

    /// The busy session and its message, added when the agent started the shell command, if it
    /// did.
    pub fn busy(&mut self, session: &PtySession, sent: Option<&Sent>) -> String {
        let Some(sent) = sent else {
            let details = format!("{}; the agent ran no shell command", session.summary());
            return self.checks.report("busy session", false, &details);
        };
        let pass = session.clean() && session.got(&sent.token) && handed_over(sent.state);
        let turn = match session.in_first_turn(&sent.token) {
            Some(true) => "in the turn that ran the command",
            Some(false) => "in a later turn",
            None => "in no turn",
        };
        let details = format!(
            "{}; added while the shell command ran, handed over {} and reached the model {} \
             later, {turn}",
            session.summary(),
            handover_delay(sent.state, sent.added_at_ms),
            reached_delay(session, &sent.token, sent.added_at_ms)
        );
        self.checks.report("busy session", pass, &details)
    }

    /// A session without the channel flag: its message is handed over, yet never reaches the
    /// model, and the server cannot tell.
    pub fn unloaded(&mut self, session: &PtySession, sent: &Sent) -> String {
        let pass = session.clean()
            && session.channel_listening == Some(false)
            && handed_over(sent.state)
            && !session.got(&sent.token);
        let details = format!(
            "{}; handed over {} after the session started, the token {} the model",
            session.summary(),
            handover_delay(sent.state, session.started_ms),
            reached(session.got(&sent.token))
        );
        self.checks
            .report("session without the channel flag", pass, &details)
    }

    /// Whether every check passed.
    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output.
    pub fn verdict(&self, options: &Options) -> String {
        self.checks.verdict("channel", Agent::Claude, options)
    }
}

fn reached(yes: bool) -> &'static str {
    if yes { "reached" } else { "did not reach" }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::tests::read_json;

    use super::*;

    #[test]
    fn prompts_scripts_and_loads_the_channel_per_case() {
        assert_eq!(Case::Idle.prompt(), None);
        assert_eq!(Case::Unloaded.prompt(), None);
        assert_eq!(
            Case::Busy.prompt().unwrap(),
            "Run the shell command `sleep 20`. Then reply with only the check token from the \
             skald channel message, or NONE if there is none."
        );
        assert_eq!(Case::Busy.script(), [Call::Shell]);
        assert_eq!(Case::Idle.script(), []);
        assert_eq!(Case::Unloaded.script(), []);
        assert!(Case::Idle.loads_channel());
        assert!(Case::Busy.loads_channel());
        assert!(!Case::Unloaded.loads_channel());
    }

    #[test]
    fn adds_a_message_that_asks_for_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let message = add_message(&source, "tok-1").unwrap();
        assert_eq!(message.sender, "live-check of skald");
        assert_eq!(
            message.text,
            "The check token is tok-1. Reply with only the check token."
        );
        assert_eq!(source.state(&message.msg_id).unwrap(), Some(State::Waiting));
    }

    #[test]
    fn places_a_session_s_files_in_its_directory() {
        let files = Files::new(Path::new("/w/a"));
        assert_eq!(
            files,
            Files {
                dir: "/w/a".into(),
                project: "/w/a/project".into(),
                source: "/w/a/source".into(),
                hooks: "/w/a/hooks.json".into(),
                screen: "/w/a/screen.log".into(),
            }
        );
    }

    #[test]
    fn a_session_loads_only_its_own_config() {
        let tmp = tempfile::tempdir().unwrap();
        // A `'` in the path must not end the recording command's quoted word.
        let dir = tmp.path().join("it's");
        let files = Files::new(&dir);
        let at = |name: &str| dir.join(name).to_string_lossy().into_owned();
        let common = [
            "--setting-sources",
            "local",
            "--strict-mcp-config",
            "--mcp-config",
            &at("mcp.json"),
            "--settings",
            &at("settings.json"),
            "--disable-slash-commands",
            "--tools",
        ];
        let channel = ["--dangerously-load-development-channels", "server:skald"];

        let idle = session(&files, Path::new("/bin/channel"), Case::Idle, None).unwrap();
        let rest = ["", "--permission-mode", "dontAsk", "--effort", "low"];
        assert_eq!(idle, [&common[..], &rest, &channel].concat());
        assert!(files.project.is_dir());
        assert_eq!(
            fs::read_dir(&files.project).unwrap().count(),
            0,
            "the project is empty"
        );
        assert_eq!(
            read_json(at("mcp.json")),
            json!({ "mcpServers": { "skald": {
                "command": "/bin/channel", "args": [files.source] } } })
        );
        let settings = read_json(at("settings.json"));
        let hook = json!([{ "hooks": [{
            "type": "command",
            "command": format!("cat >> {}", shell_quote(&at("hooks.json"))),
        }] }]);
        assert_eq!(
            settings,
            json!({
                "promptSuggestionEnabled": false,
                "hooks": { "Stop": hook, "PreToolUse": hook },
            })
        );

        let busy = session(&files, Path::new("/bin/channel"), Case::Busy, Some("haiku")).unwrap();
        let prompt = Case::Busy.prompt().unwrap();
        let rest = [
            "Bash",
            "--permission-mode",
            "dontAsk",
            "--effort",
            "low",
            "--allowedTools",
            "Bash(sleep:*)",
            "--model",
            "haiku",
        ];
        assert_eq!(
            busy,
            [&[prompt.as_str()][..], &common, &rest, &channel].concat()
        );

        let unloaded = session(&files, Path::new("/bin/channel"), Case::Unloaded, None).unwrap();
        assert_eq!(unloaded, idle[..idle.len() - 2]);
    }

    #[test]
    fn the_session_s_environment_adds_only_the_terminal_and_on_the_real_model_the_child_mark() {
        let owned = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect()
        };
        assert_eq!(
            extra_environment(false),
            owned(&[("TERM", "xterm-256color")])
        );
        assert_eq!(
            extra_environment(true),
            owned(&[
                ("TERM", "xterm-256color"),
                ("CLAUDE_CODE_CHILD_SESSION", "1")
            ])
        );

        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let env = stub_environment(home, "http://127.0.0.1:1", "/bin").unwrap();
        let mut expected = Agent::Claude
            .environment(home, "http://127.0.0.1:1", "/bin")
            .unwrap();
        expected.extend(owned(&[("TERM", "xterm-256color")]));
        assert_eq!(env, expected);
        assert_eq!(
            read_json(home.join(".claude/.claude.json")),
            json!({
                "hasCompletedOnboarding": true,
                "theme": "dark",
                "customApiKeyResponses": { "approved": ["ve-check-placeholder"], "rejected": [] },
            })
        );
    }

    // Screens recorded from Claude Code 2.1.322 in a pseudo-terminal, shortened.
    const TRUST_SCREEN: &[u8] = b"\x1b[?2004h\x1b]0;Claude Code\x07Quick\x1b[1Csafety\x1b[1Ccheck\r\n\
        \x1b[38;5;153m\xe2\x9d\xaf\x1b[1CNo,\x1b[1Cexit\x1b[39m\r\n  Yes,\x1b[1CI\x1b[1Ctrust\x1b[1Cthis\
        \x1b[1Cfolder\r\n\x1b(BEnter to confirm";
    const TRUST_MOVED: &[u8] =
        b"\x1b[2A  No,\x1b[1Cexit\r\n\x1b[38;5;153m\xe2\x9d\xaf\x1b[1CYes,\x1b[1C\
        I\x1b[1Ctrust\x1b[1Cthis\x1b[1Cfolder\x1b[39m\r\n";
    const CHANNELS_SCREEN: &[u8] = b"WARNING:\x1b[1CLoading\x1b[1Cdevelopment\x1b[1Cchannels\r\n\
        \xe2\x9d\xaf\x1b[1C1.\x1b[1CI\x1b[1Cam\x1b[1Cusing\x1b[1Cthis\x1b[1Cfor\x1b[1Clocal\x1b[1C\
        development\r\n  2.\x1b[1CExit\r\n";
    const LISTENING_SCREEN: &[u8] = b"Channels\x1b[1C(experimental)\x1b[1Cmessages\x1b[1Cfrom\
        \x1b[1Cserver:skald\x1b[1Cinject\x1b[1Cdirectly\x1b[1Cin\x1b[1Cthis\x1b[1Csession";

    #[test]
    fn answers_the_trust_dialog_with_yes_and_the_channel_warning_with_local_development() {
        let mut screen = Screen::default();
        assert_eq!(screen.feed(b"\x1b[?25l"), None);
        assert_eq!(screen.feed(TRUST_SCREEN), Some((Dialog::Trust, DOWN)));
        // The old frame does not count: Enter waits for the selection on "Yes".
        assert_eq!(screen.feed(b"\x1b[?25h"), None);
        assert_eq!(screen.feed(TRUST_SCREEN), None);
        assert_eq!(screen.feed(TRUST_MOVED), Some((Dialog::Trust, ENTER)));
        assert_eq!(screen.feed(TRUST_MOVED), None);
        assert_eq!(
            screen.feed(CHANNELS_SCREEN),
            Some((Dialog::DevelopmentChannels, ENTER))
        );
        assert_eq!(screen.feed(CHANNELS_SCREEN), None);
        assert_eq!(screen.feed(TRUST_SCREEN), None, "each dialog once");
        assert!(!screen.showed(LISTENING));
        screen.feed(LISTENING_SCREEN);
        assert!(screen.showed(LISTENING));
    }

    #[test]
    fn answers_the_channel_warning_without_a_trust_dialog() {
        let mut screen = Screen::default();
        assert_eq!(
            screen.feed(CHANNELS_SCREEN),
            Some((Dialog::DevelopmentChannels, ENTER))
        );
    }

    #[test]
    fn reads_a_dialog_split_across_reads_and_escape_sequences() {
        let mut screen = Screen::default();
        let (head, tail) = CHANNELS_SCREEN.split_at(40);
        assert_eq!(screen.feed(head), None);
        assert_eq!(
            screen.feed(tail),
            Some((Dialog::DevelopmentChannels, ENTER))
        );
        // A string ends with BEL or with ESC \; a character set takes one byte.
        let mut screen = Screen::default();
        screen.feed(b"a\x1b]0;x y\x1b\\b\x1b(Bc\x1b]2;t\x07d\x1b[1;2Hd\x1b=e\x1bP1\x1b\\f");
        assert!(screen.showed("abcdde f"));
        assert!(!screen.showed("x"));
        assert!(!screen.showed("Bc"));
        // Spaces and control characters do not count.
        let mut screen = Screen::default();
        screen.feed(b"a b\x08\tc\r\n");
        assert!(screen.showed("abc"));
    }

    // Hook inputs recorded from Claude Code 2.1.322, shortened.
    const PRE_TOOL: &str = r#"{"session_id":"s","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"sleep 20"}}"#;
    const STOP: &str = "{\n  \"session_id\": \"s\",\n  \"hook_event_name\": \"Stop\",\n  \"stop_hook_active\": false,\n  \"last_assistant_message\": \"tok-1\"\n}";
    const SESSION_START: &str = r#"{"hook_event_name":"SessionStart","source":"startup"}"#;

    #[test]
    fn reads_the_hook_events() {
        let text = format!("{PRE_TOOL}{STOP}\n{SESSION_START}");
        assert_eq!(
            hook_events(&text),
            [
                HookEvent::ToolStarted("Bash".to_owned()),
                HookEvent::Stopped("tok-1".to_owned())
            ]
        );
        // A partly written event waits.
        let partial = format!("{PRE_TOOL}{}", &STOP[..20]);
        assert_eq!(
            hook_events(&partial),
            [HookEvent::ToolStarted("Bash".to_owned())]
        );
        assert_eq!(hook_events(""), []);
    }

    /// A session on the real model whose turns ended with `replies`.
    pub(crate) fn real(replies: &[&str]) -> PtySession {
        let mut session = PtySession::new(1_000);
        session.channel_listening = Some(true);
        for reply in replies {
            session.hooks.push(HookEvent::Stopped((*reply).to_owned()));
        }
        session
    }

    /// A session on the stub whose turns ended with `replies`, whose requests are `bodies` and
    /// whose credential headers other than the placeholder are `credentials`.
    pub(crate) fn on_stub(
        replies: &[&str],
        bodies: &[impl ToString],
        credentials: &[&str],
    ) -> PtySession {
        let mut session = real(replies);
        session.stub = Some(StubLog {
            requests: bodies.iter().map(ToString::to_string).collect(),
            credentials: credentials.iter().map(|name| (*name).to_owned()).collect(),
        });
        session
    }

    /// A session on the stub whose one turn ended with `DONE` and whose requests are `bodies`.
    fn stubbed(bodies: &[Value], credentials: &[&str]) -> PtySession {
        on_stub(&["Done."], bodies, credentials)
    }

    fn channel_turn(token: &str) -> Value {
        json!({ "messages": [{ "role": "user", "content": [
            { "type": "text", "text": format!("<channel source=\"skald\">{token}</channel>") }
        ] }] })
    }

    /// A request recorded from Claude Code 2.1.322, shortened: the message arrived while the
    /// command ran.
    fn after_the_command(token: &str) -> Value {
        json!({ "messages": [
            { "role": "user", "content": "Run the shell command" },
            { "role": "assistant", "content": [{ "type": "tool_use", "id": "t" }] },
            { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t" }] },
            { "role": "system", "content": [{ "type": "text", "text": format!(
                "A message arrived from skald while you were working:\n\
                 <channel source=\"skald\">{token}</channel>") }] }
        ] })
    }

    /// A request recorded from Claude Code 2.1.322, shortened: the message started a turn after
    /// the command's.
    fn after_the_turn(token: &str) -> Value {
        json!({ "messages": [
            { "role": "user", "content": "Run the shell command" },
            { "role": "assistant", "content": [{ "type": "tool_use", "id": "t" }] },
            { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t" }] },
            { "role": "assistant", "content": [{ "type": "text", "text": "NONE" }] },
            { "role": "user", "content": [{ "type": "text", "text": format!(
                "<channel source=\"skald\">{token}</channel>") }] }
        ] })
    }

    fn sent(token: &str, at_ms: Option<u64>) -> Sent {
        Sent {
            token: token.to_owned(),
            added_at_ms: 1_500,
            state: at_ms.map(|at_ms| State::HandedOver { at_ms }),
        }
    }

    #[test]
    fn a_token_reaches_the_stub_or_the_real_model_s_reply() {
        assert!(real(&["NONE", "tok-1"]).got("tok-1"));
        assert!(!real(&["NONE"]).got("tok-1"));
        let stub = stubbed(&[channel_turn("tok-1")], &[]);
        assert!(stub.got("tok-1"));
        assert!(!stub.got("tok-2"));
        // On the stub the reply does not count.
        let mut replied = stubbed(&[], &[]);
        replied.hooks.push(HookEvent::Stopped("tok-1".to_owned()));
        assert!(!replied.got("tok-1"));
    }

    #[test]
    fn notes_when_each_token_first_reached_the_model() {
        let mut session = real(&["tok-1"]);
        session.note(&["tok-1", "tok-2"], 2_000);
        session.hooks.push(HookEvent::Stopped("tok-2".to_owned()));
        session.note(&["tok-1", "tok-2"], 3_000);
        assert_eq!(session.reached_at("tok-1"), Some(2_000));
        assert_eq!(session.reached_at("tok-2"), Some(3_000));
        assert_eq!(session.reached_at("tok-3"), None);
        assert_eq!(reached_delay(&session, "tok-2", 1_000), "2.0 s");
        assert_eq!(reached_delay(&session, "tok-3", 1_000), "never");
    }

    #[test]
    fn tells_whether_the_message_came_in_the_turn_of_the_command() {
        let stub = stubbed(&[after_the_command("tok-1")], &[]);
        assert_eq!(stub.in_first_turn("tok-1"), Some(true));
        let later = stubbed(&[after_the_command("none"), after_the_turn("tok-1")], &[]);
        assert_eq!(later.in_first_turn("tok-1"), Some(false));
        assert_eq!(
            stubbed(&[channel_turn("tok-1")], &[]).in_first_turn("tok-1"),
            Some(false)
        );
        assert_eq!(later.in_first_turn("tok-2"), None);
        assert_eq!(real(&["tok-1"]).in_first_turn("tok-1"), Some(true));
        assert_eq!(real(&["NONE", "tok-1"]).in_first_turn("tok-1"), Some(false));
        assert_eq!(real(&["NONE"]).in_first_turn("tok-1"), None);
        assert_eq!(real(&[]).in_first_turn("tok-1"), None);
    }

    #[test]
    fn reads_what_a_session_did() {
        let mut session = real(&[" tok-1\n"]);
        session
            .hooks
            .insert(0, HookEvent::ToolStarted("Read".to_owned()));
        assert!(!session.shell_started());
        session
            .hooks
            .push(HookEvent::ToolStarted("Bash".to_owned()));
        assert!(session.shell_started());
        assert_eq!(session.replies(), [" tok-1\n"]);
        assert_eq!(session.summary(), r#"replies ["tok-1"]"#);
        session.channel_listening = Some(false);
        assert_eq!(
            session.summary(),
            r#"replies ["tok-1"], the channel did not load"#
        );
        assert_eq!(
            stubbed(&[json!({})], &["x-api-key"]).summary(),
            r#"replies ["Done."], requests to the stub: 1, the stub got a credential other than the placeholder in x-api-key"#
        );
    }

    #[test]
    fn checks_the_idle_session() {
        let mut check = ChannelCheck::default();
        let mut session = stubbed(&[channel_turn("tok-1")], &[]);
        session.note(&["tok-1"], 3_500);
        assert_eq!(
            check.idle(&session, &sent("tok-1", Some(2_000))),
            r#"  idle session: pass — replies ["Done."], requests to the stub: 1; handed over 1.0 s and reached the model 2.5 s after the session started"#
        );
        assert!(check.pass());
        assert_eq!(
            check.verdict(&Options::default()),
            "channel on claude with the model stub: pass"
        );
        for (session, state) in [
            (stubbed(&[channel_turn("tok-1")], &[]), None),
            (stubbed(&[channel_turn("tok-2")], &[]), Some(2_000)),
            (
                stubbed(&[channel_turn("tok-1")], &["x-api-key"]),
                Some(2_000),
            ),
            (
                PtySession {
                    channel_listening: Some(false),
                    ..stubbed(&[channel_turn("tok-1")], &[])
                },
                Some(2_000),
            ),
        ] {
            let mut check = ChannelCheck::default();
            assert!(
                check
                    .idle(&session, &sent("tok-1", state))
                    .contains(": fail — ")
            );
            assert!(!check.pass());
        }
        let real_options = Options {
            real: true,
            ..Options::default()
        };
        let mut check = ChannelCheck::default();
        check.idle(&real(&["NONE"]), &sent("tok-1", Some(2_000)));
        assert_eq!(
            check.verdict(&real_options),
            "channel on claude with the real model: fail"
        );
    }

    #[test]
    fn checks_messages_in_a_row() {
        let tokens = ["tok-1", "tok-2", "tok-3"];
        let all: Vec<Sent> = tokens.iter().map(|t| sent(t, Some(1_600))).collect();
        let mut session = real(&["tok-0", "tok-1 tok-2 tok-3"]);
        session.note(&tokens, 2_500);
        let mut check = ChannelCheck::default();
        assert_eq!(
            check.in_a_row(&session, &all),
            "  messages in a row: pass — 3 of 3 reached the model; turns ended: 2; handed \
             over/reached the model after adding: 0.1 s/1.0 s, 0.1 s/1.0 s, 0.1 s/1.0 s"
        );
        assert!(check.pass());

        let missing = real(&["tok-0", "tok-1 tok-3"]);
        let mut check = ChannelCheck::default();
        assert!(check.in_a_row(&missing, &all).contains(": fail — 2 of 3"));
        assert!(!check.pass());
        let mut waiting = all.clone();
        waiting[1].state = Some(State::Taken);
        let mut check = ChannelCheck::default();
        assert!(check.in_a_row(&session, &waiting).contains(": fail — "));
        let mut check = ChannelCheck::default();
        assert!(check.in_a_row(&session, &[]).contains(": fail — "));
        let mut check = ChannelCheck::default();
        let dirty = stubbed(&[channel_turn("tok-1 tok-2 tok-3")], &["cookie"]);
        assert!(check.in_a_row(&dirty, &all).contains(": fail — "));
    }

    #[test]
    fn checks_the_other_of_two_sessions() {
        let first = real(&["tok-a"]);
        let mut check = ChannelCheck::default();
        assert_eq!(
            check.other(
                &real(&["tok-b"]),
                &sent("tok-b", Some(2_000)),
                &first,
                "tok-a"
            ),
            r#"  the other of two idle sessions: pass — replies ["tok-b"]; its own message reached it, the first session's did not reach it, its own did not reach the first session"#
        );
        assert!(check.pass());
        for (session, state, first) in [
            (real(&["tok-b tok-a"]), Some(2_000), real(&["tok-a"])),
            (real(&["NONE"]), Some(2_000), real(&["tok-a"])),
            (real(&["tok-b"]), None, real(&["tok-a"])),
            (real(&["tok-b"]), Some(2_000), real(&["tok-a tok-b"])),
            (
                stubbed(&[channel_turn("tok-b")], &["authorization"]),
                Some(2_000),
                real(&["tok-a"]),
            ),
        ] {
            let mut check = ChannelCheck::default();
            assert!(
                check
                    .other(&session, &sent("tok-b", state), &first, "tok-a")
                    .contains(": fail — ")
            );
            assert!(!check.pass());
        }
    }

    #[test]
    fn checks_the_busy_session() {
        let mut session = stubbed(&[after_the_command("tok-1")], &[]);
        session.note(&["tok-1"], 2_000);
        let mut check = ChannelCheck::default();
        assert_eq!(
            check.busy(&session, Some(&sent("tok-1", Some(1_600)))),
            r#"  busy session: pass — replies ["Done."], requests to the stub: 1; added while the shell command ran, handed over 0.1 s and reached the model 0.5 s later, in the turn that ran the command"#
        );
        assert!(check.pass());
        let later = real(&["NONE", "tok-1"]);
        assert!(
            check
                .busy(&later, Some(&sent("tok-1", Some(1_600))))
                .ends_with("in a later turn")
        );
        assert!(check.pass());

        assert_eq!(
            check.busy(&real(&["NONE"]), None),
            r#"  busy session: fail — replies ["NONE"]; the agent ran no shell command"#
        );
        assert!(!check.pass());
        for (session, state) in [
            (real(&["NONE"]), Some(1_600)),
            (real(&["tok-1"]), None),
            (
                stubbed(&[after_the_command("tok-1")], &["x-api-key"]),
                Some(1_600),
            ),
        ] {
            let mut check = ChannelCheck::default();
            let line = check.busy(&session, Some(&sent("tok-1", state)));
            assert!(line.contains(": fail — "), "{line}");
            assert!(!check.pass());
        }
        let mut check = ChannelCheck::default();
        assert!(
            check
                .busy(&real(&["NONE"]), Some(&sent("tok-1", Some(1_600))))
                .ends_with("in no turn")
        );
    }

    #[test]
    fn checks_a_session_without_the_channel_flag() {
        let mut session = stubbed(&[], &[]);
        session.channel_listening = Some(false);
        session.hooks.clear();
        let mut check = ChannelCheck::default();
        assert_eq!(
            check.unloaded(&session, &sent("tok-1", Some(2_000))),
            "  session without the channel flag: pass — replies [], the channel did not load, \
             requests to the stub: 0; handed over 1.0 s after the session started, the token did \
             not reach the model"
        );
        assert!(check.pass());
        let listening = PtySession {
            channel_listening: Some(true),
            ..session.clone()
        };
        let reached = PtySession {
            stub: Some(StubLog {
                requests: vec![channel_turn("tok-1").to_string()],
                credentials: Vec::new(),
            }),
            ..session.clone()
        };
        let dirty = PtySession {
            stub: Some(StubLog {
                requests: Vec::new(),
                credentials: vec!["x-api-key".to_owned()],
            }),
            ..session.clone()
        };
        for (session, state) in [
            (listening, Some(2_000)),
            (reached, Some(2_000)),
            (dirty, Some(2_000)),
            (session, None),
        ] {
            let mut check = ChannelCheck::default();
            assert!(
                check
                    .unloaded(&session, &sent("tok-1", state))
                    .contains(": fail — ")
            );
            assert!(!check.pass());
        }
    }
}
