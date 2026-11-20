//! The relay of a Claude Code session's socket in the session's `skald mcp` (`crate::claude_socket`):
//!
//! - On Linux a session that bypasses permission prompts takes a line as its own only from a
//!   process that descends from the session's, whatever the token. The helper does not; the
//!   session's `skald mcp`, which Claude Code starts, does.
//! - `skald mcp`, given the session's socket and token, listens at [`path`], next to the socket,
//!   and at a reply address of its own, both readable only by the person, removing those of relays
//!   that ended without removing them ([`Relay::start`]).
//! - A connection that starts with the auth line of the session's token has its lines written
//!   into the session's socket, each `from` the relay's reply address, and gets [`TAKEN`] back.
//!   Any other connection gets nothing written.
//! - A receipt that comes to the relay's reply address goes on to the last reply address the
//!   relay replaced.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::hook;

/// What the relay answers once the session's socket took the lines.
pub const TAKEN: &str = "taken";

/// How long a read or write of a connection may take.
const PATIENCE: Duration = Duration::from_secs(5);

/// The relay's address for the session socket `socket`.
pub fn path(socket: &Path) -> PathBuf {
    let name = socket.file_name().unwrap_or_default().to_string_lossy();
    socket.with_file_name(format!("sk-{name}.relay"))
}

/// A listening relay; its addresses are removed on drop.
pub struct Relay {
    paths: Vec<PathBuf>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

impl Relay {
    /// The relay of the session whose socket and token Claude Code gave this process, if it gave
    /// both.
    pub fn from_settings() -> Option<io::Result<Relay>> {
        let socket = std::env::var_os(hook::SOCKET_SETTING)?;
        let token = std::env::var(hook::TOKEN_SETTING).ok()?;
        Some(Relay::start(PathBuf::from(socket), token))
    }

    /// Starts relaying to `socket` the lines of connections that present `token`.
    pub fn start(socket: PathBuf, token: String) -> io::Result<Relay> {
        let reply = socket.with_file_name(format!("sk-{}-relay.sock", std::process::id()));
        if let Some(dir) = socket.parent() {
            sweep(dir);
        }
        let mut relay = Relay { paths: Vec::new() };
        let taking = relay.bind(path(&socket))?;
        let receiving = relay.bind(reply.clone())?;
        let back = Arc::new(Mutex::new(None));
        let to = Arc::clone(&back);
        let from = format!("uds:{}", reply.display());
        thread::spawn(move || {
            for stream in taking.incoming().map_while(Result::ok) {
                let _ = take(&stream, &socket, &token, &from, &to);
            }
        });
        thread::spawn(move || {
            for stream in receiving.incoming().map_while(Result::ok) {
                let _ = pass_back(&stream, &back);
            }
        });
        Ok(relay)
    }

    fn bind(&mut self, path: PathBuf) -> io::Result<UnixListener> {
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        self.paths.push(path);
        fs::set_permissions(
            &self.paths[self.paths.len() - 1],
            fs::Permissions::from_mode(0o600),
        )?;
        Ok(listener)
    }
}

/// Removes the addresses in `dir` of relays that ended without removing them, as when killed.
fn sweep(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.map_while(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let relay = name.ends_with(".relay") || name.ends_with("-relay.sock");
        if name.starts_with("sk-")
            && relay
            && let Err(err) = UnixStream::connect(entry.path())
            && err.kind() == io::ErrorKind::ConnectionRefused
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Writes the lines `stream` sends into `socket` if they start with the auth line of `token`,
/// each `from` the relay's reply address, keeping the reply address it replaces in `back`.
fn take(
    stream: &UnixStream,
    socket: &Path,
    token: &str,
    from: &str,
    back: &Mutex<Option<PathBuf>>,
) -> io::Result<()> {
    let _ = stream.set_read_timeout(Some(PATIENCE));
    let _ = stream.set_write_timeout(Some(PATIENCE));
    let mut lines = BufReader::new(stream).lines();
    let auth = json!({ "type": "auth", "token": token });
    let first = lines.next().transpose()?.unwrap_or_default();
    if serde_json::from_str::<Value>(&first).ok() != Some(auth.clone()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "no auth line",
        ));
    }
    let mut text = format!("{auth}\n");
    let mut sender = None;
    for line in lines {
        let mut value: Value = serde_json::from_str(&line?)?;
        let Some(fields) = value.as_object_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a line is not an object",
            ));
        };
        let replaced = fields.insert("from".to_owned(), json!(from));
        if let Some(address) = replaced.as_ref().and_then(Value::as_str) {
            sender = address.strip_prefix("uds:").map(PathBuf::from).or(sender);
        }
        text.push_str(&format!("{value}\n"));
    }
    let mut session = UnixStream::connect(socket)?;
    session.set_write_timeout(Some(PATIENCE))?;
    session.write_all(text.as_bytes())?;
    let _ = session.shutdown(Shutdown::Write);
    if sender.is_some() {
        *back.lock().unwrap_or_else(PoisonError::into_inner) = sender;
    }
    let mut stream = stream;
    stream.write_all(TAKEN.as_bytes())
}

