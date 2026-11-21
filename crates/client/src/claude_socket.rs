//! Claude Code sessions over their cross-session socket (`docs/dev/architecture.md`,
//! "Components"; `docs/dev/agent-delivery.md`, "Waking an idle session" and "Read confirmations";
//! `docs/protocol/delivery.md`, "Device to agent"):
//!
//! - `skald hook claude` on `SessionStart` registers the session, as the agent the hooks act as,
//!   with its socket and token, which Claude Code gives its hooks as [`hook::SOCKET_SETTING`] and
//!   [`hook::TOKEN_SETTING`], and its session file ([`register`]). The token stays in the local
//!   state. The hook goes through the helper, which it starts if none runs. `SessionEnd`
//!   ([`end`]), or a socket that is gone or takes no connection ([`runs`]), ends the
//!   registration.
//! - The helper keeps running while a session is registered, and serves the sessions ([`Served`]):
//!   as each message for an agent comes, it writes it into the socket of the agent's last
//!   registered session that still runs ([`write`]), after an auth line with the token, through the
//!   session's relay where it runs (`crate::claude_relay`), so a session that bypasses permission
//!   prompts takes it too: an `own` message as its line, a message below `own` only as the signal
//!   (`crate::recv`).
//! - What a socket took counts as handed over, and its `read` goes, only once the session file
//!   confirms it: an entry that went to the model holds its `msg_id`, and a later entry is the
//!   model's own, not an API error ([`Scan`]). The helper reads the file while the session runs,
//!   and a `Stop` hook reads it as the fallback ([`confirm`]): only the registered session's file,
//!   only for `msg_id`s, keeping no line. Meanwhile no other path hands the message over.
//! - The session's receipts `held`, `expired`, `refused`, `dropped` and `denied` say a line did not
//!   reach the model: its message waits for the hooks and `inbox`, its signal for the hooks. The
//!   file still confirms one the session later releases. A receipt goes to the line's reply
//!   address, a socket of the helper's next to the session's ([`reply_path`]), through the relay
//!   if the line went through it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::net::Shutdown;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use serde::Deserialize;
use serde_json::json;
use skald_proto::{Name, SpaceId, VersionRange};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

use crate::devices::text;
use crate::mailbox::Came;
use crate::state::{ClaudeSession, State};
use crate::{folders, hook, output, recv, resend, secrets};

/// The statuses of the receipts that say a line did not reach the model.
const FAILING: [&str; 5] = ["held", "expired", "refused", "dropped", "denied"];

/// How long a write to a session's socket, and a read of a receipt, may take.
const PATIENCE: Duration = Duration::from_secs(5);

/// How long the helper waits after a failed run before what comes starts another.
const AFTER_FAILURE: Duration = Duration::from_secs(5);

/// How often the helper checks that a registered session's socket takes a connection.
const ALIVE_EVERY: Duration = Duration::from_secs(2);

/// Registers the session of `agent` that `session` names, its folder the working directory, if
/// the folder is in a space and Claude Code gave the session's ID, file and socket.
pub fn register(dir: &Path, agent: &Name, session: &hook::Session) -> Result<(), String> {
    let (Some(id), Some(file), Some(socket)) = (&session.id, &session.file, &session.socket) else {
        return Ok(());
    };
    let Ok(space) = folders::working_space(dir) else {
        return Ok(());
    };
    let folder = std::env::current_dir()
        .map_err(|err| format!("cannot read the working directory: {err}"))?;
    let state = State::open(dir).map_err(text)?;
    state.add_agent(space.id, agent.as_str()).map_err(text)?;
    let registered = ClaudeSession {
        position: 0,
        session: id.clone(),
        space: space.id,
        agent: agent.as_str().to_owned(),
        folder,
        socket: socket.clone(),
        token: session.token.clone(),
        file: file.clone(),
    };
    state.register_claude(&registered).map_err(text)
}

/// Ends the registration of the session `session` names, once its file confirmed what it can.
pub fn end(dir: &Path, agent: &Name, session: &hook::Session) -> Result<(), String> {
    let Some(id) = &session.id else {
        return Ok(());
    };
    let state = State::open(dir).map_err(text)?;
    confirm(&state, agent, session).map_err(text)?;
    state.end_claude(id).map_err(text)
}

