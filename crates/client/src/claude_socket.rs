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
//!   or, under `inline`, as its line, as the delivery settings say, which may hold it back to the
//!   end of a window of `batch`, when the helper runs again (`crate::recv`, [`crate::delivery`]).
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
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use serde::Deserialize;
use serde_json::json;
use skald_proto::{Name, VersionRange};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

use crate::devices::text;
use crate::mailbox::Came;
use crate::session_file::{Reader, opened};
use crate::state::{ClaudeSession, State};
use crate::{hook, output, recv, secrets};

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
    let Some((state, space, folder)) = hook::registering(dir, agent)? else {
        return Ok(());
    };
    let registered = ClaudeSession {
        position: 0,
        session: id.clone(),
        space,
        agent: agent.as_str().to_owned(),
        folder,
        socket: socket.clone(),
        token: session.token.clone(),
        file: file.clone(),
    };
    state.register_claude(&registered).map_err(text)
}

/// Ends the registration of the session `session` names, once its file confirmed what it can.
pub fn end(dir: &Path, session: &hook::Session) -> Result<(), String> {
    let Some(id) = &session.id else {
        return Ok(());
    };
    let state = State::open(dir).map_err(text)?;
    confirm(&state, id).map_err(text)?;
    state.end_claude(id).map_err(text)
}

/// Records what the file of the registered session `session`, if it is one, confirms of what its
/// socket took, reading it whole.
pub fn confirm(state: &State, session: &str) -> Result<(), crate::state::Error> {
    let registered = state.claude_sessions()?;
    let Some(registered) = registered.iter().find(|each| each.session == session) else {
        return Ok(());
    };
    let confirmed = Scan::default().read(&registered.file);
    state.confirmed(session, &confirmed)?;
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

/// A reading of a session file, from where the last one ended: the `msg_id`s of the entries that
/// went to the model whose model's own entry has not come yet.
#[derive(Default)]
pub struct Scan {
    offset: u64,
    seen: BTreeSet<String>,
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
}

/// A queued line that went to the model during a turn, as the turn's attachment.
#[derive(Deserialize)]
struct Attachment {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    origin: Option<Origin>,
}

#[derive(Deserialize)]
struct Origin {
    #[serde(default)]
    msg_id: Option<String>,
}

/// What it reads confirms the `msg_id` of each message the model got: one an entry that went to the
/// model holds, a user entry or, for a line queued during a turn, its attachment, once a later
/// entry is the model's own, a reply, a tool call or reasoning, not an API error.
impl Reader for Scan {
    type Confirmed = String;

    fn offset(&mut self) -> &mut u64 {
        &mut self.offset
    }

    fn entry(&mut self, line: &[u8], confirmed: &mut Vec<String>) {
        let Ok(entry) = serde_json::from_slice::<Entry>(line) else {
            return;
        };
        if entry.sidechain {
            return;
        }
        let origin = match entry.kind.as_str() {
            "user" => entry.origin,
            "attachment" => (entry.attachment)
                .filter(|attachment| attachment.kind == "queued_command")
                .and_then(|attachment| attachment.origin),
            "assistant" if !entry.api_error => {
                confirmed.extend(std::mem::take(&mut self.seen));
                None
            }
            _ => None,
        };
        if let Some(msg_id) = origin.and_then(|origin| origin.msg_id) {
            self.seen.insert(msg_id);
        }
    }
}

/// The settings of the helper's runs for the sessions: where the secrets are, the protocol
/// versions and the request timeout.
pub type Settings = (secrets::Storage, VersionRange, Duration);

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
    /// Of each agent whose last run held messages back under `batch`, its folder and when the
    /// first such window ends ([`recv::due`]).
    batches: BTreeMap<String, (PathBuf, Instant)>,
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
}

impl Served {
    pub fn new(settings: Settings) -> Self {
        Served {
            settings,
            due: true,
            reload: true,
            live: BTreeMap::new(),
            failed: None,
            batches: BTreeMap::new(),
        }
    }

    /// Notes that a command ran.
    pub fn ran(&mut self) {
        self.due = true;
    }

    /// Whether a session is registered, as of the last look.
    pub fn serving(&self) -> bool {
        !self.live.is_empty()
    }

    /// Whether the next look has anything to do: a session is registered, or may be.
    pub fn looks(&self) -> bool {
        self.due || self.reload || self.serving() || !self.batches.is_empty()
    }

