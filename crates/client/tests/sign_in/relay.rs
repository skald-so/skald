//! A recording relay between a machine and the server: it forwards every byte both ways and keeps
//! them, so a test sees what the machine sent and what the server answered, as on the wire. It can
//! also lose the server's answer to a request, or hold requests until several have come, as a
//! network or a race would, and rewrite the body of an answer, as a compromised server would.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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

/// How the relay loses the server's answer to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lose {
    /// Closes the machine's connection once the server has answered, forwarding none of it.
    Close,
    /// Keeps the machine's connection open and forwards nothing more on it.
    Withhold,
}

/// Gives the body the machine gets instead of the body of an answer the server sent.
pub type Rewrite = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// What the relay does to requests and answers beyond forwarding them.
#[derive(Default)]
struct Rules {
    /// The next request whose line starts so loses its answer, and how.
    lose: Option<(String, Lose)>,
    /// The connections whose next answer is lost, and how.
    losing: HashMap<usize, Lose>,
    /// The connections whose answers are withheld.
    withholding: HashSet<usize>,
    /// How many answers were lost.
    lost: usize,
    /// Requests whose line starts so wait until `count` of them have come, or until `until`.
    hold: Option<Hold>,
    /// The answers to requests whose line starts so get their bodies rewritten.
    rewrites: Vec<(String, Rewrite)>,
    /// The connections whose next answer gets its body rewritten, and what of it came so far.
    rewriting: HashMap<usize, (Rewrite, Vec<u8>)>,
}

struct Hold {
    request: String,
    count: usize,
    held: usize,
    until: Instant,
}

/// The rules, and the signal of a request that came to a hold.
#[derive(Default)]
struct Shared {
    rules: Mutex<Rules>,
    came: Condvar,
}

/// What becomes of an answer the server sent.
enum Fate {
    Forward,
    Close,
    Withhold,
    /// Kept until the whole answer has come, to be rewritten.
    Keep,
    /// These bytes go to the machine instead.
    Send(Vec<u8>),
}

impl Shared {
    /// Applies the rules to `chunk`, a read the machine sent on `connection`, before it goes on.
    fn sending(&self, connection: usize, chunk: &[u8]) {
        let text = String::from_utf8_lossy(chunk);
        let starts = |request: &str| {
            request_starts(&text)
                .into_iter()
                .any(|at| text[at..].starts_with(request))
        };
        let mut rules = self.rules.lock().expect("the relay's rules");
        if let Some((request, how)) = rules.lose.clone()
            && starts(&request)
        {
            rules.lose = None;
            rules.losing.insert(connection, how);
        }
        let rewrite = rules
            .rewrites
            .iter()
            .find(|(request, _)| starts(request))
            .map(|(_, rewrite)| Arc::clone(rewrite));
        if let Some(rewrite) = rewrite {
            rules.rewriting.insert(connection, (rewrite, Vec::new()));
        }
        let held = match &mut rules.hold {
            Some(hold) if starts(&hold.request) => {
                hold.held += 1;
                true
            }
            _ => false,
        };
        if held {
            self.came.notify_all();
            while let Some(hold) = &rules.hold {
                let now = Instant::now();
                if hold.held >= hold.count || now >= hold.until {
                    break;
                }
                let wait = hold.until - now;
                rules = self
                    .came
                    .wait_timeout(rules, wait)
                    .expect("the relay's rules")
                    .0;
            }
        }
    }

    /// What becomes of `chunk`, a read the server sent on `connection`.
    fn answering(&self, connection: usize, chunk: &[u8]) -> Fate {
        let mut rules = self.rules.lock().expect("the relay's rules");
        if rules.withholding.contains(&connection) {
            return Fate::Withhold;
        }
        if let Some(how) = rules.losing.remove(&connection) {
            rules.lost += 1;
            return match how {
                Lose::Close => Fate::Close,
                Lose::Withhold => {
                    rules.withholding.insert(connection);
                    Fate::Withhold
                }
            };
        }
        let Some((rewrite, answer)) = rules.rewriting.get_mut(&connection) else {
            return Fate::Forward;
        };
        answer.extend_from_slice(chunk);
        match rewritten(answer, rewrite.as_ref()) {
            None => Fate::Keep,
            Some(bytes) => {
                rules.rewriting.remove(&connection);
                Fate::Send(bytes)
            }
        }
    }
}

/// The relay on a free local port; the machine talks to [`Relay::url`].
pub struct Relay {
    pub url: String,
    reads: Arc<Mutex<Vec<Chunk>>>,
    shared: Arc<Shared>,
}

impl Relay {
    /// Forwards every connection to `target`, such as `127.0.0.1:41234`.
    pub fn start(target: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        let reads = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::new(Shared::default());
        let kept = Arc::clone(&reads);
        let rules = Arc::clone(&shared);
        let target = target.to_owned();
        thread::spawn(move || {
            for (connection, machine) in listener.incoming().enumerate() {
                let machine = match machine {
                    Ok(machine) => machine,
                    Err(err) => {
                        eprintln!("relay: connection {connection}: cannot accept: {err}");
                        continue;
                    }
                };
                let server = match TcpStream::connect(&target) {
                    Ok(server) => server,
                    Err(err) => {
                        eprintln!("relay: connection {connection}: cannot reach {target}: {err}");
                        continue;
                    }
                };
                pipe(&machine, &server, connection, true, &kept, &rules);
                pipe(&server, &machine, connection, false, &kept, &rules);
            }
        });
        Relay { url, reads, shared }
    }