/// Records what the file of `session`, a session of `agent`, confirms, reading it whole: of a
/// registered session, its registered file, of what its socket took and what the other paths took;
/// otherwise, once a path took a message for `agent`, the file its hook input names, of what the
/// paths took, by the references of the messages a path gave in it ([`given`]).
pub fn confirm(
    state: &State,
    agent: &Name,
    session: &hook::Session,
) -> Result<(), crate::state::Error> {
    let Some(id) = &session.id else {
        return Ok(());
    };
    let registered = state.claude_sessions()?;
    let mut scan = Scan::default();
    if let Some(registered) = registered.iter().find(|each| each.session == *id) {
        let confirmed = scan.read(&registered.file);
        state.confirmed(id, &confirmed)?;
        confirmed_references(state, &registered.agent, &scan.references())?;
        return Ok(());
    }
    let Some(file) = &session.file else {
        return Ok(());
    };
    if state.taken_by_paths(agent.as_str())? {
        scan.read(file);
        confirmed_references(state, agent.as_str(), &scan.references())?;
    }
    Ok(())
}

/// Writes `lines`, each a line's `msg_id` and its text, into the socket of `session`, after an
/// auth line with its token, if it has one, from the helper's reply address, if it listens.
pub fn write(session: &ClaudeSession, lines: &[(String, String)]) -> io::Result<()> {
    let reply = reply_path(session);
    let from = match reply.exists() {
        true => format!("uds:{}", reply.display()),
        false => "skald".to_owned(),
    };
    let mut text = String::new();
    if let Some(token) = &session.token {
        text.push_str(&format!("{}\n", json!({ "type": "auth", "token": token })));
    }
    for (msg_id, content) in lines {
        let line = json!({
            "type": "user",
            "message": { "role": "user", "content": content },
            "from": from,
            "msg_id": msg_id,
            "session_id": session.session,
        });
        text.push_str(&format!("{line}\n"));
    }
    #[cfg(unix)]
    if session.token.is_some()
        && let Ok(relay) = UnixStream::connect(crate::claude_relay::path(&session.socket))
    {
        return relayed(relay, &text);
    }
    let mut stream = UnixStream::connect(&session.socket)?;
    stream.set_write_timeout(Some(PATIENCE))?;
    stream.write_all(text.as_bytes())?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

/// Writes `text` through `relay`, the session's relay (`crate::claude_relay`), once it answers
/// that the session's socket took it.
#[cfg(unix)]
fn relayed(mut relay: UnixStream, text: &str) -> io::Result<()> {
    use std::io::Read;

    relay.set_write_timeout(Some(PATIENCE))?;
    relay.set_read_timeout(Some(PATIENCE))?;
    relay.write_all(text.as_bytes())?;
    relay.shutdown(Shutdown::Write)?;
    let mut answer = String::new();
    relay.read_to_string(&mut answer)?;
    match answer == crate::claude_relay::TAKEN {
        true => Ok(()),
        false => Err(io::Error::other(
            "the session's relay did not take the lines",
        )),
    }
}

/// Whether the session whose socket is `socket` still runs: its socket takes a connection. Claude
/// Code leaves the file of a socket behind when it is killed. On Windows its socket is a named
/// pipe, which takes no Unix socket's connection: there the hooks hand messages over.
pub fn runs(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

/// The reply address of the helper for `session`: a socket next to the session's, the only place
/// Claude Code sends receipts to.
fn reply_path(session: &ClaudeSession) -> PathBuf {
    let name = format!("sk-{}-{}.sock", std::process::id(), session.position);
    match session.socket.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

/// The `orig_msg_id` of `line`, a receipt a session sent, if it says its line did not reach the
/// model.
fn failing(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let status = value["status"].as_str()?;
    let failed = value["type"] == "control"
        && value["action"] == "peer_message_status"
        && FAILING.contains(&status);
    failed.then(|| value["orig_msg_id"].as_str().map(str::to_owned))?
}

/// A reference of a message a path gave a session, with when the session's file got it, in Unix
/// milliseconds.
pub type Given = (String, i64);

/// A reading of a session file, from where the last one ended: the `msg_id`s of the entries that
/// went to the model, and the references of the messages a path gave in them ([`given`]), whose
/// model's own entry has not come yet.
#[derive(Default)]
pub struct Scan {
    offset: u64,
    seen: BTreeSet<String>,
    seen_references: BTreeSet<Given>,
    /// The references the model got since they were last taken ([`Scan::references`]).
    references: Vec<Given>,
    /// The `msg_id`s and references that went to the model since they were last taken, whose
    /// model's own entry has not come yet, and the references of the lines the session queued
    /// ([`Scan::shown`]).
    shown: (Vec<String>, Vec<Given>, Vec<Given>),
}

/// A session file's entry, as far as a confirmation needs.
#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(rename = "isSidechain", default)]
    sidechain: bool,
    #[serde(rename = "isApiErrorMessage", default)]
    api_error: bool,
    #[serde(default)]
    origin: Option<Origin>,
    #[serde(default)]
    attachment: Option<Attachment>,
    #[serde(default)]
    message: Option<Said>,
    /// What a `queue-operation` does, and the line it queues.
    #[serde(default)]
    operation: String,
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    timestamp: Option<String>,
}

/// A queued line that went to the model during a turn, as the turn's attachment.
#[derive(Deserialize)]
struct Attachment {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    origin: Option<Origin>,
    #[serde(default)]
    prompt: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Origin {
    #[serde(default)]
    msg_id: Option<String>,
}

/// What a user entry says.
#[derive(Deserialize)]
struct Said {
    #[serde(default)]
    content: serde_json::Value,
}

/// The text of `content`, a user entry's or an attachment's: a string, or the text blocks of a
/// list; empty for a tool's result.
fn text_of(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(blocks) => (blocks.iter())
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// How Claude Code opens a channel event in the conversation, before its source: the server's
/// name, `skald`, or `plugin:<plugin>:skald` for a plugin's.
const CHANNEL_TAG: &str = "<channel source=\"";

/// What closes a channel event.
const CHANNEL_END: &str = "</channel>";

/// What comes before the output of an `asyncRewake` hook that exited with 2.
const REWAKE_OUTPUT: &str = "hook blocking error from command";

/// The references of the messages a path gave in `text`, a user entry's: in each channel event
/// of the session's `skald mcp` and in an `asyncRewake` hook's output, those Skald's lines put as
/// their own ([`own_reference`]).
fn given(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(CHANNEL_TAG) {
        let event = &rest[start + CHANNEL_TAG.len()..];
        let end = event.find(CHANNEL_END).unwrap_or(event.len());
        let source = event[..end].split('"').next().unwrap_or_default();
        if source == "skald" || source.ends_with(":skald") {
            parts.push(&event[source.len()..end]);
        }
        rest = &event[end..];
    }
    if let Some(start) = text.find(REWAKE_OUTPUT) {
        parts.push(&text[start + REWAKE_OUTPUT.len()..]);
    }
    (parts.into_iter())
        .flat_map(str::lines)
        .filter_map(own_reference)
        .collect()
}

/// The reference `line` names as its message's own: right after the bracket of sender, device and
/// trust level that opens Skald's short line ([`crate::forms::Message::shown`]), the first such
/// in `line`, or in the line that ends a cut one ([`crate::hook::fitted`]). A reply's reference and
/// those a text quotes are not its own.
fn own_reference(line: &str) -> Option<String> {
    let reference = match line.trim_start().strip_prefix("skald: #") {
        Some(tail) => tail.split_once(" is cut here: ")?.0,
        None => (line.match_indices('[')).find_map(|(at, _)| {
            let (bracket, rest) = line[at + 1..].split_once("] #")?;
            (bracket.split(" · ").count() == 3 && !bracket.contains('['))
                .then(|| rest.split(|c: char| !c.is_ascii_alphanumeric()).next())?
        })?,
    };
    let valid = reference.len() >= crate::forms::SHORTEST
        && (reference.bytes()).all(|byte| crate::forms::BASE32.contains(&byte));
    valid.then(|| reference.to_owned())
}

impl Scan {
    /// Reads the whole lines `file` got since; gives the `msg_id` of each message the model got:
    /// one an entry that went to the model holds, a user entry or, for a line queued during a
    /// turn, its attachment, once a later entry is the model's own, a reply, a tool call or
    /// reasoning, not an API error. The references such entries name are confirmed so too
    /// ([`Scan::references`]). A file that is gone or shorter is read again from its start.
    pub fn read(&mut self, file: &Path) -> Vec<String> {
        let mut confirmed = Vec::new();
        let Ok(mut opened) = File::open(file) else {
            return confirmed;
        };
        let length = opened.metadata().map_or(0, |metadata| metadata.len());
        if length < self.offset {
            *self = Scan::default();
        }
        if length == self.offset || opened.seek(SeekFrom::Start(self.offset)).is_err() {
            return confirmed;
        }
        let mut reader = BufReader::new(opened);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(read) if read > 0 && line.ends_with(b"\n") => {}
                // The end, or a line still being written.
                _ => break,
            }
            self.offset += line.len() as u64;
            self.entry(&line, &mut confirmed);
        }
        confirmed
    }

    /// Takes the references the model got since this was last asked.
    pub fn references(&mut self) -> Vec<Given> {
        std::mem::take(&mut self.references)
    }

    /// Takes the `msg_id`s and references that went to the model since this was last asked, whose
    /// model's own entry has not come yet, and the references of the lines the session queued
    /// since: Claude Code gives a queued line once its turn's tool ends.
    pub fn shown(&mut self) -> (Vec<String>, Vec<Given>) {
        let (ids, references, queued) = std::mem::take(&mut self.shown);
        let ids = ids.into_iter().filter(|id| self.seen.contains(id));
        let references =
            (references.into_iter()).filter(|reference| self.seen_references.contains(reference));
        (ids.collect(), references.chain(queued).collect())
    }

    fn entry(&mut self, line: &[u8], confirmed: &mut Vec<String>) {
        let Ok(entry) = serde_json::from_slice::<Entry>(line) else {
            return;
        };
        if entry.sidechain {
            return;
        }
        let at = (entry.timestamp.as_deref())
            .and_then(|timestamp| timestamp.parse::<jiff::Timestamp>().ok())
            .map(jiff::Timestamp::as_millisecond);
        let (origin, text) = match entry.kind.as_str() {
            "user" => (
                entry.origin,
                entry.message.map(|said| text_of(&said.content)),
            ),
            "attachment" => match entry.attachment {
                Some(attachment) if attachment.kind == "queued_command" => {
                    (attachment.origin, attachment.prompt.as_ref().map(text_of))
                }
                _ => (None, None),
            },
            "queue-operation" if entry.operation == "enqueue" => {
                let content = entry.content.as_ref().map(text_of).unwrap_or_default();
                let queued = (content.lines()).filter_map(own_reference);
                if let Some(at) = at {
                    self.shown.2.extend(queued.map(|reference| (reference, at)));
                }
                (None, None)
            }
            "assistant" if !entry.api_error => {
                confirmed.extend(std::mem::take(&mut self.seen));
                self.references
                    .extend(std::mem::take(&mut self.seen_references));
                (None, None)
            }
            _ => (None, None),
        };
        if let Some(msg_id) = origin.and_then(|origin| origin.msg_id) {
            self.shown.0.push(msg_id.clone());
            self.seen.insert(msg_id);
        }
        let Some(at) = at else {
            return;
        };
        for reference in given(&text.unwrap_or_default()) {
            self.shown.1.push((reference.clone(), at));
            self.seen_references.insert((reference, at));
        }
    }
}

/// The settings of the helper's runs for the sessions: where the secrets are, the protocol
/// versions and the request timeout.
pub type Settings = (secrets::Storage, VersionRange, Duration);

/// The agents whose sessions have paths the helper holds ([`resend::Held`]), each in its space,
/// with its session's folder.
pub type Holders = BTreeMap<(String, SpaceId), (PathBuf, resend::Held)>;

/// What a look gave each agent's paths the helper holds, with the agent and its space.
pub type Pushes = Vec<(String, SpaceId, resend::Pushed)>;

/// The sessions the helper serves.
pub struct Served {
    settings: Settings,
    /// Whether a command ran since the last look: the registrations may have changed, and it may
    /// have kept a message.
    due: bool,
    /// Whether the registrations may have changed.
    reload: bool,
    live: BTreeMap<String, Live>,
    /// When the last run failed, if it did.
    failed: Option<Instant>,
    /// The paths the helper held at the last look.
    held: Holders,
    /// When, in Unix milliseconds, the first message a path took times out, if any does.
    next: Option<i64>,
}

/// A registered session the helper serves: its reply address, its file's reading, and when its
/// socket last took a connection.
struct Live {
    session: ClaudeSession,
    reply: Option<Reply>,
    scan: Scan,
    alive: Option<Instant>,
}

/// The reply address of a session, removed on drop.
struct Reply {
    listener: UnixListener,
    path: PathBuf,
}

impl Drop for Reply {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Live {
    fn new(session: ClaudeSession) -> Self {
        let path = reply_path(&session);
        let _ = fs::remove_file(&path);
        let reply = UnixListener::bind(&path).ok().and_then(|listener| {
            let reply = Reply { listener, path };
            reply.listener.set_nonblocking(true).ok()?;
            Some(reply)
        });
        Live {
            session,
            reply,
            scan: Scan::default(),
            alive: None,
        }
    }

    /// Whether its session still runs: its socket is there and, at most [`ALIVE_EVERY`] ago,
    /// took a connection.
    fn runs(&mut self) -> bool {
        if !self.session.socket.exists() {
            return false;
        }
        if (self.alive).is_some_and(|alive| alive.elapsed() < ALIVE_EVERY) {
            return true;
        }
        let runs = runs(&self.session.socket);
        self.alive = runs.then(Instant::now);
        runs
    }

    /// The `orig_msg_id` of each failing receipt that came to the reply address.
    fn receipts(&self) -> Vec<String> {
        let mut failed = Vec::new();
        let Some(reply) = &self.reply else {
            return failed;
        };
        while let Ok((stream, _)) = reply.listener.accept() {
            // What a sender that closed sent is still read.
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(PATIENCE));
            let lines = BufReader::new(stream).lines().map_while(Result::ok);
            failed.extend(lines.filter_map(|line| failing(&line)));
        }
        failed
    }

    /// Records in `state` what came of its session since the last look: the failing receipts,
    /// what its file shows going to the model and what it confirms. Gives whether any came.
    fn record(&mut self, state: &State) -> bool {
        let (id, agent) = (&self.session.session, &self.session.agent);
        let failed = self.receipts();
        let confirmed = self.scan.read(&self.session.file);
        let references = self.scan.references();
        let (shown, shown_references) = self.scan.shown();
        for line in &failed {
            let _ = state.not_taken(id, line);
        }
        let mut came = !failed.is_empty();
        if !shown.is_empty() || !shown_references.is_empty() {
            let mut seqs = state
                .by_reference(agent, &shown_references)
                .unwrap_or_default();
            seqs.extend(state.written_ids(id, &shown).unwrap_or_default());
            let _ = state.shown(id, &seqs, agent);
        }
        came |= state.confirmed(id, &confirmed).is_ok_and(|count| count > 0);
        came |= confirmed_references(state, agent, &references).is_ok_and(|count| count > 0);
        came
    }
}

/// Records that the model of a session of `agent` got the messages whose `references` its file
/// names: each is handed over to it. Gives how many.
fn confirmed_references(
    state: &State,
    agent: &str,
    references: &[Given],
) -> Result<usize, crate::state::Error> {
    let seqs = state.by_reference(agent, references)?;
    for seq in &seqs {
        state.handed_over(*seq, agent)?;
    }
    Ok(seqs.len())
}

impl Served {
    pub fn new(settings: Settings) -> Self {
        Served {
            settings,
            due: true,
            reload: true,
            live: BTreeMap::new(),
            failed: None,
            held: Holders::new(),
            next: None,
        }
    }

    /// Notes that a command ran.
    pub fn ran(&mut self) {
        self.due = true;
    }

    /// Whether a session is registered, or the helper holds a path, as of the last look.
    pub fn serving(&self) -> bool {
        !self.live.is_empty() || !self.held.is_empty()
    }

    /// Whether the next look has anything to do: a session is registered, or may be, or a message
    /// a path took waits for its timeout.
    pub fn looks(&self) -> bool {
        self.due || self.reload || self.serving() || self.next.is_some()
    }

    /// Serves the sessions registered in the state in `dir`, and those whose paths the helper
    /// holds, `held`, `came` what came on the connection: ends those whose socket is gone or takes
    /// no connection; records the receipts that came and what the session files show and confirm;
    /// then, for each agent that has a confirmation, a session that ended, a path that came or a
    /// message whose timeout passed, whose space `came` concerns, or for each after a command ran,
    /// runs as the agent ([`recv::to_sessions`]). Gives whether it ran, and what the runs gave the
    /// paths the helper holds; `None` once a run panicked.
    pub fn look(&mut self, dir: &Path, came: &Came, held: &Holders) -> Option<(bool, Pushes)> {
        let mut due = std::mem::take(&mut self.due);
        if (self.next).is_some_and(|next| crate::state::unix_ms() >= next) {
            self.next = None;
            due = true;
        }
        // A run for the agent of each session that ended issues what its file confirmed.
        let mut runs = BTreeMap::new();
        if due || std::mem::take(&mut self.reload) {
            runs.extend(self.load(dir));
        }
        let retry = (self.failed).is_none_or(|failed| failed.elapsed() >= AFTER_FAILURE);
        let mut gone = Vec::new();
        let mut state = None;
        for (id, live) in &mut self.live {
            let key = (live.session.agent.clone(), live.session.space);
            if !live.runs() {
                gone.push(id.clone());
                runs.insert(key, live.session.folder.clone());
                continue;
            }
            let mut ran = due || (retry && came.concerns(Some(key.1)));
            let Some(state) = opened(&mut state, dir) else {
                continue;
            };
            ran |= live.record(state);
            if ran {
                runs.insert(key, live.session.folder.clone());
            }
        }
        for id in gone {
            if let Some(state) = opened(&mut state, dir) {
                let _ = state.end_claude(&id);
            }
            self.live.remove(&id);
        }
        drop(state);
        for (key, (folder, paths)) in held {
            let changed = self.held.get(key) != Some(&(folder.clone(), *paths));
            if due || changed || (retry && came.concerns(Some(key.1))) {
                runs.insert(key.clone(), folder.clone());
            }
        }
        self.held = held.clone();
        let mut pushes = Vec::new();
        for ((agent, space), folder) in &runs {
            let paths = (self.held.get(&(agent.clone(), *space)))
                .map(|(_, paths)| *paths)
                .unwrap_or_default();
            match self.run(dir, folder, agent, paths)? {
                Some(pushed) => {
                    self.failed = None;
                    if let Some(next) = pushed.next {
                        self.next = Some(self.next.map_or(next, |first| first.min(next)));
                    }
                    pushes.push((agent.clone(), *space, pushed));
                }
                None => self.failed = Some(Instant::now()),
            }
            // A run ends a registration whose socket does not take a write.
            self.reload = true;
        }
        Some((!runs.is_empty(), pushes))
    }

    /// Takes the registrations from the state in `dir`, keeping what it has of each it served;
    /// gives the agent, space and folder of each it served that ended.
    fn load(&mut self, dir: &Path) -> Vec<((String, SpaceId), PathBuf)> {
        let Ok(registered) = State::open(dir).and_then(|state| state.claude_sessions()) else {
            return Vec::new();
        };
        let mut live = std::mem::take(&mut self.live);
        for session in registered {
            let kept = live
                .remove(&session.session)
                .filter(|kept| kept.session == session);
            let id = session.session.clone();
            self.live
                .insert(id, kept.unwrap_or_else(|| Live::new(session)));
        }
        (live.into_values())
            .map(|ended| {
                let session = ended.session;
                ((session.agent, session.space), session.folder)
            })
            .collect()
    }

    /// Runs as `agent` in its `folder`, by the paths `held` the helper holds, its output dropped,
    /// then goes back to `dir`: what the run gave the paths, `None` if it failed; `None` if it
    /// panicked.
    fn run(
        &self,
        dir: &Path,
        folder: &Path,
        agent: &str,
        held: resend::Held,
    ) -> Option<Option<resend::Pushed>> {
        let Ok(agent) = crate::agents::name(agent) else {
            return Some(None);
        };
        let (storage, speaks, timeout) = self.settings;
        let ran = panic::catch_unwind(AssertUnwindSafe(|| {
            output::into(
                |_, _| Ok(()),
                || {
                    std::env::set_current_dir(folder).map_err(|err| err.to_string())?;
                    let held = (held, resend::resend_after());
                    recv::to_sessions(dir, storage, (speaks, timeout), &agent, held)
                },
            )
        }));
        let _ = std::env::set_current_dir(dir);
        ran.ok().map(Result::ok)
    }
}

/// The state in `dir`, opened once into `state`; `None` if it does not open.
fn opened<'s>(state: &'s mut Option<State>, dir: &Path) -> Option<&'s State> {
    if state.is_none() {
        *state = State::open(dir).ok();
    }
    state.as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Entries recorded from Claude Code 2.1.329 (`tests/claude_code_socket/recorded/`).
    const PEER_MESSAGE: &str =
        include_str!("../tests/claude_code_socket/recorded/peer-message.json");
    const QUEUED: &str = include_str!("../tests/claude_code_socket/recorded/queued.json");
    const QUEUED_COMMAND: &str =
        include_str!("../tests/claude_code_socket/recorded/queued-command.json");
    const REPLY: &str = include_str!("../tests/claude_code_socket/recorded/reply.json");
    const REASONING: &str = include_str!("../tests/claude_code_socket/recorded/reasoning.json");
    const TOOL_CALL: &str = include_str!("../tests/claude_code_socket/recorded/tool-call.json");
    const TOOL_RESULT: &str = include_str!("../tests/claude_code_socket/recorded/tool-result.json");
    const HOOK_OUTPUT: &str = include_str!("../tests/claude_code_socket/recorded/hook-output.json");
    const API_ERROR: &str = include_str!("../tests/claude_code_socket/recorded/api-error.json");

    /// The `msg_id` of the recorded message.
    const RECORDED: &str = "035fb4b5-a279-4ea4-a865-aaf77f8174bb";

    /// A session file of `entries`, read in one scan, then in another after `more`.
    fn scanned(entries: &[&str], more: &[&str]) -> (Vec<String>, Vec<String>) {
        // `cargo test` runs the tests in threads of one process: each scan has a directory of its
        // own.
        static SCANS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let scan = SCANS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = crate::test_dir::TestDir::new(&format!("claude-scan-{scan}"));
        let file = dir.join("session.jsonl");
        let lines = |entries: &[&str]| {
            (entries.iter())
                .map(|entry| format!("{}\n", entry.trim()))
                .collect::<String>()
        };
        fs::write(&file, lines(entries)).unwrap();
        let mut scan = Scan::default();
        let first = scan.read(&file);
        let mut opened = fs::OpenOptions::new().append(true).open(&file).unwrap();
        opened.write_all(lines(more).as_bytes()).unwrap();
        (first, scan.read(&file))
    }

    /// A message counts once the model's reply, tool call or reasoning follows it, once.
    #[test]
    fn the_model_s_own_entry_confirms_the_message_before_it() {
        for own in [REPLY, TOOL_CALL, REASONING] {
            let (first, then) = scanned(&[QUEUED, PEER_MESSAGE], &[own, REPLY]);
            assert!(first.is_empty(), "{first:?}");
            assert_eq!(then, [RECORDED], "{own}");
        }
        let (first, then) = scanned(&[PEER_MESSAGE, REPLY], &[REPLY]);
        assert_eq!((first, then), (vec![RECORDED.to_owned()], vec![]));
    }

    /// A line queued during a turn goes to the model as the turn's attachment: the model's entry
    /// after it confirms it.
    #[test]
    fn the_model_s_own_entry_confirms_a_line_queued_during_a_turn() {
        let (first, then) = scanned(&[QUEUED, TOOL_CALL, TOOL_RESULT, QUEUED_COMMAND], &[REPLY]);
        assert!(first.is_empty(), "{first:?}");
        assert_eq!(then, [RECORDED]);
    }

    /// An API error confirms nothing; the model's entry after it does.
    #[test]
    fn an_api_error_confirms_nothing() {
        let (first, then) = scanned(&[PEER_MESSAGE, API_ERROR], &[REPLY]);
        assert!(first.is_empty(), "{first:?}");
        assert_eq!(then, [RECORDED]);
    }

    /// The `msg_id` in a tool result, a queued prompt or a hook's own output is no message that
    /// went to the model.
    #[test]
    fn a_tool_result_a_queued_prompt_or_a_hook_s_output_confirms_nothing() {
        let holding = |recorded: &str, path: &[&str]| {
            let mut entry: serde_json::Value = serde_json::from_str(recorded).unwrap();
            let mut at = &mut entry;
            for key in path {
                at = &mut at[*key];
            }
            *at = json!(format!("msg_id {RECORDED}"));
            entry.to_string()
        };
        let result = holding(TOOL_RESULT, &["toolUseResult", "stdout"]);
        let hook = holding(HOOK_OUTPUT, &["attachment", "content"]);
        let queued = holding(QUEUED, &["content"]);
        let (first, then) = scanned(&[&result, &hook, &queued], &[REPLY]);
        assert!(first.is_empty() && then.is_empty(), "{first:?} {then:?}");
    }

    /// A line still being written waits for its end.
    #[test]
    fn a_line_still_being_written_waits() {
        let reply = REPLY.trim();
        let (first, then) = scanned(&[PEER_MESSAGE], &[&reply[..20]]);
        assert!(first.is_empty() && then.is_empty());
        let dir = crate::test_dir::TestDir::new("claude-scan-partial");
        let file = dir.join("session.jsonl");
        fs::write(&file, format!("{}\n{}", PEER_MESSAGE.trim(), &reply[..20])).unwrap();
        let mut scan = Scan::default();
        assert!(scan.read(&file).is_empty());
        let mut opened = fs::OpenOptions::new().append(true).open(&file).unwrap();
        opened
            .write_all(format!("{}\n", &reply[20..]).as_bytes())
            .unwrap();
        assert_eq!(scan.read(&file), [RECORDED]);
    }

    /// The own line of `reference`, a message from api, as Skald puts it, replying to `answered`, if
    /// it does, and saying `says`.
    fn own_line(reference: &str, answered: Option<&str>, says: &str) -> String {
        let reply =
            answered.map_or_else(String::new, |answered| format!(" (reply to #{answered})"));
        format!("[clienta/api · laptop · own] #{reference}{reply}: {says}")
    }

    /// A user entry of `text`, written at `timestamp`.
    fn user_entry(text: &str, timestamp: &str) -> String {
        json!({"type": "user", "timestamp": timestamp, "message": {"role": "user", "content": text}})
            .to_string()
    }

    /// When the entries of these tests were written, and that in Unix milliseconds.
    const WRITTEN: &str = "2026-11-20T11:44:39.396Z";
    const WRITTEN_MS: i64 = 1_795_175_079_396;

    /// A session file of `entries` and a reply after them, scanned: the references it confirms.
    fn confirming(entries: &[&str]) -> Vec<Given> {
        static FILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let reply = REPLY.trim();
        let mut all = entries.to_vec();
        all.push(reply);
        let name = FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = crate::test_dir::TestDir::new(&format!("claude-references-{name}"));
        let file = dir.join("session.jsonl");
        fs::write(&file, all.join("\n") + "\n").unwrap();
        let mut scan = Scan::default();
        scan.read(&file);
        scan.references()
    }

    /// A channel event of Skald's, its server's or its plugin's, confirms its message by the
    /// reference its line puts as its own, with when the file got it; not the reference of the
    /// message it replies to, nor those its text quotes. Another server's event confirms nothing.
    #[test]
    fn a_channel_event_confirms_its_own_reference_only() {
        let event = format!(
            "<channel source=\"skald\" from=\"skald\">\n{}\n</channel>",
            own_line(
                "ab3d",
                Some("xy7k"),
                "as #qq4m and [a · b · own] #rr5n said"
            )
        );
        let entry = user_entry(&event, WRITTEN);
        assert_eq!(confirming(&[&entry]), [("ab3d".to_owned(), WRITTEN_MS)]);
        let plugin = event.replace("\"skald\" from", "\"plugin:skald:skald\" from");
        let other = event.replace("\"skald\" from", "\"other\" from");
        let entries = [user_entry(&plugin, WRITTEN), user_entry(&other, WRITTEN)];
        let references = confirming(&[&entries[0], &entries[1]]);
        assert_eq!(references, [("ab3d".to_owned(), WRITTEN_MS)]);
    }

    /// The output of an `asyncRewake` hook confirms the messages whose lines it holds, or the
    /// line that ends a cut one; not the signal.
    #[test]
    fn a_hook_s_output_confirms_the_messages_of_its_lines() {
        let cut = crate::hook::fitted(&own_line("cd4e", None, &"x".repeat(12_000)), "cd4e");
        let output = format!(
            "Stop hook blocking error from command \"skald hook claude --wait 120\": {}\n{}\n{}",
            crate::hook::SIGNAL,
            own_line("ab3d", None, "first"),
            cut.lines().last().unwrap(),
        );
        let entry = user_entry(&output, WRITTEN);
        let references: Vec<String> = (confirming(&[&entry]).into_iter())
            .map(|(reference, _)| reference)
            .collect();
        assert_eq!(references, ["ab3d", "cd4e"]);
    }

    /// A reference the person types, or Skald's line outside a channel event or a hook's output,
    /// confirms nothing.
    #[test]
    fn references_outside_what_a_path_gave_confirm_nothing() {
        let typed = user_entry("please answer #ab3d", WRITTEN);
        let pasted = user_entry(&own_line("cd4e", None, "pasted"), WRITTEN);
        assert!(confirming(&[&typed, &pasted]).is_empty());
    }

    /// A line the session queued is shown, by the reference of its own line, until the model gets
    /// it; the model's entry after the queuing confirms nothing.
    #[test]
    fn a_queued_line_is_shown_and_not_confirmed() {
        let mut queued: serde_json::Value = serde_json::from_str(QUEUED).unwrap();
        queued["content"] = json!(own_line("ab3d", None, "queued"));
        queued["timestamp"] = json!(WRITTEN);
        let dir = crate::test_dir::TestDir::new("claude-queued-reference");
        let file = dir.join("session.jsonl");
        fs::write(&file, format!("{queued}\n{}\n", REPLY.trim())).unwrap();
        let mut scan = Scan::default();
        assert!(scan.read(&file).is_empty());
        assert!(scan.references().is_empty());
        let (_, shown) = scan.shown();
        assert_eq!(shown, [("ab3d".to_owned(), WRITTEN_MS)]);
    }

    /// Only a receipt of a failing status says its line did not reach the model.
    #[test]
    fn failing_receipts_name_their_line() {
        let receipt = |status: &str| {
            json!({
                "type": "control", "action": "peer_message_status", "status": status,
                "orig_msg_id": "m-1",
            })
            .to_string()
        };
        for status in FAILING {
            assert_eq!(
                failing(&receipt(status)).as_deref(),
                Some("m-1"),
                "{status}"
            );
        }
        assert_eq!(failing(&receipt("delivered")), None);
        assert_eq!(failing("{\"type\":\"user\"}"), None);
    }
}
