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
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::Shutdown;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use skald_proto::{Name, SpaceId, VersionRange};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

use crate::devices::text;
use crate::mailbox::Came;
use crate::session_file::{Reader, opened};
use crate::state::{ClaudeSession, ClaudeStopDelivery, State};
use crate::{hook, output, recv, resend, secrets};

/// The statuses of the receipts that say a line did not reach the model.
const FAILING: [&str; 5] = ["held", "expired", "refused", "dropped", "denied"];

/// How long a write to a session's socket, and a read of a receipt, may take.
const PATIENCE: Duration = Duration::from_secs(5);

/// How long the helper waits after a failed run before what comes starts another.
const AFTER_FAILURE: Duration = Duration::from_secs(5);

/// How often the helper checks that a registered session's socket takes a connection.
const ALIVE_EVERY: Duration = Duration::from_secs(2);

/// How long after Stop the helper looks for the assistant entry the hook missed.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// The most transcript bytes a deferred confirmation reads at a time.
const STOP_TAIL: u64 = 64 << 10;
/// The longest transcript line a deferred confirmation reads: a request blob Claude Code writes
/// beside the conversation can carry all of it, several MiB.
const STOP_LINE: u64 = 32 << 20;
/// How many bookkeeping lines a deferred confirmation passes over between a delivery and its
/// reply, on either side of Stop ([`StopHead::bookkeeping`]).
const STOP_SKIPS: usize = 1024;
/// How many Stops the helper can wait for together.
const STOP_WATCHES: usize = 16;
/// How many emitted own lines the helper can remember without retaining their text.
const STOP_EMISSIONS: usize = 256;

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

/// The single delivery whose final assistant entry may finish writing after Stop returns.
pub struct DeferredStop {
    space: SpaceId,
    agent: String,
    folder: PathBuf,
    path: resend::Way,
    reference: String,
    digest: [u8; 32],
    transcript: StopFile,
    shown: i64,
    started: Instant,
}

struct ConfirmingStop {
    stop: DeferredStop,
    delivery: ClaudeStopDelivery,
    until: Instant,
}

struct Emission {
    space: SpaceId,
    agent: String,
    path: resend::Way,
    /// A rewake hook knows its session ID. A Channels holder identifies the agent and space.
    session: Option<String>,
    delivery: ClaudeStopDelivery,
    reference: String,
    digest: [u8; 32],
    sent: i64,
    until: Instant,
    first_stop: Option<Instant>,
}

#[derive(Default)]
struct Emissions {
    records: Vec<Emission>,
    /// When saturated, none of the attempts we could not keep may be selected.
    unavailable_until: Option<Instant>,
}

impl Emissions {
    fn record(&mut self, now: Instant, emission: Emission) {
        self.records.retain(|record| record.until > now);
        let unavailable = self.unavailable_until.filter(|until| *until > now);
        if unavailable.is_some() || self.records.len() == STOP_EMISSIONS {
            self.unavailable_until = self
                .records
                .drain(..)
                .map(|record| record.until)
                .chain(unavailable)
                .chain([emission.until])
                .max();
            return;
        }
        self.records.push(emission);
    }

    fn bind(&mut self, session: &str, stop: DeferredStop, now: Instant) -> Option<ConfirmingStop> {
        if self.unavailable_until.is_some_and(|until| until > now) {
            return None;
        }
        self.records.retain(|record| record.until > now);
        let mut emitted = self.records.iter_mut().filter(|record| {
            record.space == stop.space
                && record.agent == stop.agent
                && record.path == stop.path
                && record.reference == stop.reference
                && record.sent <= stop.shown
                && record.session.as_deref().is_none_or(|id| id == session)
        });
        let record = emitted.next()?;
        if emitted.next().is_some() || record.digest != stop.digest {
            return None;
        }
        let first_stop = *record.first_stop.get_or_insert(stop.started);
        let until = record.until.min(first_stop + STOP_GRACE);
        (now < until).then(|| ConfirmingStop {
            stop,
            delivery: record.delivery.clone(),
            until,
        })
    }
}

impl ConfirmingStop {
    /// None discards a watch, false means it still waits, and true confirms the exact handover.
    fn poll(&self, state: &State, id: &str) -> Result<Option<bool>, crate::state::Error> {
        let stop = &self.stop;
        let scope = (stop.space, stop.agent.as_str(), id);
        let path = stop.path.name();
        if !state.awaits_claude_stop(scope, path, &self.delivery, stop.shown)? {
            return Ok(None);
        }
        match stop.transcript.assistant(id) {
            Some(true) => Ok(state
                .confirmed_claude_stop(scope, path, &self.delivery, stop.shown, self.until)?
                .then_some(true)),
            waiting => Ok(waiting),
        }
    }
}