/// Passes what `stream` sends, the session's receipts, on to the reply address in `back`.
fn pass_back(stream: &UnixStream, back: &Mutex<Option<PathBuf>>) -> io::Result<()> {
    // On macOS a socket whose sender already closed takes no timeout; what it sent is still read.
    let _ = stream.set_read_timeout(Some(PATIENCE));
    let mut text = Vec::new();
    let mut stream = stream;
    stream.read_to_end(&mut text)?;
    let Some(address) = back.lock().unwrap_or_else(PoisonError::into_inner).clone() else {
        return Ok(());
    };
    let mut sender = UnixStream::connect(address)?;
    sender.set_write_timeout(Some(PATIENCE))?;
    sender.write_all(&text)?;
    let _ = sender.shutdown(Shutdown::Write);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    /// A stand-in session socket in a new directory, and a relay to it for `token`.
    fn relayed(name: &str) -> (TestDir, UnixListener, PathBuf, Relay) {
        let dir = TestDir::named(&format!("sk-relay-{name}-{}", std::process::id()));
        let socket = dir.join("1.sock");
        let session = UnixListener::bind(&socket).expect("bind the session's socket");
        let relay = Relay::start(socket.clone(), "token".to_owned()).expect("start the relay");
        (dir, session, socket, relay)
    }

    /// Sends `text` to the relay of `socket` and reads its answer.
    fn send(socket: &Path, text: &str) -> String {
        let mut stream = UnixStream::connect(path(socket)).expect("connect to the relay");
        stream
            .write_all(text.as_bytes())
            .expect("write to the relay");
        stream.shutdown(Shutdown::Write).expect("end the write");
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer
    }

    fn read(session: &UnixListener) -> Vec<Value> {
        let (stream, _) = session.accept().expect("take the relay's connection");
        let lines = BufReader::new(stream).lines().map_while(Result::ok);
        lines
            .map(|line| serde_json::from_str(&line).expect("a JSON line"))
            .collect()
    }

    #[test]
    fn lines_after_the_token_go_on_from_the_relay_and_receipts_come_back() {
        let (dir, session, socket, _relay) = relayed("token");
        let helper = UnixListener::bind(dir.join("sk-helper.sock")).expect("bind the helper's");
        let line = json!({ "type": "user", "msg_id": "m", "from": format!("uds:{}", dir.join("sk-helper.sock").display()) });
        let answer = send(
            &socket,
            &format!("{}\n{line}\n", json!({ "type": "auth", "token": "token" })),
        );
        assert_eq!(answer, TAKEN);

        let lines = read(&session);
        assert_eq!(lines[0], json!({ "type": "auth", "token": "token" }));
        let from = lines[1]["from"].as_str().expect("a reply address");
        let reply = from.strip_prefix("uds:").expect("a socket address");
        assert_eq!(Path::new(reply).parent(), Some(dir.as_ref()));
        assert!(reply.ends_with(".sock"));
        assert_eq!((lines.len(), &lines[1]["msg_id"]), (2, &json!("m")));

        let mut receipt = UnixStream::connect(reply).expect("connect to the relay's reply");
        receipt
            .write_all(b"{\"status\":\"held\"}\n")
            .expect("send a receipt");
        drop(receipt);
        let (stream, _) = helper.accept().expect("take the passed receipt");
        let passed: Vec<String> = BufReader::new(stream)
            .lines()
            .map_while(Result::ok)
            .collect();
        assert_eq!(passed, ["{\"status\":\"held\"}"]);
    }

    #[test]
    fn a_connection_without_the_token_gets_nothing_written() {
        let (_dir, session, socket, _relay) = relayed("refused");
        let line = json!({ "type": "user", "msg_id": "m" });
        let wrong = json!({ "type": "auth", "token": "other" });
        assert_eq!(send(&socket, &format!("{wrong}\n{line}\n")), "");
        assert_eq!(send(&socket, &format!("{line}\n")), "");

        session.set_nonblocking(true).expect("stop blocking");
        assert!(
            session.accept().is_err(),
            "the relay connected to the session"
        );
    }

    #[test]
    fn a_new_relay_removes_the_addresses_of_ended_ones_only() {
        let dir = TestDir::named(&format!("sk-relay-sweep-{}", std::process::id()));
        let ended = [dir.join("sk-2.sock.relay"), dir.join("sk-3-relay.sock")];
        for address in &ended {
            drop(UnixListener::bind(address).expect("bind an ended relay's address"));
        }
        let other = dir.join("4.sock");
        let _other = UnixListener::bind(&other).expect("bind another socket");
        let _session = UnixListener::bind(dir.join("1.sock")).expect("bind the session's socket");
        let running = Relay::start(dir.join("5.sock"), "token".to_owned()).expect("start a relay");

        let _relay = Relay::start(dir.join("1.sock"), "token".to_owned()).expect("start the relay");
        assert!(ended.iter().all(|address| !address.exists()));
        assert!(other.exists());
        assert!(running.paths.iter().all(|address| address.exists()));
    }

    #[test]
    fn the_relay_s_addresses_are_the_person_s_only_and_go_with_it() {
        let (dir, _session, socket, relay) = relayed("drop");
        let addresses = relay.paths.clone();
        for address in &addresses {
            let mode = fs::metadata(address)
                .expect("an address")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{}", address.display());
        }
        assert_eq!(addresses[0], path(&socket));
        drop(relay);
        assert!(addresses.iter().all(|address| !address.exists()));
        drop(dir);
    }
}