    /// Serves the sessions registered in the state in `dir`, `came` what came on the connection:
    /// ends those whose socket is gone or takes no connection; records the receipts that came and
    /// what the session files confirm; then, for each agent that has a confirmation or a session
    /// that ended, whose space `came` concerns, whose window of `batch` ended, or for each after a
    /// command ran, runs as the agent ([`recv::to_sessions`]). Gives whether it ran; `None` once a
    /// run panicked.
    pub fn look(&mut self, dir: &Path, came: &Came) -> Option<bool> {
        let due = std::mem::take(&mut self.due);
        // A run for the agent of each session that ended issues what its file confirmed.
        let mut runs = BTreeMap::new();
        if due || std::mem::take(&mut self.reload) {
            runs.extend(self.load(dir));
        }
        let now = Instant::now();
        self.batches.retain(|agent, (folder, at)| {
            let ended = *at <= now;
            if ended {
                runs.insert(agent.clone(), folder.clone());
            }
            !ended
        });
        let retry = (self.failed).is_none_or(|failed| failed.elapsed() >= AFTER_FAILURE);
        let mut gone = Vec::new();
        let mut state = None;
        for (id, live) in &mut self.live {
            if !live.runs() {
                gone.push(id.clone());
                runs.insert(live.session.agent.clone(), live.session.folder.clone());
                continue;
            }
            let session = &live.session;
            let failed = live.receipts();
            let confirmed = live.scan.read(&session.file);
            let mut ran = due || (retry && came.concerns(Some(session.space)));
            if !failed.is_empty() || !confirmed.is_empty() {
                let Some(state) = opened(&mut state, dir) else {
                    continue;
                };
                for line in &failed {
                    let _ = state.not_taken(id, line);
                }
                ran |= state.confirmed(id, &confirmed).is_ok_and(|count| count > 0);
            }
            if ran {
                runs.insert(session.agent.clone(), session.folder.clone());
            }
        }
        for id in gone {
            if let Some(state) = opened(&mut state, dir) {
                let _ = state.end_claude(&id);
            }
            self.live.remove(&id);
        }
        drop(state);
        for (agent, folder) in &runs {
            match self.run(dir, folder, agent)? {
                true => self.failed = None,
                false => self.failed = Some(Instant::now()),
            }
            // A run ends a registration whose socket does not take a write.
            self.reload = true;
        }
        Some(!runs.is_empty())
    }

    /// Takes the registrations from the state in `dir`, keeping what it has of each it served;
    /// gives the agent and folder of each it served that ended.
    fn load(&mut self, dir: &Path) -> Vec<(String, PathBuf)> {
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
            .map(|ended| (ended.session.agent, ended.session.folder))
            .collect()
    }

    /// Runs as `agent` in its `folder`, as [`run_as`], noting when a window of `batch` it held
    /// messages back for ends.
    fn run(&mut self, dir: &Path, folder: &Path, agent: &str) -> Option<bool> {
        let ran = run_as(
            self.settings,
            dir,
            (folder, agent),
            |storage, settings, agent| recv::to_sessions(dir, storage, settings, agent).map(drop),
        );
        if ran == Some(true) {
            match recv::due() {
                Some(at) => {
                    let wait =
                        Duration::from_millis(at.saturating_sub(crate::state::unix_millis()));
                    let at = Instant::now() + wait;
                    self.batches
                        .insert(agent.to_owned(), (folder.to_owned(), at));
                }
                None => {
                    self.batches.remove(agent);
                }
            }
        }
        ran
    }
}

/// Runs `run` with `settings` as `agent` in its `folder`, its output dropped, then goes back to
/// `dir`, the helper's: whether the run did not fail; `None` if it panicked.
pub fn run_as(
    (storage, speaks, timeout): Settings,
    dir: &Path,
    (folder, agent): (&Path, &str),
    run: impl FnOnce(secrets::Storage, (VersionRange, Duration), &Name) -> Result<(), String>,
) -> Option<bool> {
    let Ok(agent) = crate::agents::name(agent) else {
        return Some(false);
    };
    let ran = panic::catch_unwind(AssertUnwindSafe(|| {
        output::into(
            |_, _| Ok(()),
            || {
                std::env::set_current_dir(folder).map_err(|err| err.to_string())?;
                run(storage, (speaks, timeout), &agent)
            },
        )
    }));
    let _ = std::env::set_current_dir(dir);
    ran.ok().map(|ran| ran.is_ok())
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
        let lines = |entries: &[&str]| {
            (entries.iter())
                .map(|entry| format!("{}\n", entry.trim()))
                .collect::<String>()
        };
        let mut scan = Scan::default();
        let contents = (lines(entries), lines(more));
        crate::session_file::read_twice("claude", (&contents.0, &contents.1), |file| {
            scan.read(file)
        })
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
