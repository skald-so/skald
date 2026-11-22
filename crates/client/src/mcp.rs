//! `skald mcp`: the MCP server on stdio, with rmcp, that an agent CLI starts from its config
//! (`docs/dev/architecture.md`, "Components"). Its session's folder is its working directory.
//!
//! - Before it answers `initialize`, or `server/discover` for a client that opens the session
//!   without it, the session registers as its agent in its folder's space, so the messages the
//!   device takes in from then on are kept for it, before its first tool call too.
//!   Its agent is the one `--agent` names or, without it, [`default_name`] of the client's name.
//! - The `inbox` tool returns the lines the agent's `skald recv` shows. The run writes them in one
//!   write, which returns once the tool's result is written to the client, and only then marks
//!   them handed over ([`recv::inbox`]): a result not written, or a call that fails, leaves them
//!   for the next call. [`Written`] says whether each result was written.
//! - The send tool sends as the agent's `skald send` does and returns what it says: `to` is its
//!   `--to`, or `*` its `--all`.
//! - The reply tool replies as the agent's `skald reply` does and returns what it says.
//! - The status tool sets the agent's state and note as its `skald set-status` does.
//! - The `message_status` tool returns what the agent's `skald message-status` says.
//! - The `agents` tool returns the lines the agent's `skald agents` shows ([`presence`]).
//! - The registration warns when another device of the account runs the agent's address, and the
//!   agent's next tool result after it says so once ([`presence::registered`]).
//! - The registration and each tool call are operations, as a command's: through the helper, or
//!   one-shot, and through a helper newer than the server too. Once the `skald` installed where the
//!   server's binary was is newer, as after an update while the server runs, that `skald` runs each
//!   ([`helper::relay`]).
//! - Out of one-shot mode, once registered, the server holds a session of the helper for its whole
//!   run, its agent and runtime with it, starting the helper again when it exits ([`hold`]): the
//!   helper knows the agent runs and does not exit idle under it ([`crate::uploads`]).

use std::collections::HashMap;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, CancelledNotificationParam,
    ContentBlock, DiscoverResult, Implementation, InitializeRequestParams, InitializeResult,
    JsonRpcMessage, ListToolsResult, PaginatedRequestParams, RequestId, ServerCapabilities,
    ServerConfig, Tool,
};
use rmcp::service::{NotificationContext, RequestContext, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::{IntoTransport, Transport};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use skald_proto::{DeviceId, Name, VersionRange};
use tokio::sync::oneshot;

use crate::devices::text;
use crate::forms::Form;
use crate::helper::{self, Route};
use crate::output::{self, Stream};
use crate::state::{self, State};
use crate::{
    Command, Operation, folders, forms, mailbox, message_status, presence, recovery, recv, reply,
    secrets, send, uploads,
};

/// The tool that returns the agent's messages.
pub const INBOX: &str = "inbox";

const INBOX_DESCRIPTION: &str = "Returns the Skald messages that came for this session, each \
                                 once: a line per message, [<from> · <device>] #<ref>: <text>.";
const AGAIN_DESCRIPTION: &str = "A #<ref> it returned: returns that message again, whole.";

/// The tool that sends a message as the agent, and its arguments.
pub const SEND: &str = "send";
const TO: &str = "to";
const TEXT: &str = "text";
const CHANNEL: &str = "channel";

/// What `to` takes for every agent in the channel.
const EVERYONE: &str = "*";

const SEND_DESCRIPTION: &str = "Sends a Skald message as this session: sent #<ref>.";
const TO_DESCRIPTION: &str = "An agent's address, space/owner/agent@server or owner/agent, or * for \
                              every agent in the channel. Every member device still gets the \
                              message: an addressee does not make it private.";
const CHANNEL_DESCRIPTION: &str = "The channel's ID; by default the space's default channel.";

/// The tool that lists the agents of the space.
pub const AGENTS: &str = "agents";

const AGENTS_DESCRIPTION: &str = "Lists the agents of this space, a line per agent and device: \
                                  [<owner>/<agent> · <device> · <level>] <state>.";

/// The tool that replies to a message as the agent, and its argument besides `text`.
pub const REPLY: &str = "reply";
const MESSAGE: &str = "message";

const REPLY_DESCRIPTION: &str = "Replies to a Skald message, to its sender in its channel: sent \
                                 #<ref>.";
const MESSAGE_DESCRIPTION: &str = "The message's #<ref>, as inbox shows it, or its msg_id.";

/// The tool that sets the agent's state and note, and its arguments.
pub const SET_STATUS: &str = "set_status";
const STATE: &str = "state";
const NOTE: &str = "note";

const SET_STATUS_DESCRIPTION: &str = "Sets this session's Skald status, which other agents see.";
const NOTE_DESCRIPTION: &str = "What it works on, up to 128 bytes.";

/// The tool that says where a message is.
pub const MESSAGE_STATUS: &str = "message_status";

const MESSAGE_STATUS_DESCRIPTION: &str = "Where a Skald message this session sent or got is: each \
                                          device's status, read, replied.";

/// What the send tool says without `to`.
const NO_TO: &str = "name the addressee in `to`: an agent's address, or * for every agent in the \
                     channel; nothing is sent";

/// What `inbox` returns with nothing waiting.
const NOTHING: &str = "No new messages.";

/// What a tool says on a machine with no device to name the agent by.
const NO_DEVICE: &str = "skald: this machine has no device yet: sign in with skald sign-in";

/// The most characters of an agent's name (`docs/protocol/README.md`, "Agent address").
const NAME_MOST: usize = 63;

/// How many characters of the device's reference a default name ends with.
const REFERENCE: usize = 6;

/// An operation of the server's, which it hands over as a command does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Call {
    /// Registers the session's agent in its folder's space.
    Register,
    /// The `inbox` tool.
    Inbox,
    /// The `inbox` tool with `message`: the message it names, again, whole.
    Again { message: String },
    /// The send tool: `skald send` with `--to` the agent `to` names, or `--all` without it, the
    /// channel `channel` names, if any, and the text `text`.
    Send {
        to: Option<String>,
        channel: Option<String>,
        text: String,
    },
    /// The reply tool: `skald reply` to the message `message` names with the text `text`.
    Reply { message: String, text: String },
    /// The status tool: `skald set-status` with `state` and `note`.
    SetStatus {
        state: uploads::Settable,
        note: Option<String>,
    },
    /// The session the server holds of the helper, its agent CLI `runtime` ([`hold`]).
    Hold { runtime: Option<String> },
    /// The `message_status` tool: `skald message-status` of the message `message` names.
    Status { message: String },
    /// The `agents` tool.
    Agents,
}

