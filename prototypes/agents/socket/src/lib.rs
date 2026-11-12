//! Hands the waiting messages of a [`Source`] to a Claude Code session through the session's
//! cross-session inbox socket ([docs](https://code.claude.com/docs/en/cross-session-messaging.md#the-sessions-inbox-socket)):
//! the session's `SessionStart` hook registers the socket, `CLAUDE_CODE_MESSAGING_SOCKET`, and its
//! token, `CLAUDE_CODE_MESSAGING_TOKEN`, in the source ([`Registration`]).
//!
//! Each message goes as one line, `{"type":"user","message":{"role":"user","content":<text>},
//! "from":<sender>,"msg_id":<msg_id>,"session_id":<session>}`, its text that of [`render`]. With
//! the token, an auth line `{"type":"auth","token":<token>}` comes first: Claude Code then takes
//! the lines as from the session's own child, which a session that bypasses permission prompts
//! does not hold. The session drops a line whose `session_id` is not its own.
//!
//! With a reply address, a socket of this process next to the session's, the session sends back
//! `peer_message_status` receipts, `held`, `expired`, `refused`, `dropped`, `denied` or
//! `delivered` (a held message released): [`Receipts`] records each as the signal
//! `socket-<status>` of its message. An accepted message gets no receipt.
//!
//! It marks the messages handed over once their lines are written and puts them back if the write
//! fails, as `docs/protocol/delivery.md`, "Device to agent", says.

use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agent_inbox::render;
use agent_source::{Message, Registration, Source};
use serde_json::{Value, json};

/// The `from` of a message without a reply address.
pub const FROM: &str = "skald";

/// The receipts' statuses, as Claude Code 2.1.322 sends them.
pub const STATUSES: [&str; 6] = [
    "held",
    "denied",
    "expired",
    "delivered",
    "refused",
    "dropped",
];

/// The lines that carry `messages` to the registered session: an auth line if `auth` and the
/// registration has a token, then one line per message from `from`.
pub fn lines(registration: &Registration, messages: &[Message], auth: bool, from: &str) -> String {
    let mut lines = String::new();
    if let (true, Some(token)) = (auth, &registration.token) {
        lines.push_str(&format!("{}\n", json!({ "type": "auth", "token": token })));
    }
    for message in messages {
        let line = json!({
            "type": "user",
            "message": { "role": "user", "content": render(std::slice::from_ref(message)) },
            "from": from,
            "msg_id": message.msg_id,
            "session_id": registration.session_id,
        });
        lines.push_str(&format!("{line}\n"));
    }
    lines
}

/// The options `[--no-auth] [--listen <seconds>]` of a handover in `flags`: whether it sends the
/// session's messaging token, and for how long it keeps a reply address; none if `flags` are not
/// these options.
pub fn options(flags: &[&str]) -> Option<(bool, Option<Duration>)> {
    let (auth, listen) = match flags {
        [] => (true, None),
        ["--no-auth"] => (false, None),
        ["--listen", seconds] => (true, Some(*seconds)),
        ["--no-auth", "--listen", seconds] => (false, Some(*seconds)),
        _ => return None,
    };
    let listen = match listen.map(str::parse::<u64>) {
        None => None,
        Some(Ok(seconds)) => Some(Duration::from_secs(seconds)),
        Some(Err(_)) => return None,
    };
    Some((auth, listen))
}

/// Takes the waiting messages of `source` and writes them to the socket of the session it
/// registered, with the token if `auth`, from `from`. Returns the messages handed over; writes
/// nothing when none wait. Fails, leaving them waiting, without a registered socket.
pub fn hand_over(source: &Source, auth: bool, from: &str) -> io::Result<Vec<Message>> {
    let registration = registered(source)?;
    let socket = registration.socket.clone().ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "the registered session has no messaging socket",
        )
    })?;
    source.hand_over(|messages| {
        let mut stream = UnixStream::connect(&socket)?;
        stream.write_all(lines(&registration, messages, auth, from).as_bytes())?;
        stream.flush()?;
        stream.shutdown(std::net::Shutdown::Write)
    })
}

/// The session `source` registered.
pub fn registered(source: &Source) -> io::Result<Registration> {
    source.registration()?.ok_or_else(|| {
        io::Error::new(
            ErrorKind::NotFound,
            "no session registered: its SessionStart hook has not run",
        )
    })
}