/// A delivery line, the bookkeeping lines after it, and the exact place the next line starts in
/// this file.
struct StopFile {
    file: PathBuf,
    identity: (u64, u64),
    anchor: u64,
    next: u64,
    anchor_hash: [u8; 32],
    parent: Option<String>,
    turn: Option<String>,
    prefix: Option<(usize, [u8; 32])>,
    identified_reply: bool,
}

/// Watches only a complete delivery at the transcript tail and an identifiable continuation.
pub fn defer_stop(
    state: &State,
    (space, agent): (SpaceId, &Name),
    session: &hook::Session,
    turn: Option<&str>,
) -> Option<DeferredStop> {
    let id = session.id.as_deref()?;
    let registered = state.claude_sessions().ok()?;
    let file = if let Some(registered) = registered.iter().find(|each| each.session == id) {
        if registered.space != space || registered.agent != agent.as_str() {
            return None;
        }
        &registered.file
    } else {
        session.file.as_ref()?
    };
    let started = Instant::now();
    let file = std::path::absolute(file).ok()?;
    let (mut transcript, entry) = StopFile::at_tail(&file)?;
    let expected_turn = turn.or(entry.turn.as_deref());
    if entry.kind != "user"
        || !entry.in_session(id)
        || !same_stop_turn(
            expected_turn,
            entry.turn.as_deref(),
            transcript.identified_reply,
        )
    {
        return None;
    }
    transcript.turn = expected_turn.map(str::to_owned);
    let said = entry.message.filter(|said| said.role == "user")?;
    let at = entry
        .timestamp?
        .parse::<jiff::Timestamp>()
        .ok()?
        .as_millisecond();
    let (path, reference, digest) = stop_reference(&text_of(&said.content))?;
    if at > crate::state::unix_ms() {
        return None;
    }
    Some(DeferredStop {
        space,
        agent: agent.as_str().to_owned(),
        folder: std::env::current_dir().ok()?,
        path,
        reference,
        digest,
        transcript,
        shown: at,
        started,
    })
}

/// A single reference in the delivery itself, with the path that gave the line.
fn stop_reference(text: &str) -> Option<(resend::Way, String, [u8; 32])> {
    let text = text.trim();
    let (path, body) = if let Some(event) = text.strip_prefix(CHANNEL_TAG) {
        let (source, rest) = event.split_once('"')?;
        if source != "skald" && !source.ends_with(":skald") {
            return None;
        }
        let (_, body) = rest.split_once('>')?;
        let body = body.strip_suffix(CHANNEL_END)?;
        if body.contains(CHANNEL_TAG) || body.contains(CHANNEL_END) {
            return None;
        }
        (resend::Way::Channel, body.trim())
    } else {
        let text = text
            .strip_prefix("<task-notification>")
            .and_then(|rest| rest.split_once("</task-notification>"))
            .map_or(text, |(_, rest)| rest.trim_start());
        let text = text
            .strip_prefix("<system-reminder>")
            .and_then(|body| body.strip_suffix("</system-reminder>"))
            .unwrap_or(text);
        (resend::Way::Rewake, rewake_output(text.trim())?)
    };
    let line = body.lines().next()?;
    let (header, _) = line.strip_prefix('[')?.split_once("] #")?;
    if !header.ends_with(" · own") {
        return None;
    }
    let reference = own_reference(line)?;
    (reference.len() == crate::forms::WHOLE_REFERENCE_LEN)
        .then(|| (path, reference, Sha256::digest(body.as_bytes()).into()))
}

/// The output in `text`, an `asyncRewake` hook's blocking error as Claude Code words it:
/// `<event> hook blocking error from command "<command>": <output>`.
fn rewake_output(text: &str) -> Option<&str> {
    let (event, rest) = text.split_once(REWAKE_OUTPUT)?;
    if event.trim_end().contains(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let rest = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split_once("\":")?.1,
        None => rest,
    };
    Some(
        rest.trim_start_matches([':', ' ', '\r', '\n', '\t'])
            .trim_end(),
    )
}