/// Runs `call` as `agent`, by the state in `dir`, its secrets in `storage`.
pub fn perform(
    call: Option<Call>,
    dir: &Path,
    storage: secrets::Storage,
    settings: (VersionRange, Duration),
    agent: Option<&Name>,
) -> Result<(), String> {
    let (Some(call), Some(agent)) = (call, agent) else {
        return Err("skald mcp runs only as itself".to_owned());
    };
    // The folder's space, its warnings held for the person.
    let space = || recv::holding_warnings(dir, true, || folders::working_space(dir));
    // Runs `sends` in the folder's space; in no space, sends nothing.
    let sending = |sends: &dyn Fn() -> Result<(), String>| match space() {
        Ok(_) => sends(),
        Err(why) => Err(format!("this agent sees no space: {why}; nothing is sent")),
    };
    if call != Call::Register {
        presence::notices(dir, agent);
    }
    match call {
        // In no space, there is nothing to register in.
        Call::Register => match folders::working_space(dir) {
            Ok(space) => {
                State::open(dir)
                    .and_then(|state| state.add_agent(space.id, agent.as_str()))
                    .map_err(text)?;
                presence::registered(dir, storage, settings, agent, &space);
                Ok(())
            }
            Err(_) => Ok(()),
        },
        Call::Agents => match space() {
            Ok(_) => presence::tool(dir, storage, settings),
            Err(why) => {
                println!("This agent sees no space: {why}.");
                Ok(())
            }
        },
        Call::Inbox => match space() {
            Ok(_) => recv::inbox(dir, storage, settings, agent),
            Err(why) => {
                println!("This agent sees no space: {why}.");
                Ok(())
            }
        },
        Call::Again { message } => recv::reread(dir, storage, agent, &message, Form::Short),
        Call::Send { to, channel, text } => sending(&|| {
            let named = (to.as_deref(), channel.as_deref(), text.as_str());
            send::run(dir, storage, settings, named, Some(agent), false)
        }),
        Call::Reply { message, text } => sending(&|| {
            let named = (message.as_str(), text.as_str());
            reply::run(dir, storage, settings, named, Some(agent), false)
        }),
        Call::SetStatus { state, note } => recv::holding_warnings(dir, true, || {
            uploads::set_status(dir, agent, state, note.as_deref())
        }),
        Call::Hold { .. } => Err("skald mcp holds a session only of the helper".to_owned()),
        Call::Status { message } => message_status::run(dir, storage, &message, Some(agent), false),
    }
}

/// The call of the `set_status` tool with `arguments`; fails, saying why, without `state` or with
/// one that is neither `busy` nor `available`, or with `note` not a string.
fn set_status_call(arguments: Option<&rmcp::model::JsonObject>) -> Result<Call, String> {
    let field = |name: &str| arguments.and_then(|arguments| arguments.get(name));
    let state = match field(STATE) {
        Some(Value::String(state)) if state == "busy" => uploads::Settable::Busy,
        Some(Value::String(state)) if state == "available" => uploads::Settable::Available,
        _ => return Err("give `state` as busy or available; nothing changed".to_owned()),
    };
    let note = match field(NOTE) {
        None | Some(Value::Null) => None,
        Some(Value::String(note)) => Some(note.clone()),
        Some(_) => return Err("`note` is not a string; nothing changed".to_owned()),
    };
    uploads::note_fits(note.as_deref())?;
    Ok(Call::SetStatus { state, note })
}

/// The call of the send tool with `arguments`; fails, saying why, without `to` or `text`, or with
/// one of its fields not a string.
fn send_call(arguments: Option<&rmcp::model::JsonObject>) -> Result<Call, String> {
    let field = |name: &str| match arguments.and_then(|arguments| arguments.get(name)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(given)) => Ok(Some(given.clone())),
        Some(_) => Err(format!("`{name}` is not a string; nothing is sent")),
    };
    let to = field(TO)?.ok_or(NO_TO)?;
    let text = field(TEXT)?.ok_or("give the text in `text`; nothing is sent")?;
    Ok(Call::Send {
        to: (to != EVERYONE).then_some(to),
        channel: field(CHANNEL)?,
        text,
    })
}

