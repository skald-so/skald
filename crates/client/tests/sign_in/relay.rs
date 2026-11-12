//! A recording relay between a machine and the server: it forwards every byte both ways and keeps
//! them, so a test sees what the machine sent and what the server answered, as on the wire.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

/// One read off a connection: when, which connection, which way, and the bytes.
struct Chunk {
    at: Instant,
    connection: usize,
    to_server: bool,
    bytes: Vec<u8>,
}

/// A request the machine sent: when its first byte came and its head, in lowercase.
#[derive(Debug, Clone)]
pub struct Sent {
    pub at: Instant,
    pub head: String,
}

/// The relay on a free local port; the machine talks to [`Relay::url`].
pub struct Relay {
    pub url: String,
    reads: Arc<Mutex<Vec<Chunk>>>,
}

impl Relay {
    /// Forwards every connection to `target`, such as `127.0.0.1:41234`.
    pub fn start(target: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        let reads = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&reads);
        let target = target.to_owned();
        thread::spawn(move || {
            for (connection, machine) in listener.incoming().enumerate() {
                let Ok(machine) = machine else { continue };
                let Ok(server) = TcpStream::connect(&target) else {
                    continue;
                };
                pipe(&machine, &server, connection, true, &kept);
                pipe(&server, &machine, connection, false, &kept);
            }
        });
        Relay { url, reads }
    }

    /// The time from which [`Relay::sent_since`] and [`Relay::answered_since`] count.
    pub fn mark(&self) -> Instant {
        Instant::now()
    }

    /// Every request the machine sent from `since` on, in the order they came.
    pub fn sent_since(&self, since: Instant) -> Vec<Sent> {
        let reads = self.reads.lock().expect("the relay's reads");
        let mut connections: Vec<usize> = reads.iter().map(|read| read.connection).collect();
        connections.sort_unstable();
        connections.dedup();
        let mut sent = Vec::new();
        for connection in connections {
            // The connection's bytes to the server, and when each of its reads came.
            let mut stream = Vec::new();
            let mut starts = Vec::new();
            for read in reads
                .iter()
                .filter(|read| read.connection == connection && read.to_server)
            {
                starts.push((stream.len(), read.at));
                stream.extend_from_slice(&read.bytes);
            }
            let text = String::from_utf8_lossy(&stream).into_owned();
            for at in request_starts(&text) {
                let came = starts
                    .iter()
                    .rev()
                    .find(|(offset, _)| *offset <= at)
                    .map(|(_, came)| *came)
                    .expect("a read holds the request");
                let end = text[at..]
                    .find("\r\n\r\n")
                    .map_or(text.len(), |end| at + end);
                if came >= since {
                    sent.push(Sent {
                        at: came,
                        head: text[at..end].to_lowercase(),
                    });
                }
            }
        }
        sent.sort_by_key(|request| request.at);
        sent
    }

    /// Every request the machine sent.
    pub fn sent(&self) -> Vec<Sent> {
        self.sent_since(self.started())
    }

    /// Everything the server answered from `since` on, every connection's bytes in the order they
    /// came.
    pub fn answered_since(&self, since: Instant) -> String {
        let reads = self.reads.lock().expect("the relay's reads");
        let bytes: Vec<u8> = reads
            .iter()
            .filter(|read| !read.to_server && read.at >= since)
            .flat_map(|read| read.bytes.iter().copied())
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Everything the server answered.
    pub fn answered(&self) -> String {
        self.answered_since(self.started())
    }

    fn started(&self) -> Instant {
        let reads = self.reads.lock().expect("the relay's reads");
        reads.first().map_or_else(Instant::now, |read| read.at)
    }
}

/// Copies `from` to `to` on a thread of its own, keeping each read; closes `to`'s writing half once
/// `from` ends.
fn pipe(
    from: &TcpStream,
    to: &TcpStream,
    connection: usize,
    to_server: bool,
    reads: &Arc<Mutex<Vec<Chunk>>>,
) {
    let mut from = from.try_clone().expect("clone a stream");
    let mut to = to.try_clone().expect("clone a stream");
    let reads = Arc::clone(reads);
    thread::spawn(move || {
        let mut buffer = [0; 16 * 1024];
        loop {
            let count = match from.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            reads.lock().expect("the relay's reads").push(Chunk {
                at: Instant::now(),
                connection,
                to_server,
                bytes: buffer[..count].to_vec(),
            });
            if to.write_all(&buffer[..count]).is_err() {
                break;
            }
        }
        let _ = to.shutdown(Shutdown::Write);
    });
}

/// Where each request line starts in `text`, a connection's bytes to the server.
fn request_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    for method in ["GET ", "POST ", "PUT ", "PATCH ", "DELETE ", "HEAD "] {
        for (at, _) in text.match_indices(method) {
            let line = text[at..].split("\r\n").next().unwrap_or_default();
            // A request on a kept-alive connection follows the last one's body on the same line.
            if line.ends_with(" HTTP/1.1") && line[method.len()..].starts_with('/') {
                starts.push(at);
            }
        }
    }
    starts.sort_unstable();
    starts
}

/// The values of every JSON string member `name` in `text`, such as the server's answers.
pub fn strings_named(text: &str, name: &str) -> Vec<String> {
    let key = format!("\"{name}\"");
    text.match_indices(&key)
        .filter_map(|(at, _)| {
            let rest = text[at + key.len()..].trim_start().strip_prefix(':')?;
            let rest = rest.trim_start().strip_prefix('"')?;
            Some(rest[..rest.find('"')?].to_owned())
        })
        .collect()
}