/// Without an exact partial line, both entries have to name the same turn explicitly.
fn same_stop_turn(expected: Option<&str>, found: Option<&str>, partial: bool) -> bool {
    match (expected, found) {
        (Some(expected), Some(found)) => expected == found,
        _ => partial,
    }
}

impl StopFile {
    /// The delivery must be the last complete entry of the conversation when Stop reads this file:
    /// only bookkeeping lines may follow it ([`StopHead::bookkeeping`]), up to [`STOP_SKIPS`].
    fn at_tail(file: &Path) -> Option<(StopFile, StopEntry)> {
        let mut opened = open_transcript(file)?;
        let identity = file_identity(&opened).ok()?;
        let length = opened.metadata().ok()?.len();
        let next = line_start(&mut opened, length)?;
        let incomplete = read_range(&mut opened, next, length)?;
        let mut end = next;
        let mut skipped = 0;
        let (anchor, entry) = loop {
            if end == 0 || skipped > STOP_SKIPS {
                return None;
            }
            let start = line_start(&mut opened, end - 1)?;
            let line = read_range(&mut opened, start, end)?;
            if !serde_json::from_slice::<StopHead>(&line)
                .ok()?
                .bookkeeping()
            {
                break (start, serde_json::from_slice::<StopEntry>(&line).ok()?);
            }
            skipped += 1;
            end = start;
        };
        let parent = entry.uuid.clone();
        let prefix = (!incomplete.is_empty())
            .then(|| (incomplete.len(), Sha256::digest(&incomplete).into()));
        let identified_reply = partial_assistant(&incomplete, &entry);
        if parent.is_none() && !identified_reply {
            return None;
        }
        let found = StopFile {
            file: file.to_path_buf(),
            identity,
            anchor,
            next,
            anchor_hash: hash_range(&mut opened, anchor, next)?,
            parent,
            turn: entry.turn.clone(),
            prefix,
            identified_reply,
        };
        Some((found, entry))
    }

    /// None means the file or its next entry no longer matches, false means it is still incomplete.
    fn assistant(&self, session: &str) -> Option<bool> {
        let mut opened = open_transcript(&self.file)?;
        if file_identity(&opened).ok()? != self.identity {
            return None;
        }
        let length = opened.metadata().ok()?.len();
        let observed = self.next + self.prefix.map_or(0, |(len, _)| len as u64);
        if length < observed {
            return None;
        }
        if length == observed {
            return Some(false);
        }
        if hash_range(&mut opened, self.anchor, self.next)? != self.anchor_hash {
            return None;
        }
        opened.seek(SeekFrom::Start(self.next)).ok()?;
        let mut after = BufReader::new(opened);
        // The bookkeeping lines Claude Code writes before its reply are passed over.
        for index in 0..=STOP_SKIPS {
            let mut line = Vec::new();
            (&mut after)
                .take(STOP_LINE)
                .read_until(b'\n', &mut line)
                .ok()?;
            if index == 0
                && self.prefix.is_some_and(|(len, hash)| {
                    line.get(..len)
                        .is_none_or(|prefix| <[u8; 32]>::from(Sha256::digest(prefix)) != hash)
                })
            {
                return None;
            }
            if !line.ends_with(b"\n") {
                return (line.len() < STOP_LINE as usize).then_some(false);
            }
            if serde_json::from_slice::<StopHead>(&line)
                .ok()?
                .bookkeeping()
            {
                continue;
            }
            let entry: StopEntry = serde_json::from_slice(&line).ok()?;
            return (entry.assistant_in(session)
                && self.replies_in_turn(entry.turn.as_deref())
                && self
                    .parent
                    .as_ref()
                    .is_none_or(|parent| entry.parent.as_ref() == Some(parent)))
            .then_some(true);
        }
        None
    }

    /// Whether a reply that names the turn `found`, or none, answers the delivery's turn. Claude
    /// Code's reply names none: its parent, the delivery's own entry, ties it to the turn.
    fn replies_in_turn(&self, found: Option<&str>) -> bool {
        match (self.turn.as_deref(), found) {
            (Some(expected), Some(found)) => expected == found,
            (_, None) => self.parent.is_some() || self.identified_reply,
            (None, Some(_)) => self.identified_reply,
        }
    }
}