/// The call of the `inbox` tool with `arguments`: with `message`, that message again; fails,
/// saying why, if it is not a string.
fn inbox_call(arguments: Option<&rmcp::model::JsonObject>) -> Result<Call, String> {
    match arguments.and_then(|arguments| arguments.get(MESSAGE)) {
        None | Some(Value::Null) => Ok(Call::Inbox),
        Some(Value::String(message)) => Ok(Call::Again {
            message: message.clone(),
        }),
        Some(_) => Err(format!("`{MESSAGE}` is not a string")),
    }
}

/// The call of the reply tool with `arguments`; fails, saying why, without `message` or `text`,
/// or with one of them not a string.
fn reply_call(arguments: Option<&rmcp::model::JsonObject>) -> Result<Call, String> {
    let field = |name: &str| match arguments.and_then(|arguments| arguments.get(name)) {
        Some(Value::String(given)) => Ok(given.clone()),
        _ => Err(format!("give `{name}` as a string; nothing is sent")),
    };
    Ok(Call::Reply {
        message: field(MESSAGE)?,
        text: field(TEXT)?,
    })
}

/// The call of the `message_status` tool with `arguments`; fails, saying why, without `message`
/// as a string.
fn status_call(arguments: Option<&rmcp::model::JsonObject>) -> Result<Call, String> {
    match arguments.and_then(|arguments| arguments.get(MESSAGE)) {
        Some(Value::String(message)) => Ok(Call::Status {
            message: message.clone(),
        }),
        _ => Err(format!("give `{MESSAGE}` as a string")),
    }
}

/// The default name of an agent of the MCP client named `client` on the device `device` named
/// `device_name`: the client's name, the device's name and the first [`REFERENCE`] characters of
/// the base32 of SHA-256 of the device ID's bytes, joined with `-`. Each name is made as
/// [`name_part`] makes it, and left out if empty. Over 63 characters, the device's name is cut
/// from its end, then, once it is gone, the client's, each cut name's `-` at its end trimmed.
pub fn default_name(client: &str, device_name: &str, device: DeviceId) -> Name {
    let reference = &forms::hashed(device.as_bytes())[..REFERENCE];
    let room = NAME_MOST - REFERENCE - 1;
    let (mut client, mut device_name) = (name_part(client), name_part(device_name));
    let joined = client.len() + device_name.len() + usize::from(!client.is_empty());
    if !device_name.is_empty() && joined > room {
        let over = joined - room;
        if over < device_name.len() {
            let length = device_name.len() - over;
            cut(&mut device_name, length);
        } else {
            device_name.clear();
        }
    }
    if client.len() > room {
        cut(&mut client, room);
    }
    let parts = [client.as_str(), device_name.as_str(), reference];
    let name: Vec<&str> = parts.into_iter().filter(|part| !part.is_empty()).collect();
    name.join("-")
        .parse()
        .expect("a default name is a valid name")
}

/// `text` made a name as [`name_part`] makes it, cut to 63 characters as [`cut`] cuts it.
pub fn name_of(text: &str) -> String {
    let mut part = name_part(text);
    cut(&mut part, NAME_MOST);
    part
}

/// `text` with its ASCII letters lowercased, each run of characters other than `a`–`z` and `0`–`9`
/// made one `-`, `-` trimmed at both ends, as `architecture.md`, "Data model", makes a username.
fn name_part(text: &str) -> String {
    let mut part = String::new();
    for char in text.chars().map(|char| char.to_ascii_lowercase()) {
        if char.is_ascii_lowercase() || char.is_ascii_digit() {
            part.push(char);
        } else if !part.ends_with('-') {
            part.push('-');
        }
    }
    part.trim_matches('-').to_owned()
}

/// Cuts `part`, ASCII, to `length` characters, and trims a `-` the cut leaves at its end.
fn cut(part: &mut String, length: usize) {
    part.truncate(length);
    let trimmed = part.trim_end_matches('-').len();
    part.truncate(trimmed);
}

/// `skald mcp`: serves MCP on standard input and output until the client closes it, for
/// `operation`'s settings, as a run in one-shot mode if `one_shot`.
pub fn serve(operation: Operation, one_shot: bool) -> Result<(), String> {
    // Claude Code logs what its server says.
    #[cfg(unix)]
    let _relay = crate::claude_relay::Relay::from_settings().and_then(|started| {
        started
            .map_err(|err| eprintln!("the session's relay: {err}"))
            .ok()
    });
    let agent = operation.agent()?;
    let dir = state::dir(operation.state_dir.clone())?;
    // The person reads it in the agent CLI's log of the server.
    recovery::say_if_waiting(&dir);
    let pending = Pending::default();
    let server = Server {
        settings: Settings::of(&operation, one_shot, helper::installed()),
        dir,
        agent: Mutex::new(agent),
        pending: Arc::clone(&pending),
        holding: AtomicBool::new(false),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("cannot start the MCP server: {err}"))?;
    let served = runtime.block_on(async move {
        let transport = Written {
            stdio: IntoTransport::<RoleServer, _, _>::into_transport(rmcp::transport::stdio()),
            pending,
        };
        let running = (server.serve(transport).await)
            .map_err(|err| format!("the MCP session did not start: {err}"))?;
        running
            .waiting()
            .await
            .map(drop)
            .map_err(|err| format!("the MCP session failed: {err}"))
    });
    // A call that still runs, such as one waiting on the lock, ends with the process.
    runtime.shutdown_background();
    served
}