/// A receipt from the session: its status and the `msg_id` of the message it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub status: String,
    pub msg_id: String,
    pub reason: Option<String>,
}

/// The receipt in `line`, a line the session sent to the reply address, if it is one.
pub fn receipt(line: &str) -> Option<Receipt> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value["type"] != "control" || value["action"] != "peer_message_status" {
        return None;
    }
    let status = value["status"].as_str()?;
    STATUSES.contains(&status).then(|| Receipt {
        status: status.to_owned(),
        msg_id: value["orig_msg_id"].as_str().unwrap_or_default().to_owned(),
        reason: value["reason"].as_str().map(String::from),
    })
}

/// A reply address: a socket of this process in the directory of the session's socket, the only
/// place Claude Code sends receipts to. It is removed when dropped.
pub struct Receipts {
    listener: UnixListener,
    path: PathBuf,
}

impl Receipts {
    /// Binds the reply address next to `socket`, the session's.
    pub fn bind(socket: &Path) -> io::Result<Self> {
        let dir = socket.parent().ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidInput, "the socket has no directory")
        })?;
        let path = dir.join(format!("skald-{}.sock", std::process::id()));
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, path })
    }

    /// The `from` of messages sent with this reply address.
    pub fn address(&self) -> String {
        format!("uds:{}", self.path.display())
    }

    /// Takes the receipts that come until `until` and records each in `source` as the signal
    /// `socket-<status>` of its message, its reason as the detail; calls `seen` with each.
    pub fn collect(
        &self,
        source: &Source,
        until: Instant,
        mut seen: impl FnMut(&Receipt),
    ) -> io::Result<()> {
        while !until.saturating_duration_since(Instant::now()).is_zero() {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    // macOS refuses these on a connection its peer already closed; what it sent
                    // is still read.
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    for line in BufReader::new(stream).lines().map_while(Result::ok) {
                        let Some(receipt) = receipt(&line) else {
                            continue;
                        };
                        // A receipt for a message this source does not have names no record.
                        if source.state(&receipt.msg_id)?.is_some() {
                            source.record(
                                &receipt.msg_id,
                                &format!("socket-{}", receipt.status),
                                receipt.reason.as_deref(),
                            )?;
                        }
                        seen(&receipt);
                    }
                }
                Err(error) => {
                    if error.kind() != ErrorKind::WouldBlock {
                        return Err(error);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        Ok(())
    }
}

impl Drop for Receipts {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use agent_source::State;
    use agent_testing::{message, one_waiting};

    use super::*;

    fn registration(socket: Option<&Path>) -> Registration {
        Registration {
            session_id: "3b19".to_owned(),
            socket: socket.map(|path| path.to_string_lossy().into_owned()),
            token: Some("child-token".to_owned()),
            at_ms: 1,
            ..Registration::default()
        }
    }

    #[test]
    fn writes_an_auth_line_then_a_line_per_message() {
        let messages = [message("m1"), message("m2")];
        let text = lines(&registration(None), &messages, true, "uds:/tmp/r.sock");
        let lines: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines[0], json!({ "type": "auth", "token": "child-token" }));
        for (line, message) in lines[1..].iter().zip(&messages) {
            assert_eq!(
                *line,
                json!({
                    "type": "user",
                    "message": { "role": "user", "content": render(std::slice::from_ref(message)) },
                    "from": "uds:/tmp/r.sock",
                    "msg_id": message.msg_id,
                    "session_id": "3b19",
                })
            );
        }
        assert_eq!(lines.len(), 3);
        // Without auth, or without a token, no auth line.
        let plain = lines_of(&registration(None), &messages, false);
        assert!(plain.iter().all(|line| line["type"] == "user"));
        let tokenless = Registration {
            token: None,
            ..registration(None)
        };
        assert_eq!(lines_of(&tokenless, &messages, true).len(), 2);
    }

    fn lines_of(registration: &Registration, messages: &[Message], auth: bool) -> Vec<Value> {
        lines(registration, messages, auth, FROM)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// A source in `dir` with `socket` registered, if given, whose message the handover leaves
    /// waiting.
    fn unsent(dir: &Path, socket: Option<&Path>) -> (Source, Message) {
        let (source, added) = one_waiting(&dir.join("source"));
        if let Some(socket) = socket {
            source.register(&registration(Some(socket))).unwrap();
        }
        assert!(hand_over(&source, true, FROM).is_err());
        assert_eq!(source.state(&added.msg_id).unwrap(), Some(State::Waiting));
        (source, added)
    }

    #[test]
    fn hands_the_messages_to_the_registered_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("1.sock");
        let listener = UnixListener::bind(&path).unwrap();
        // Nothing registered yet: the messages wait.
        let (source, added) = unsent(dir.path(), None);
        source
            .register(&Registration {
                socket: None,
                ..registration(None)
            })
            .unwrap();
        assert!(hand_over(&source, true, FROM).is_err());
        source.register(&registration(Some(&path))).unwrap();

        let reader = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            BufReader::new(stream)
                .lines()
                .map(Result::unwrap)
                .collect::<Vec<_>>()
        });
        let handed = hand_over(&source, false, FROM).unwrap();
        assert_eq!(handed.len(), 1);
        let got = reader.join().unwrap();
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].contains(&added.msg_id));
        assert!(matches!(
            source.state(&added.msg_id).unwrap(),
            Some(State::HandedOver { .. })
        ));
        // None waiting: nothing is written, so no connection is needed.
        assert!(hand_over(&source, true, FROM).unwrap().is_empty());
    }

    #[test]
    fn a_failed_write_leaves_the_messages_waiting() {
        let dir = tempfile::tempdir().unwrap();
        unsent(dir.path(), Some(&dir.path().join("gone.sock")));
    }

    // A receipt recorded from Claude Code 2.1.322.
    const HELD: &str = r#"{"type":"control","action":"peer_message_status","status":"held","reason":"Your message is held for the recipient user's approval before it reaches their Claude session (permission-mode parity).","from":"uds:/tmp/cc-socks/59250.sock","orig_msg_id":"95500005-ab10-470c-8898-fb85bcd33e1a","msgV":1,"msg_id":"92ca6fd9-7147-4432-bf4c-592dff73b251"}"#;

    #[test]
    fn reads_the_receipts() {
        let held = receipt(HELD).unwrap();
        assert_eq!(held.status, "held");
        assert_eq!(held.msg_id, "95500005-ab10-470c-8898-fb85bcd33e1a");
        assert!(held.reason.unwrap().starts_with("Your message is held"));
        for status in STATUSES {
            let line = HELD.replace("\"held\"", &format!("\"{status}\""));
            assert_eq!(receipt(&line).unwrap().status, status);
        }
        for other in [
            HELD.replace("\"held\"", "\"read\""),
            HELD.replace("peer_message_status", "peer_idle_notice"),
            HELD.replace("\"control\"", "\"user\""),
            "not JSON".to_owned(),
        ] {
            assert_eq!(receipt(&other), None, "{other}");
        }
    }

    #[test]
    fn records_the_receipts_at_the_reply_address() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("1.sock");
        let receipts = Receipts::bind(&socket).unwrap();
        let address = receipts.address();
        let path = PathBuf::from(address.strip_prefix("uds:").unwrap());
        assert_eq!(path.parent(), Some(dir.path()));
        assert!(path.to_string_lossy().ends_with(".sock"));
        let (source, added) = one_waiting(&dir.path().join("source"));
        let sender = {
            let path = path.clone();
            let line = HELD.replace("95500005-ab10-470c-8898-fb85bcd33e1a", &added.msg_id);
            thread::spawn(move || {
                let mut stream = UnixStream::connect(path).unwrap();
                writeln!(stream, "{line}").unwrap();
                writeln!(stream, "{}", line.replace(&added.msg_id, "unknown")).unwrap();
                writeln!(stream, "noise").unwrap();
            })
        };
        let mut seen = Vec::new();
        receipts
            .collect(
                &source,
                Instant::now() + Duration::from_millis(500),
                |receipt| seen.push(receipt.msg_id.clone()),
            )
            .unwrap();
        sender.join().unwrap();
        assert_eq!(seen.len(), 2, "{seen:?}");
        let records = source.records(&seen[0]).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].signal, "socket-held");
        drop(receipts);
        assert!(!path.exists(), "the reply address is removed");
    }
}