/// An already identifiable reply can still lack the final object brace or newline.
fn partial_assistant(prefix: &[u8], delivery: &StopEntry) -> bool {
    let Some(session) = delivery.session.as_deref() else {
        return false;
    };
    if prefix.is_empty() {
        return false;
    }
    let entry = serde_json::from_slice::<StopEntry>(prefix).or_else(|_| {
        let mut object = prefix.to_vec();
        object.push(b'}');
        serde_json::from_slice::<StopEntry>(&object)
    });
    entry.is_ok_and(|entry| {
        entry.assistant_in(session)
            && delivery
                .uuid
                .as_ref()
                .is_none_or(|id| entry.parent.as_ref() == Some(id))
            && same_stop_turn(delivery.turn.as_deref(), entry.turn.as_deref(), true)
    })
}

/// Where the line that ends at `before` in `file` starts: just after the newline before it, or at
/// the file's start; None if it is longer than [`STOP_LINE`].
fn line_start(file: &mut fs::File, before: u64) -> Option<u64> {
    let floor = before.saturating_sub(STOP_LINE);
    let mut at = before;
    let mut chunk = vec![0; usize::try_from(STOP_TAIL).ok()?];
    while at > floor {
        let from = at.saturating_sub(STOP_TAIL).max(floor);
        let read = &mut chunk[..usize::try_from(at - from).ok()?];
        file.seek(SeekFrom::Start(from)).ok()?;
        file.read_exact(read).ok()?;
        if let Some(newline) = read.iter().rposition(|&byte| byte == b'\n') {
            return Some(from + newline as u64 + 1);
        }
        at = from;
    }
    (at == 0).then_some(0)
}

/// The bytes of `file` from `start` to `end`.
fn read_range(file: &mut fs::File, start: u64, end: u64) -> Option<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(end.checked_sub(start)?).ok()?];
    file.seek(SeekFrom::Start(start)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

/// The digest of the bytes of `file` from `start` to `end`, read [`STOP_TAIL`] at a time.
fn hash_range(file: &mut fs::File, start: u64, end: u64) -> Option<[u8; 32]> {
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut digest = Sha256::new();
    let mut chunk = vec![0; usize::try_from(STOP_TAIL).ok()?];
    let mut left = end.checked_sub(start)?;
    while left > 0 {
        let read = &mut chunk[..usize::try_from(left.min(STOP_TAIL)).ok()?];
        file.read_exact(read).ok()?;
        digest.update(&*read);
        left -= read.len() as u64;
    }
    Some(digest.finalize().into())
}

fn open_transcript(file: &Path) -> Option<fs::File> {
    if !fs::metadata(file).ok()?.is_file() {
        return None;
    }
    fs::File::open(file).ok()
}

#[cfg(unix)]
pub fn file_identity(file: &fs::File) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
pub fn file_identity(file: &fs::File) -> io::Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    // The open file keeps this handle valid, and the call fills the information structure.
    let mut found: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut found) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let index = (u64::from(found.nFileIndexHigh) << 32) | u64::from(found.nFileIndexLow);
    Ok((u64::from(found.dwVolumeSerialNumber), index))
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

/// The entries next to Stop, including the session and parent information for a deferred check.
#[derive(Deserialize)]
struct StopEntry {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "isSidechain", default)]
    sidechain: bool,
    #[serde(rename = "isApiErrorMessage", default)]
    api_error: bool,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    message: Option<StopMessage>,
    #[serde(rename = "sessionId", default)]
    session: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(rename = "parentUuid", default)]
    parent: Option<String>,
    #[serde(rename = "promptId", default)]
    turn: Option<String>,
}

/// What tells a bookkeeping line from an entry of the conversation, leaving the rest unread.
#[derive(Deserialize)]
struct StopHead {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    uuid: Option<serde::de::IgnoredAny>,
}

impl StopHead {
    /// A line Claude Code writes beside the conversation, such as `api-request` or `last-prompt`:
    /// no entry of the conversation's type, and no `uuid` to join its chain.
    fn bookkeeping(&self) -> bool {
        self.uuid.is_none()
            && !matches!(
                self.kind.as_str(),
                "user" | "assistant" | "attachment" | "system"
            )
    }
}

impl StopEntry {
    fn in_session(&self, session: &str) -> bool {
        !self.sidechain && self.session.as_deref() == Some(session)
    }

    fn assistant_in(&self, session: &str) -> bool {
        self.in_session(session)
            && self.kind == "assistant"
            && !self.api_error
            && self
                .message
                .as_ref()
                .is_some_and(|said| said.role == "assistant")
    }
}