/// `skald mcp --relayed`: runs the call an older `skald mcp` relays to this one
/// ([`helper::relay`]), as a run in one-shot mode if `one_shot`, its output going back to it.
pub fn relayed(one_shot: bool) -> Result<(), String> {
    let (cwd, operation) = helper::relayed()?;
    let ran = std::env::set_current_dir(&cwd)
        .map_err(|err| format!("cannot use the working directory {}: {err}", cwd.display()))
        .and_then(|()| match (&operation.command, operation.agent()?) {
            (
                Command::Mcp {
                    call: Some(call), ..
                },
                Some(agent),
            ) => Settings::of(&operation, one_shot, None)
                .run(call.clone(), &agent, || helper::to_relaying),
            _ => Err("skald mcp relays only its own calls".to_owned()),
        });
    let code = match ran {
        Ok(ran) => u8::from(!ran),
        Err(err) => {
            let _ = helper::to_relaying(Stream::Err, format!("skald: {err}\n").as_bytes());
            1
        }
    };
    helper::relayed_code(code)
}

/// What a call's operation takes from the server's own settings.
#[derive(Clone)]
struct Settings {
    speaks: VersionRange,
    timeout: Duration,
    clock_offset: u64,
    state_dir: Option<PathBuf>,
    key_storage: secrets::Storage,
    github_api: String,
    dns_resolver: String,
    one_shot: bool,
    /// Where the server's binary is installed, which runs each call once it is a newer `skald`.
    installed: Option<PathBuf>,
}

impl Settings {
    /// The settings of `operation`'s server, in one-shot mode if `one_shot`, installed at
    /// `installed`.
    fn of(operation: &Operation, one_shot: bool, installed: Option<PathBuf>) -> Self {
        Settings {
            speaks: operation.speaks,
            timeout: operation.timeout,
            clock_offset: operation.clock_offset,
            state_dir: operation.state_dir.clone(),
            key_storage: operation.key_storage,
            github_api: operation.github_api.clone(),
            dns_resolver: operation.dns_resolver.clone(),
            one_shot,
            installed,
        }
    }