    /// Loses the server's answer to the next request whose line starts with `request`, such as
    /// `POST /devices `, as `how` says. The server gets the request whole.
    pub fn lose_answer(&self, request: &str, how: Lose) {
        let mut rules = self.shared.rules.lock().expect("the relay's rules");
        rules.lose = Some((request.to_owned(), how));
    }

    /// How many answers of the server the relay has lost.
    pub fn lost(&self) -> usize {
        self.shared.rules.lock().expect("the relay's rules").lost
    }

    /// Holds each request whose line starts with `request` until `count` of them have come, or for
    /// `limit` at most, then lets them all go on at once.
    pub fn hold(&self, request: &str, count: usize, limit: Duration) {
        let mut rules = self.shared.rules.lock().expect("the relay's rules");
        rules.hold = Some(Hold {
            request: request.to_owned(),
            count,
            held: 0,
            until: Instant::now() + limit,
        });
    }

    /// From now on, gives the machine the body `rewrite` makes of the body of each answer with
    /// status 200 to a request whose line starts with `request`, such as `GET /devices`; replaces
    /// an earlier rewrite of the same request.
    pub fn rewrite(&self, request: &str, rewrite: impl Fn(&str) -> String + Send + Sync + 'static) {
        let mut rules = self.shared.rules.lock().expect("the relay's rules");
        rules.rewrites.retain(|(earlier, _)| earlier != request);
        rules.rewrites.push((request.to_owned(), Arc::new(rewrite)));
    }

    /// How many requests came to the hold.
    pub fn held(&self) -> usize {
        let rules = self.shared.rules.lock().expect("the relay's rules");
        rules.hold.as_ref().map_or(0, |hold| hold.held)
    }

    /// Every byte the machines sent, every connection's in the order they came.
    pub fn sent_bytes(&self) -> Vec<u8> {
        let reads = self.reads.lock().expect("the relay's reads");
        reads
            .iter()
            .filter(|read| read.to_server)
            .flat_map(|read| read.bytes.iter().copied())
            .collect()
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

/// Copies `from` to `to` on a thread of its own, keeping each read and applying `shared`'s rules;
/// closes `to`'s writing half once `from` ends.
fn pipe(
    from: &TcpStream,
    to: &TcpStream,
    connection: usize,
    to_server: bool,
    reads: &Arc<Mutex<Vec<Chunk>>>,
    shared: &Arc<Shared>,
) {
    let mut from = from.try_clone().expect("clone a stream");
    let mut to = to.try_clone().expect("clone a stream");
    let reads = Arc::clone(reads);
    let shared = Arc::clone(shared);
    thread::spawn(move || {
        let mut buffer = [0; 16 * 1024];
        loop {
            let count = match from.read(&mut buffer) {
                Ok(0) => break,
                Err(err) => {
                    let side = if to_server { "machine" } else { "server" };
                    eprintln!("relay: connection {connection}: cannot read the {side}: {err}");
                    break;
                }
                Ok(count) => count,
            };
            reads.lock().expect("the relay's reads").push(Chunk {
                at: Instant::now(),
                connection,
                to_server,
                bytes: buffer[..count].to_vec(),
            });
            if to_server {
                shared.sending(connection, &buffer[..count]);
            } else {
                match shared.answering(connection, &buffer[..count]) {
                    Fate::Forward => {}
                    Fate::Withhold | Fate::Keep => continue,
                    Fate::Close => {
                        let _ = to.shutdown(Shutdown::Both);
                        break;
                    }
                    Fate::Send(bytes) => {
                        if to.write_all(&bytes).is_err() {
                            break;
                        }
                        continue;
                    }
                }
            }
            if to.write_all(&buffer[..count]).is_err() {
                break;
            }
        }
        let _ = to.shutdown(Shutdown::Write);
    });
}

/// The answer `answer` with its body rewritten by `rewrite` if its status is 200, and whatever came
/// after it; `None` until the whole answer has come.
fn rewritten(answer: &[u8], rewrite: &(dyn Fn(&str) -> String + Send + Sync)) -> Option<Vec<u8>> {
    let end = answer.windows(4).position(|four| four == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&answer[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let headers: Vec<(String, &str)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim()))
        .collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(found, _)| found == name)
            .map(|(_, value)| *value)
    };
    let rest = &answer[end + 4..];
    let (body, after) = if header("transfer-encoding")
        .is_some_and(|value| value.contains("chunked"))
    {
        dechunked(rest)?
    } else {
        let length: usize = header("content-length").map_or(Some(0), |value| value.parse().ok())?;
        if rest.len() < length {
            return None;
        }
        (rest[..length].to_vec(), length)
    };
    let body = if status.split(' ').nth(1) == Some("200") {
        rewrite(&String::from_utf8_lossy(&body)).into_bytes()
    } else {
        body
    };
    let mut out = format!("{status}\r\n");
    for (name, value) in &headers {
        if name != "content-length" && name != "transfer-encoding" {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    out.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
    let mut out = out.into_bytes();
    out.extend_from_slice(&body);
    out.extend_from_slice(&rest[after..]);
    Some(out)
}

/// The body of `chunks`, a chunked body without trailers, and how many bytes it took; `None` until
/// its last chunk has come.
fn dechunked(chunks: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut body = Vec::new();
    let mut at = 0;
    loop {
        let line = chunks[at..].windows(2).position(|two| two == b"\r\n")?;
        let size = String::from_utf8_lossy(&chunks[at..at + line]).into_owned();
        let size = usize::from_str_radix(size.split(';').next().unwrap_or_default().trim(), 16)
            .expect("a chunk size");
        at += line + 2;
        if chunks.len() < at + size + 2 {
            return None;
        }
        body.extend_from_slice(&chunks[at..at + size]);
        at += size + 2;
        if size == 0 {
            return Some((body, at));
        }
    }
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