#[derive(Deserialize)]
struct StopMessage {
    role: String,
    #[serde(default)]
    content: serde_json::Value,
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
}

/// What it reads confirms the `msg_id` of each message the model got: one an entry that went to the
/// model holds, a user entry or, for a line queued during a turn, its attachment, once a later
/// entry is the model's own, a reply, a tool call or reasoning, not an API error. The references
/// such entries name are confirmed so too ([`Scan::references`]).
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
    deferred: BTreeMap<String, ConfirmingStop>,
    emitted: Emissions,
    /// When the last run failed, if it did.
    failed: Option<Instant>,
    /// The paths the helper held at the last look.
    held: Holders,
    /// When, in Unix milliseconds, the first message a path took times out, if any does.
    next: Option<i64>,
    /// Of each agent and space whose last run held messages back under `batch`, its folder and
    /// when the first such window ends ([`resend::Pushed::batch`]).
    batches: BTreeMap<(String, SpaceId), (PathBuf, Instant)>,
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
            deferred: BTreeMap::new(),
            emitted: Emissions::default(),
            failed: None,
            held: Holders::new(),
            next: None,
            batches: BTreeMap::new(),
        }
    }

    /// Notes that a command ran.
    pub fn ran(&mut self) {
        self.due = true;
    }

    /// Remembers the identity of a single own message the selected path just wrote.
    pub fn emitted(
        &mut self,
        (space, agent): (SpaceId, &str),
        (path, session, started): (resend::Way, Option<&str>, i64),
        push: &resend::Push,
    ) {
        let Some(own) = &push.own else { return };
        let now = Instant::now();
        let remaining = own.due.saturating_sub(crate::state::unix_ms());
        let Some(until) = u64::try_from(remaining)
            .ok()
            .and_then(|ms| now.checked_add(Duration::from_millis(ms)))
        else {
            return;
        };
        self.emitted.record(
            now,
            Emission {
                space,
                agent: agent.to_owned(),
                path,
                session: session.map(str::to_owned),
                delivery: own.clone(),
                reference: crate::forms::whole_reference(own.msg_id),
                digest: Sha256::digest(push.content.trim().as_bytes()).into(),
                sent: started,
                until,
                first_stop: None,
            },
        );
    }

    /// A hook ends any previous watch of its session, or replaces it after Stop.
    pub fn stopped(&mut self, id: String, deferred: Option<DeferredStop>) {
        self.deferred.remove(&id);
        if self.deferred.len() < STOP_WATCHES
            && let Some(watch) =
                deferred.and_then(|stop| self.emitted.bind(&id, stop, Instant::now()))
        {
            self.deferred.insert(id, watch);
        }
    }

    /// Whether a session is registered, or the helper holds a path, as of the last look.
    pub fn serving(&self) -> bool {
        !self.live.is_empty() || !self.held.is_empty() || !self.deferred.is_empty()
    }

    /// Whether the next look has anything to do: a session is registered, or may be, or a message
    /// a path took waits for its timeout.
    pub fn looks(&self) -> bool {
        self.due || self.reload || self.serving() || self.next.is_some() || !self.batches.is_empty()
    }

    /// Serves the sessions registered in the state in `dir`, and those whose paths the helper
    /// holds, `held`, `came` what came on the connection: ends those whose socket is gone or takes
    /// no connection; records the receipts that came and what the session files show and confirm;
    /// then, for each agent that has a confirmation, a session that ended, a path that came, a
    /// message whose timeout passed or a window of `batch` that ended, whose space `came` concerns,
    /// or for each after a command ran, runs as the agent ([`recv::to_sessions`]). Gives whether
    /// it ran, and what the runs gave the paths the helper holds; `None` once a run panicked.
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
        let now = Instant::now();
        self.batches.retain(|key, (folder, at)| {
            let ended = *at <= now;
            if ended {
                runs.insert(key.clone(), folder.clone());
            }
            !ended
        });
        let retry = (self.failed).is_none_or(|failed| failed.elapsed() >= AFTER_FAILURE);
        let mut gone = Vec::new();
        let mut state = None;
        if !self.deferred.is_empty() {
            let stopped = State::open_deferred_stop(dir);
            self.deferred.retain(|id, deferred| {
                if Instant::now() >= deferred.until {
                    return false;
                }
                let polled = match &stopped {
                    Ok(state) => deferred.poll(state, id),
                    Err(err) => return err.busy(),
                };
                match polled {
                    Ok(Some(true)) => {
                        let stop = &deferred.stop;
                        runs.insert((stop.agent.clone(), stop.space), stop.folder.clone());
                        false
                    }
                    Ok(Some(false)) => true,
                    Err(err) => err.busy() && Instant::now() < deferred.until,
                    Ok(None) => false,
                }
            });
        }
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
                    let key = (agent.clone(), *space);
                    match pushed.batch {
                        Some(at) => {
                            let wait = at.saturating_sub(crate::state::unix_millis());
                            let at = Instant::now() + Duration::from_millis(wait);
                            self.batches.insert(key, (folder.clone(), at));
                        }
                        None => {
                            self.batches.remove(&key);
                        }
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

    /// Runs as `agent` in its `folder`, as [`run_as`], by the paths `held` the helper holds: what
    /// the run gave the paths, `None` if it failed; `None` if it panicked.
    fn run(
        &self,
        dir: &Path,
        folder: &Path,
        agent: &str,
        held: resend::Held,
    ) -> Option<Option<resend::Pushed>> {
        let mut pushed = None;
        let ran = run_as(
            self.settings,
            dir,
            (folder, agent),
            |storage, settings, agent| {
                let held = (held, resend::resend_after());
                pushed = Some(recv::to_sessions(dir, storage, settings, agent, held)?);
                Ok(())
            },
        )?;
        Some(pushed.filter(|_| ran))
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

    fn write_delivery_entry(dir: &Path) -> PathBuf {
        let file = dir.join("session.jsonl");
        let delivery = json!({
            "type": "user", "sessionId": "session", "uuid": "delivery", "promptId": "turn",
        });
        fs::write(&file, format!("{delivery}\n")).unwrap();
        file
    }

    /// Claude Code's reply names no turn: its parent, the delivery, ties it to the turn.
    #[test]
    fn a_deferred_stop_only_accepts_its_adjacent_assistant_entry() {
        let reply = json!({
            "type": "assistant", "sessionId": "session", "parentUuid": "delivery",
            "promptId": "turn", "message": { "role": "assistant" },
        });
        let appends = [
            (reply.clone(), Some(true)),
            (json!({ "isApiErrorMessage": true }), None),
            (json!({ "isSidechain": true }), None),
            (json!({ "sessionId": "another" }), None),
            (json!({ "parentUuid": "another" }), None),
            (json!({ "promptId": "another" }), None),
            (json!({ "promptId": null }), Some(true)),
            (json!({ "promptId": null, "parentUuid": null }), None),
            (json!({ "type": "user" }), None),
        ];
        for (index, (fields, expected)) in appends.into_iter().enumerate() {
            let dir = crate::test_dir::TestDir::new(&format!("deferred-claude-stop-{index}"));
            let file = write_delivery_entry(&dir);
            let (watch, _) = StopFile::at_tail(&file).unwrap();
            assert_eq!(watch.assistant("session"), Some(false));
            let mut appended = reply.clone();
            appended
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let mut output = fs::OpenOptions::new().append(true).open(&file).unwrap();
            writeln!(output, "{appended}\n{reply}").unwrap();
            assert_eq!(watch.assistant("session"), expected, "{appended}");
        }
    }

    /// As Claude Code 2.1.332 writes them: bookkeeping lines without a `uuid` follow the delivery,
    /// before and after Stop, and its reply follows them, naming no turn. A conversation entry
    /// between them, or a delivery a conversation entry follows at Stop, confirms nothing.
    #[test]
    fn a_deferred_stop_passes_over_bookkeeping_lines() {
        let line = |entry: serde_json::Value| format!("{entry}\n");
        let blob = line(json!({ "type": "api-request-blob", "sessionId": "session" }));
        let request = line(json!({ "type": "api-request", "sessionId": "session" }));
        let reply = line(json!({
            "type": "assistant", "sessionId": "session", "uuid": "reply",
            "parentUuid": "delivery", "message": { "role": "assistant" },
        }));
        let attachment = line(json!({
            "type": "attachment", "sessionId": "session", "uuid": "attached",
            "parentUuid": "delivery",
        }));
        let unknown = line(json!({ "type": "new-kind", "sessionId": "session", "uuid": "new" }));
        let cases = [
            (vec![], Some(true)),
            (vec![unknown.as_str()], None),
            (vec![attachment.as_str()], None),
        ];
        for (index, (between, expected)) in cases.into_iter().enumerate() {
            let dir =
                crate::test_dir::TestDir::new(&format!("deferred-claude-bookkeeping-{index}"));
            let file = write_delivery_entry(&dir);
            let mut output = fs::OpenOptions::new().append(true).open(&file).unwrap();
            output.write_all(blob.as_bytes()).unwrap();
            let (watch, entry) = StopFile::at_tail(&file).unwrap();
            assert_eq!(entry.uuid.as_deref(), Some("delivery"));
            assert_eq!(watch.assistant("session"), Some(false));
            output.write_all(request.as_bytes()).unwrap();
            assert_eq!(watch.assistant("session"), Some(false));
            for line in &between {
                output.write_all(line.as_bytes()).unwrap();
            }
            output.write_all(reply.as_bytes()).unwrap();
            assert_eq!(watch.assistant("session"), expected, "{between:?}");
        }
        let dir = crate::test_dir::TestDir::new("deferred-claude-bookkeeping-attached");
        let file = write_delivery_entry(&dir);
        fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(format!("{attachment}{blob}").as_bytes())
            .unwrap();
        let (_, entry) = StopFile::at_tail(&file).unwrap();
        assert_eq!(entry.kind, "attachment");
    }

    /// Request blobs far longer than [`STOP_TAIL`], as Claude Code writes them with the whole
    /// conversation, between the delivery and Stop and between Stop and the reply, are passed
    /// over; more than [`STOP_SKIPS`] bookkeeping lines before Stop confirm nothing.
    #[test]
    fn a_deferred_stop_passes_over_bookkeeping_lines_longer_than_its_reads() {
        let blob = |size: usize| {
            let text = "x".repeat(size);
            let message = json!({ "role": "user", "content": [{ "type": "text", "text": text }] });
            let entry = json!({
                "type": "api-request-blob", "sessionId": "session", "hash": "h", "message": message,
            });
            format!("{entry}\n")
        };
        let long = usize::try_from(3 * STOP_TAIL).unwrap();
        let reply = format!(
            "{}\n",
            json!({
                "type": "assistant", "sessionId": "session", "uuid": "reply",
                "parentUuid": "delivery", "message": { "role": "assistant" },
            })
        );
        let dir = crate::test_dir::TestDir::new("deferred-claude-long-bookkeeping");
        let file = write_delivery_entry(&dir);
        let mut output = fs::OpenOptions::new().append(true).open(&file).unwrap();
        output.write_all(blob(long).as_bytes()).unwrap();
        let (watch, entry) = StopFile::at_tail(&file).unwrap();
        assert_eq!(entry.uuid.as_deref(), Some("delivery"));
        let after = blob(long);
        let (start, rest) = after.split_at(long / 2);
        output.write_all(start.as_bytes()).unwrap();
        assert_eq!(watch.assistant("session"), Some(false));
        output
            .write_all(format!("{rest}{reply}").as_bytes())
            .unwrap();
        assert_eq!(watch.assistant("session"), Some(true));

        let dir = crate::test_dir::TestDir::new("deferred-claude-many-bookkeeping");
        let file = write_delivery_entry(&dir);
        let many = blob(1).repeat(STOP_SKIPS + 1);
        fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(many.as_bytes())
            .unwrap();
        assert!(StopFile::at_tail(&file).is_none());
    }

    /// The deliveries of both paths, as Claude Code 2.1.332 puts them in the session file, name
    /// their own line's whole reference.
    #[test]
    fn a_stop_names_the_reference_of_a_channel_event_and_of_a_rewake_hook_s_output() {
        use crate::forms::whole_reference;
        let reference = whole_reference(skald_proto::MessageId::generate());
        let own = format!("[clienta/api · laptop · own] #{reference}: tag the release");
        let channel = format!("<channel source=\"skald\">\n{own}\n</channel>");
        let rewake = format!(
            "<task-notification>\n<summary>Stop hook feedback</summary>\n</task-notification>\n\
             <system-reminder>\nStop hook blocking error from command \"SessionStart:startup\": \
             {own}\n</system-reminder>"
        );
        let digest: [u8; 32] = Sha256::digest(own.as_bytes()).into();
        for (text, path) in [
            (channel, resend::Way::Channel),
            (rewake, resend::Way::Rewake),
        ] {
            let named = stop_reference(&text);
            assert_eq!(named, Some((path, reference.clone(), digest)), "{text}");
        }
        let typed = format!("please look at hook blocking error from command \"x\": {own}");
        assert_eq!(stop_reference(&typed), None);
    }

    #[test]
    fn deferred_stop_binds_only_to_one_emitted_identity_and_keeps_its_first_deadline() {
        let dir = crate::test_dir::TestDir::new("claude-stop-emissions");
        let file = write_delivery_entry(&dir);
        let now = Instant::now();
        let space = SpaceId::generate();
        let stop = |started| DeferredStop {
            space,
            agent: "claude".to_owned(),
            folder: dir.to_path_buf(),
            path: resend::Way::Channel,
            reference: "abcd".to_owned(),
            digest: [1; 32],
            transcript: StopFile::at_tail(&file).unwrap().0,
            shown: 3,
            started,
        };
        let record = |seq| Emission {
            space,
            agent: "claude".to_owned(),
            path: resend::Way::Channel,
            session: None,
            delivery: ClaudeStopDelivery {
                seq,
                msg_id: skald_proto::MessageId::generate(),
                due: 100,
            },
            reference: "abcd".to_owned(),
            digest: [1; 32],
            sent: 2,
            until: now + 2 * STOP_GRACE,
            first_stop: None,
        };
        let mut emitted = Emissions::default();
        assert!(emitted.bind("session", stop(now), now).is_none());

        let one = record(1);
        let delivery = one.delivery.clone();
        emitted.record(now, one);
        let mut different = stop(now);
        different.digest = [2; 32];
        assert!(emitted.bind("session", different, now).is_none());
        let first = emitted.bind("session", stop(now), now).unwrap();
        assert_eq!(first.delivery, delivery);

        let again = now + Duration::from_millis(100);
        let second = emitted.bind("session", stop(again), again).unwrap();
        assert_eq!(second.until, first.until);
        assert!(
            emitted
                .bind("session", stop(first.until), first.until)
                .is_none()
        );

        emitted.record(again, record(2));
        assert!(emitted.bind("session", stop(again), again).is_none());
    }

    #[test]
    fn a_short_reference_does_not_bind_a_later_delivery_with_the_same_prefix() {
        use crate::forms::{SHORTEST, whole_reference};
        use skald_proto::MessageId;

        let older = MessageId::from_bytes(973_u128.to_be_bytes());
        let newer = MessageId::from_bytes(1416_u128.to_be_bytes());
        let older_ref = whole_reference(older);
        let newer_ref = whole_reference(newer);
        let prefix = &older_ref[..SHORTEST];
        assert_eq!(prefix, &newer_ref[..SHORTEST]);
        let content = |reference: &str| format!("[api · device · own] #{reference}: same body");
        let event = |line: &str| format!("<channel source=\"skald\">\n{line}\n</channel>");
        assert!(stop_reference(&event(&content(prefix))).is_none());

        let dir = crate::test_dir::TestDir::new("claude-stop-reference-prefix");
        let file = write_delivery_entry(&dir);
        let now = Instant::now();
        let later = now + STOP_GRACE;
        let space = SpaceId::generate();
        let mut emissions = Emissions::default();
        for (seq, msg_id, reference, at) in [
            (1, older, older_ref.as_str(), now),
            (2, newer, newer_ref.as_str(), later),
        ] {
            emissions.record(
                at,
                Emission {
                    space,
                    agent: "claude".to_owned(),
                    path: resend::Way::Channel,
                    session: None,
                    delivery: ClaudeStopDelivery {
                        seq,
                        msg_id,
                        due: 100,
                    },
                    reference: reference.to_owned(),
                    digest: Sha256::digest(content(reference)).into(),
                    sent: 2,
                    until: at + STOP_GRACE,
                    first_stop: None,
                },
            );
        }
        let stopped = |reference: &str| {
            let (path, reference, digest) = stop_reference(&event(&content(reference))).unwrap();
            DeferredStop {
                space,
                agent: "claude".to_owned(),
                folder: dir.to_path_buf(),
                path,
                reference,
                digest,
                transcript: StopFile::at_tail(&file).unwrap().0,
                shown: 3,
                started: later,
            }
        };
        assert!(
            emissions
                .bind("session", stopped(&older_ref), later)
                .is_none()
        );
        let got = emissions
            .bind("session", stopped(&newer_ref), later)
            .unwrap();
        assert_eq!(got.delivery.msg_id, newer);
    }

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