    /// The operation of `call` as `agent`.
    fn operation(&self, call: Call, agent: &Name) -> Operation {
        Operation {
            speaks: self.speaks,
            timeout: self.timeout,
            clock_offset: self.clock_offset,
            state_dir: self.state_dir.clone(),
            key_storage: self.key_storage,
            agent: Some(agent.to_string()),
            github_api: self.github_api.clone(),
            dns_resolver: self.dns_resolver.clone(),
            command: Command::Mcp {
                relayed: false,
                call: Some(call),
            },
            said: crate::link_prompt::Said::default(),
            said_key_changes: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Runs `call` as `agent`, as a command runs its operation, or with the newer `skald`
    /// installed where the server's binary was, each write of its output going to a sink `sink`
    /// gives; gives whether it succeeded, or fails with its error.
    fn run<S>(&self, call: Call, agent: &Name, sink: impl Fn() -> S) -> Result<bool, String>
    where
        S: FnMut(Stream, &[u8]) -> io::Result<()> + 'static,
    {
        let operation = self.operation(call, agent);
        let succeeded = |code| code == ExitCode::SUCCESS;
        if let Some(exe) = (self.installed.as_deref()).filter(|exe| helper::newer_at(exe)) {
            return helper::relay(exe, &operation, self.one_shot, &mut sink()).map(succeeded);
        }
        let dir = state::dir(operation.state_dir.clone())?;
        let route = (false, !self.one_shot, true);
        match helper::route_into(&dir, &operation, route, &mut sink())? {
            Route::Helped(code) => Ok(succeeded(code)),
            Route::Here(lock) => {
                let ran = output::into(sink(), || crate::perform(operation));
                mailbox::close_kept();
                drop(lock);
                ran.map(|()| true)
            }
        }
    }
}

/// How long the session held of the helper waits before it tries again after its helper went
/// away, and after it found none to hold.
const HOLD_AGAIN: Duration = Duration::from_secs(1);
const HOLD_LATER: Duration = Duration::from_secs(10);

/// Holds a session of the helper of `settings`' state for the server's whole run, as `agent`, its
/// agent CLI `runtime`, starting the helper when none runs; ends once the helper ends the session,
/// as in a folder in no space. What the helper's run of it says goes to the server's standard
/// error.
fn hold(settings: &Settings, agent: &Name, runtime: Option<String>) {
    let Ok(dir) = state::dir(settings.state_dir.clone()) else {
        return;
    };
    let operation = settings.operation(Call::Hold { runtime }, agent);
    let mut said = |_: Stream, bytes: &[u8]| io::stderr().lock().write_all(bytes);
    loop {
        match helper::route_into(&dir, &operation, (false, true, true), &mut said) {
            Ok(Route::Helped(_)) => return,
            Ok(Route::Here(lock)) => {
                drop(lock);
                thread::sleep(HOLD_LATER);
            }
            Err(_) => thread::sleep(HOLD_AGAIN),
        }
    }
}

/// The name of the client of the request of `context`: from the request's `_meta`, or from the
/// session's `initialize`; empty with neither.
fn client_of(context: &RequestContext<RoleServer>) -> String {
    (context.client_info())
        .map(|info| info.name)
        .unwrap_or_default()
}

/// The answer to each tool call whose result waits to be written: whether it was.
type Pending = Arc<Mutex<HashMap<RequestId, mpsc::Sender<bool>>>>;

/// The MCP server of one session.
struct Server {
    settings: Settings,
    /// The state directory.
    dir: PathBuf,
    /// The session's agent, once named: as `--agent` names it, or by default.
    agent: Mutex<Option<Name>>,
    pending: Pending,
    /// Whether it holds a session of the helper ([`hold`]).
    holding: AtomicBool,
}

impl Server {
    /// The session's agent: the one it has, or, once this machine has a device, the default name
    /// of the client named `client`.
    fn agent(&self, client: &str) -> Option<Name> {
        let mut agent = lock(&self.agent);
        if agent.is_none() {
            *agent = default_agent(&self.dir, client);
        }
        agent.clone()
    }

    /// Registers the session's agent, by the client named `client`, before the server answers;
    /// says why on its standard error if that fails, and the first tool call registers it then.
    async fn register(&self, client: &str) {
        let Some(agent) = self.agent(client) else {
            return;
        };
        let settings = self.settings.clone();
        let said = Said::default();
        let registered = tokio::task::spawn_blocking(move || {
            let registered = said.ran(settings.run(Call::Register, &agent, || said.sink()));
            (registered, said.text())
        })
        .await;
        if let Ok((false, why)) = registered {
            let _ = io::stderr().write_all(why.as_bytes());
        }
        if !self.settings.one_shot && !self.holding.swap(true, AtomicOrdering::SeqCst) {
            let (settings, runtime) = (self.settings.clone(), uploads::runtime(client));
            let agent = self.agent(client);
            if let Some(agent) = agent {
                thread::spawn(move || hold(&settings, &agent, runtime));
            }
        }
    }

    /// Runs the call `inbox` as `agent` for the request `id`: gives its result once its run wrote
    /// it, or once its run ended without.
    async fn inbox(&self, agent: Name, id: RequestId) -> CallToolResult {
        let (result, taken) = oneshot::channel();
        let (written, answer) = mpsc::channel();
        lock(&self.pending).insert(id.clone(), written);
        let said = Said::giving(result, answer);
        let settings = self.settings.clone();
        let mut running = tokio::task::spawn_blocking({
            let said = said.clone();
            move || said.ran(settings.run(Call::Inbox, &agent, || said.sink()))
        });
        let ran = tokio::select! {
            biased;
            Ok(text) = taken => return CallToolResult::success(vec![ContentBlock::text(text)]),
            ran = &mut running => ran.unwrap_or(false),
        };
        lock(&self.pending).remove(&id);
        let text = said.text();
        match (ran, text.trim().is_empty()) {
            (true, true) => CallToolResult::success(vec![ContentBlock::text(NOTHING)]),
            (true, false) => CallToolResult::success(vec![ContentBlock::text(text)]),
            (false, _) => CallToolResult::error(vec![ContentBlock::text(text)]),
        }
    }

    /// Runs `call`, of the send, reply or `agents` tool, or `inbox` with `message`, as `agent`:
    /// gives what its run said.
    async fn sending(&self, agent: Name, call: Call) -> CallToolResult {
        let settings = self.settings.clone();
        let said = Said::default();
        let ran = tokio::task::spawn_blocking({
            let said = said.clone();
            move || said.ran(settings.run(call, &agent, || said.sink()))
        })
        .await
        .unwrap_or(false);
        let text = vec![ContentBlock::text(said.text())];
        if ran {
            CallToolResult::success(text)
        } else {
            CallToolResult::error(text)
        }
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new("skald", env!("CARGO_PKG_VERSION"));
        info
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        self.register(&request.client_info.name).await;
        self.negotiate_initialize(&request)
    }

    /// As `initialize`, for a client that opens the session without it, as of the protocol
    /// version 2026-07-28.
    async fn discover(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<DiscoverResult, ErrorData> {
        self.register(&client_of(&context)).await;
        let versions = self.supported_protocol_versions().into_owned();
        Ok(DiscoverResult::from_server_info(versions, self.get_info()))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let schema = |schema: Value| match schema {
            Value::Object(schema) => schema,
            _ => unreachable!("a schema is an object"),
        };
        let inbox = schema(json!({
            "type": "object",
            "properties": { MESSAGE: { "type": "string", "description": AGAIN_DESCRIPTION } },
        }));
        let send = schema(json!({
            "type": "object",
            "properties": {
                TO: { "type": "string", "description": TO_DESCRIPTION },
                TEXT: { "type": "string" },
                CHANNEL: { "type": "string", "description": CHANNEL_DESCRIPTION },
            },
            "required": [TO, TEXT],
        }));
        let reply = schema(json!({
            "type": "object",
            "properties": {
                MESSAGE: { "type": "string", "description": MESSAGE_DESCRIPTION },
                TEXT: { "type": "string" },
            },
            "required": [MESSAGE, TEXT],
        }));
        let status = schema(json!({
            "type": "object",
            "properties": { MESSAGE: { "type": "string", "description": MESSAGE_DESCRIPTION } },
            "required": [MESSAGE],
        }));
        let set_status = schema(json!({
            "type": "object",
            "properties": {
                STATE: { "enum": ["busy", "available"] },
                NOTE: { "type": "string", "description": NOTE_DESCRIPTION },
            },
            "required": [STATE],
        }));
        Ok(ListToolsResult::with_all_items(vec![
            Tool::new(INBOX, INBOX_DESCRIPTION, inbox),
            Tool::new(SEND, SEND_DESCRIPTION, send),
            Tool::new(REPLY, REPLY_DESCRIPTION, reply),
            Tool::new(MESSAGE_STATUS, MESSAGE_STATUS_DESCRIPTION, status),
            Tool::new(SET_STATUS, SET_STATUS_DESCRIPTION, set_status),
            Tool::new(
                AGENTS,
                AGENTS_DESCRIPTION,
                schema(json!({ "type": "object" })),
            ),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = request.arguments.as_ref();
        let called = match request.name.as_ref() {
            INBOX => inbox_call(arguments),
            SEND => send_call(arguments),
            REPLY => reply_call(arguments),
            MESSAGE_STATUS => status_call(arguments),
            SET_STATUS => set_status_call(arguments),
            AGENTS => Ok(Call::Agents),
            name => {
                return Err(ErrorData::invalid_params(
                    format!("no tool named {name}"),
                    None,
                ));
            }
        };
        let called = match called {
            Ok(called) => called,
            Err(why) => {
                let why = format!("skald: {why}");
                return Ok(CallToolResult::error(vec![ContentBlock::text(why)]).into());
            }
        };
        let Some(agent) = self.agent(&client_of(&context)) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(NO_DEVICE)]).into());
        };
        Ok(match called {
            Call::Inbox => self.inbox(agent, context.id).await,
            call => self.sending(agent, call).await,
        }
        .into())
    }

    async fn on_cancelled(
        &self,
        notification: CancelledNotificationParam,
        _context: NotificationContext<RoleServer>,
    ) {
        // Its result is never written.
        let id = notification.request_id;
        if let Some(written) = id.and_then(|id| lock(&self.pending).remove(&id)) {
            let _ = written.send(false);
        }
    }
}

/// The agent a session of the MCP client named `client` registers by default on the device of the
/// state in `dir`, if it has one ([`default_name`]).
pub fn default_agent(dir: &Path, client: &str) -> Option<Name> {
    device_of(dir).map(|(id, name)| default_name(client, &name, id))
}

/// The ID and the certificate's `name` of the device of the state in `dir`, if it has one.
fn device_of(dir: &Path) -> Option<(DeviceId, String)> {
    if !dir.join(state::FILE).is_file() {
        return None;
    }
    let device = State::open(dir).ok()?.device().ok()??;
    Some((device.id, device.name))
}

/// What a call's run says, which its sink takes in.
#[derive(Clone, Default)]
struct Said(Arc<Mutex<Saying>>);

#[derive(Default)]
struct Saying {
    /// What it said so far: its error output, and its output if it gives no result.
    text: String,
    /// Where its result goes, with what it said before, and the answer whether it was written;
    /// `None` once it went, or for a call that gives none.
    result: Option<(oneshot::Sender<String>, mpsc::Receiver<bool>)>,
    /// Whether its result went.
    gone: bool,
}

impl Said {
    /// What the run of a call whose result goes to `result` says, `answer` saying whether the
    /// result was written.
    fn giving(result: oneshot::Sender<String>, answer: mpsc::Receiver<bool>) -> Self {
        Said(Arc::new(Mutex::new(Saying {
            result: Some((result, answer)),
            ..Saying::default()
        })))
    }

    /// What it said so far.
    fn text(&self) -> String {
        lock(&self.0).text.clone()
    }

    /// Whether the run that gave `ran` succeeded; takes in its error as its error output.
    fn ran(&self, ran: Result<bool, String>) -> bool {
        ran.unwrap_or_else(|err| {
            let _ = self.take(Stream::Err, format!("skald: {err}\n").as_bytes());
            false
        })
    }

    /// The sink of the run's writes ([`Said::take`]).
    fn sink(&self) -> impl FnMut(Stream, &[u8]) -> io::Result<()> + 'static {
        let said = self.clone();
        move |stream, bytes| said.take(stream, bytes)
    }

    /// Takes in a write of `bytes` to `stream`. The first to the output of a call that gives a
    /// result is its result, with what it said before: the write returns once the result is
    /// written, and fails if it was not. Once the result went, each write goes to the server's
    /// own error output.
    fn take(&self, stream: Stream, bytes: &[u8]) -> io::Result<()> {
        let mut saying = lock(&self.0);
        if saying.gone {
            drop(saying);
            return io::stderr().lock().write_all(bytes);
        }
        let said = String::from_utf8_lossy(bytes);
        let result = match stream {
            Stream::Out => saying.result.take(),
            Stream::Err => None,
        };
        let Some((result, answer)) = result else {
            saying.text.push_str(&said);
            return Ok(());
        };
        saying.gone = true;
        let whole = std::mem::take(&mut saying.text) + &said;
        drop(saying);
        let unwritten = || io::Error::new(io::ErrorKind::BrokenPipe, "the client took no result");
        result.send(whole).map_err(|_| unwritten())?;
        match answer.recv() {
            Ok(true) => Ok(()),
            _ => Err(unwritten()),
        }
    }
}

/// The server's transport, which says whether the result of each tool call [`Pending`] holds was
/// written; once it closes, none of those left was.
struct Written<T> {
    stdio: T,
    pending: Pending,
}

impl<T: Transport<RoleServer>> Transport<RoleServer> for Written<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        message: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let (id, result) = match &message {
            JsonRpcMessage::Response(reply) => (Some(&reply.id), true),
            JsonRpcMessage::Error(error) => (error.id.as_ref(), false),
            _ => (None, false),
        };
        let written = id.and_then(|id| lock(&self.pending).remove(id));
        let sent = self.stdio.send(message);
        async move {
            let sent = sent.await;
            if let Some(written) = written {
                let _ = written.send(result && sent.is_ok());
            }
            sent
        }
    }

    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleServer>>> + Send {
        self.stdio.receive()
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        lock(&self.pending).clear();
        self.stdio.close().await
    }
}

/// `mutex`'s value, also after a panic while it was held.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device ID whose reference is known.
    fn device() -> (DeviceId, String) {
        let id = DeviceId::from_bytes([7; 16]);
        (id, forms::hashed(id.as_bytes())[..REFERENCE].to_owned())
    }

    /// The client's and the device's names, each with its ASCII letters lowercased, each run of
    /// other characters than `a`–`z` and `0`–`9` made one `-` and `-` trimmed at both ends, then the
    /// device's reference.
    #[test]
    fn default_name_joins_the_cleaned_names_and_the_reference() {
        let (id, r) = device();
        let cases = [
            (
                "Claude Code",
                "ada-workstation",
                format!("claude-code-ada-workstation-{r}"),
            ),
            (
                "  Codex__CLI!!  v2 ",
                "Ada's Mac",
                format!("codex-cli-v2-ada-s-mac-{r}"),
            ),
            ("a-_b", "x", format!("a-b-x-{r}")),
            ("Claude - Code", "x", format!("claude-code-x-{r}")),
            // U+212A KELVIN SIGN lowercases to `k`, but only ASCII letters are lowercased.
            ("\u{212A}ilo", "Ädä", format!("ilo-d-{r}")),
            ("", "ada-workstation", format!("ada-workstation-{r}")),
            ("Claude Code", "", format!("claude-code-{r}")),
            ("Claude Code", "★ ★", format!("claude-code-{r}")),
            ("", "", r.clone()),
            ("!!", "--", r.clone()),
        ];
        for (client, device_name, expected) in cases {
            let name = default_name(client, device_name, id);
            assert_eq!(name.as_str(), expected, "{client:?} on {device_name:?}");
        }
    }

    /// Over 63 characters, the device's name is cut from its end first, then the client's, a `-`
    /// a cut leaves at the end trimmed; the reference stays whole.
    #[test]
    fn long_default_name_cuts_the_device_name_then_the_client_name() {
        let (id, r) = device();
        let fifty_one = format!("cursor-{}", "z".repeat(44));
        let fifty_five = "c".repeat(55);
        let cases = [
            // 51 + 1 + 15 + 1 + 6 is 74: the device's name is cut to `ada-`, then trimmed.
            (
                fifty_one.as_str(),
                "ada-workstation",
                format!("{fifty_one}-ada-{r}"),
            ),
            // The device's name is gone with its `-`, and the client's cut to 56.
            (
                &"o".repeat(70),
                "ada-workstation",
                format!("{}-{r}", "o".repeat(56)),
            ),
            // No room for a character of the device's name.
            (fifty_five.as_str(), "ada", format!("{fifty_five}-{r}")),
            ("", &"d".repeat(70), format!("{}-{r}", "d".repeat(56))),
            (
                &format!("{}-x", "o".repeat(55)),
                "",
                format!("{}-{r}", "o".repeat(55)),
            ),
        ];
        for (client, device_name, expected) in cases {
            let name = default_name(client, device_name, id);
            assert_eq!(name.as_str(), expected, "{client:?} on {device_name:?}");
            assert!(name.as_str().len() <= NAME_MOST, "{name}");
        }
    }

    /// A transport whose writes work if `works`.
    struct Stdio {
        works: bool,
    }

    impl Transport<RoleServer> for Stdio {
        type Error = io::Error;

        fn send(
            &mut self,
            _message: TxJsonRpcMessage<RoleServer>,
        ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
            let works = self.works;
            async move {
                if works {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "the client is gone",
                    ))
                }
            }
        }

        async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
            None
        }

        async fn close(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    /// The message `json` the server sends.
    fn message(json: serde_json::Value) -> TxJsonRpcMessage<RoleServer> {
        serde_json::from_value(json).expect("a message the server sends")
    }

    /// `Written` over a transport whose writes work if `works`, with the tool call `7` waiting:
    /// with what it learns of its result, and a runtime to send on.
    fn waiting(
        works: bool,
    ) -> (
        Written<Stdio>,
        mpsc::Receiver<bool>,
        tokio::runtime::Runtime,
    ) {
        let pending = Pending::default();
        let (written, said) = mpsc::channel();
        lock(&pending).insert(RequestId::Number(7), written);
        let transport = Written {
            stdio: Stdio { works },
            pending,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        (transport, said, runtime)
    }

    /// What `Written` over a transport whose writes work if `works` says of the tool call `7`
    /// whose answer is `answer`; `None` if it says nothing.
    fn said_written(works: bool, answer: serde_json::Value) -> Option<bool> {
        let (mut transport, said, runtime) = waiting(works);
        let _ = runtime.block_on(transport.send(message(answer)));
        assert!(lock(&transport.pending).is_empty(), "the call still waits");
        said.try_recv().ok()
    }

    /// A tool call's result is written only once its write worked; an error in its place is not
    /// its result.
    #[test]
    fn result_is_written_only_once_its_write_works() {
        let result = json!({ "jsonrpc": "2.0", "id": 7, "result": { "content": [] } });
        let error = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "error": { "code": -32603, "message": "it failed" },
        });
        assert_eq!(said_written(true, result.clone()), Some(true));
        assert_eq!(said_written(false, result), Some(false));
        assert_eq!(said_written(true, error), Some(false));
    }

    /// A message for another request leaves the call waiting; once the transport closes, the call
    /// learns its result was not written.
    #[test]
    fn closed_transport_leaves_no_result_written() {
        let (mut transport, said, runtime) = waiting(true);
        let other = json!({ "jsonrpc": "2.0", "id": 8, "result": {} });
        let _ = runtime.block_on(transport.send(message(other)));
        assert_eq!(
            lock(&transport.pending).len(),
            1,
            "the call no longer waits"
        );

        let _ = runtime.block_on(transport.close());

        assert!(said.try_recv().is_err(), "a result was written");
    }

    /// A run that fails gives that it failed, and its error as what it said.
    #[test]
    fn failed_run_says_its_error() {
        let said = Said::default();
        let _ = said.take(Stream::Err, b"skald: dropped #k3vq: it does not open\n");

        assert!(!said.ran(Err("cannot read the local state".to_owned())));
        assert_eq!(
            said.text(),
            "skald: dropped #k3vq: it does not open\nskald: cannot read the local state\n"
        );
        assert!(Said::default().ran(Ok(true)));
    }

    /// The server says it has tools, as skald.
    #[test]
    fn server_says_it_has_tools() {
        let server = Server {
            settings: Settings {
                speaks: VersionRange::default(),
                timeout: Duration::from_secs(1),
                clock_offset: 0,
                state_dir: None,
                key_storage: secrets::Storage::File,
                github_api: String::new(),
                dns_resolver: String::new(),
                one_shot: true,
                installed: None,
            },
            dir: PathBuf::new(),
            agent: Mutex::new(None),
            pending: Pending::default(),
            holding: AtomicBool::new(false),
        };

        let info = server.get_info();

        assert!(info.capabilities.tools.is_some(), "no tools: {info:?}");
        assert_eq!(info.server_info.name, "skald");
    }

    /// The send tool's call: `*` is every agent, with no `to`; without `to` or `text`, or with a
    /// field not a string, there is none, and the error says which field.
    #[test]
    fn send_call_takes_to_text_and_an_optional_channel() {
        let call = |arguments: Value| send_call(arguments.as_object());
        let send = |to: Option<&str>, channel: Option<&str>| Call::Send {
            to: to.map(str::to_owned),
            channel: channel.map(str::to_owned),
            text: "hi".to_owned(),
        };
        assert_eq!(
            call(json!({ TO: "bob/claude", TEXT: "hi", CHANNEL: "c1" })),
            Ok(send(Some("bob/claude"), Some("c1")))
        );
        assert_eq!(call(json!({ TO: "*", TEXT: "hi" })), Ok(send(None, None)));
        assert_eq!(
            call(json!({ TO: " *", TEXT: "hi", CHANNEL: null })),
            Ok(send(Some(" *"), None))
        );
        assert_eq!(call(json!({ TEXT: "hi" })), Err(NO_TO.to_owned()));
        assert_eq!(send_call(None), Err(NO_TO.to_owned()));
        assert_eq!(call(json!({ TO: null, TEXT: "hi" })), Err(NO_TO.to_owned()));
        for (arguments, field) in [
            (json!({ TO: "*" }), "`text`"),
            (json!({ TO: 7, TEXT: "hi" }), "`to`"),
            (json!({ TO: "*", TEXT: ["hi"] }), "`text`"),
            (json!({ TO: "*", TEXT: "hi", CHANNEL: 1 }), "`channel`"),
        ] {
            let why = call(arguments.clone()).expect_err(&arguments.to_string());
            assert!(why.contains(field), "{arguments}: {why}");
        }
    }

    /// The reply tool's call takes `message` and `text`; without either, or with one not a string,
    /// there is none, and the error says which.
    #[test]
    fn reply_call_takes_a_message_and_a_text() {
        let call = |arguments: Value| reply_call(arguments.as_object());
        assert_eq!(
            call(json!({ MESSAGE: "#k2m4", TEXT: "done" })),
            Ok(Call::Reply {
                message: "#k2m4".to_owned(),
                text: "done".to_owned(),
            })
        );
        for (arguments, field) in [
            (json!({ TEXT: "done" }), "`message`"),
            (json!({ MESSAGE: "#k2m4" }), "`text`"),
            (json!({ MESSAGE: 7, TEXT: "done" }), "`message`"),
            (json!({ MESSAGE: "#k2m4", TEXT: null }), "`text`"),
        ] {
            let why = call(arguments.clone()).expect_err(&arguments.to_string());
            assert!(why.contains(field), "{arguments}: {why}");
        }
        assert!(reply_call(None).is_err());
    }

    /// The `message_status` tool's call takes `message`; without it as a string there is none.
    #[test]
    fn status_call_takes_a_message() {
        let call = |arguments: Value| status_call(arguments.as_object());
        assert_eq!(
            call(json!({ MESSAGE: "#k2m4" })),
            Ok(Call::Status {
                message: "#k2m4".to_owned(),
            })
        );
        for arguments in [json!({}), json!({ MESSAGE: 7 })] {
            let why = call(arguments.clone()).expect_err(&arguments.to_string());
            assert!(why.contains("`message`"), "{arguments}: {why}");
        }
        assert!(status_call(None).is_err());
    }

    /// Two devices of one name give one client two names.
    #[test]
    fn default_names_of_two_devices_differ() {
        let one = default_name("Claude Code", "laptop", DeviceId::from_bytes([1; 16]));
        let two = default_name("Claude Code", "laptop", DeviceId::from_bytes([2; 16]));
        assert_ne!(one, two);
    }
}
