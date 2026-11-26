//! A recording relay between a machine and the server: it forwards every byte both ways and keeps
//! them, so a test sees what the machine sent and what the server answered, as on the wire. It can
//! also lose the server's answer to a request, or hold requests until several have come, as a
//! network or a race would, rewrite the body of an answer, as a compromised server would, and answer
//! a request itself, never letting it reach the server, as an older server or a proxy that blocks it
//! would.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use socket2::SockRef;

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

/// How many of `sent` have a line that starts with `request`, whatever its case.
pub fn count_heads(sent: &[Sent], request: &str) -> usize {
    let request = request.to_lowercase();
    (sent.iter())
        .filter(|sent| sent.head.starts_with(&request))
        .count()
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
    /// Requests whose line starts so wait until `count` of them have come.
    hold: Option<Hold>,
    /// Set once the relay is dropped: nothing waits any more.
    released: bool,
    /// The answers to requests whose line starts so get their bodies rewritten.
    rewrites: Vec<(String, Rewrite)>,
    /// The connections whose next answer gets its body rewritten, and what of it came so far.
    rewriting: HashMap<usize, (Rewrite, Vec<u8>)>,
    /// Requests whose line starts so get this answer from the relay and never reach the server.
    refusals: Vec<(String, Vec<u8>)>,
    /// The connections the relay answered itself; it drops whatever else comes on them.
    refused: HashSet<usize>,
}

struct Hold {
    request: String,
    count: usize,
    held: usize,
    /// Whether it holds only the first such request, and lets the later ones go on.
    one: bool,
}

/// The rules, and the signal of a request that came to a hold.
#[derive(Default)]
struct Shared {
    rules: Mutex<Rules>,
    came: Condvar,
}

/// What becomes of a read the machine sent.
enum Pass {
    Forward,
    Drop,
    /// The relay answers with these bytes and closes the connection to the server.
    Answer(Vec<u8>),
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
    fn sending(&self, connection: usize, chunk: &[u8]) -> Pass {
        let text = String::from_utf8_lossy(chunk);
        let starts = |request: &str| {
            request_starts(&text)
                .into_iter()
                .any(|at| text[at..].starts_with(request))
        };
        let mut rules = self.rules.lock().expect("the relay's rules");
        if rules.refused.contains(&connection) {
            return Pass::Drop;
        }
        let answer = rules
            .refusals
            .iter()
            .find(|(request, _)| starts(request))
            .map(|(_, answer)| answer.clone());
        if let Some(answer) = answer {
            rules.refused.insert(connection);
            return Pass::Answer(answer);
        }
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
            Some(hold) if starts(&hold.request) && !(hold.one && hold.held > 0) => {
                hold.held += 1;
                true
            }
            _ => false,
        };
        if held {
            self.came.notify_all();
            while let Some(hold) = &rules.hold {
                if hold.held >= hold.count || rules.released {
                    break;
                }
                rules = self.came.wait(rules).expect("the relay's rules");
            }
        }
        Pass::Forward
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

/// The reads of every relay this process started, so that [`given_pairings`] reads all answers.
static ALL_READS: Mutex<Vec<Arc<Mutex<Vec<Chunk>>>>> = Mutex::new(Vec::new());

/// Every pairing a server gave a run without a session through a relay of this process, as the
/// `"pairing"` strings of its answers say.
pub fn given_pairings() -> Vec<String> {
    let all = ALL_READS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let mut pairings = Vec::new();
    for reads in all {
        let reads = reads.lock().unwrap_or_else(PoisonError::into_inner);
        let bytes: Vec<u8> = (reads.iter())
            .filter(|read| !read.to_server)
            .flat_map(|read| read.bytes.iter().copied())
            .collect();
        pairings.extend(strings_named(&String::from_utf8_lossy(&bytes), "pairing"));
    }
    pairings
}

/// The relay on a free local port; the machine talks to [`Relay::url`].
pub struct Relay {
    pub url: String,
    reads: Arc<Mutex<Vec<Chunk>>>,
    shared: Arc<Shared>,
}

impl Relay {
    /// The relay's address without the scheme, such as `127.0.0.1:41234`.
    pub fn address(&self) -> &str {
        self.url.trim_start_matches("http://")
    }

    /// Forwards every connection to `target`, such as `127.0.0.1:41234`.
    pub fn start(target: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        let reads = Arc::new(Mutex::new(Vec::new()));
        (ALL_READS.lock().unwrap_or_else(PoisonError::into_inner)).push(Arc::clone(&reads));
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

    /// Holds each request whose line starts with `request` until `count` of them have come, however
    /// long that takes, then lets them all go on at once. A machine whose request is held waits as
    /// long; until the relay is dropped, if fewer come.
    pub fn hold(&self, request: &str, count: usize) {
        let mut rules = self.shared.rules.lock().expect("the relay's rules");
        rules.hold = Some(Hold {
            request: request.to_owned(),
            count,
            held: 0,
            one: false,
        });
    }

    /// Holds the next request whose line starts with `request` until [`Relay::let_go`], however
    /// long that takes; later ones go on.
    pub fn hold_next(&self, request: &str) {
        let mut rules = self.shared.rules.lock().expect("the relay's rules");
        rules.hold = Some(Hold {
            request: request.to_owned(),
            count: usize::MAX,
            held: 0,
            one: true,
        });
    }

    /// Lets the requests the hold holds go on, and every later one.
    pub fn let_go(&self) {
        let mut rules = self.shared.rules.lock().expect("the relay's rules");
        if let Some(hold) = rules.hold.as_mut() {
            hold.count = 0;
        }
        self.shared.came.notify_all();
    }

    /// From now on, gives the machine the body `rewrite` makes of the body of each answer with
    /// status 200 to a request whose line starts with `request`, such as `GET /devices`; replaces
    /// an earlier rewrite of the same request.
    pub fn rewrite(&self, request: &str, rewrite: impl Fn(&str) -> String + Send + Sync + 'static) {
        let mut rules = self.shared.rules.lock().expect("the relay's rules");
        rules.rewrites.retain(|(earlier, _)| earlier != request);
        rules.rewrites.push((request.to_owned(), Arc::new(rewrite)));
    }

    /// As [`Relay::rewrite`], for the first such answer alone; later ones are given as the server
    /// gives them.
    pub fn rewrite_once(
        &self,
        request: &str,
        rewrite: impl Fn(&str) -> String + Send + Sync + 'static,
    ) {
        let first = AtomicBool::new(true);
        self.rewrite(request, move |answer| {
            if first.swap(false, Ordering::SeqCst) {
                rewrite(answer)
            } else {
                answer.to_owned()
            }
        });
    }

    /// From now on, answers each request whose line starts with `request` itself with `status`,
    /// such as `404 Not Found`, and a problem details object, closing the connection; the server
    /// never gets the request.
    pub fn refuse(&self, request: &str, status: &str) {
        let code = status.split(' ').next().unwrap_or_default();
        let body = format!(r#"{{"title":"{status}","status":{code}}}"#);
        self.refuse_with(request, status, &body);
    }

    /// As [`Relay::refuse`], with the problem details object `body`.
    pub fn refuse_with(&self, request: &str, status: &str, body: &str) {
        self.answer_itself(request, status, ("application/problem+json", body));
    }

    /// From now on, answers each request whose line starts with `request` itself with
    /// `403 Forbidden` and no problem details, as a proxy that blocks it would, closing the
    /// connection; the server never gets the request.
    pub fn block(&self, request: &str) {
        self.answer_itself(request, "403 Forbidden", ("text/plain", ""));
    }

    /// From now on, answers each request whose line starts with `request` itself with `status` and
    /// a body `body` of the media type `media`, closing the connection.
    fn answer_itself(&self, request: &str, status: &str, (media, body): (&str, &str)) {
        self.answer_with(request, status, (media, body.as_bytes()));
    }

    /// As [`Relay::refuse`], with any body: `body` of the media type `media`, such as a CBOR item
    /// a compromised server could give in place of the server's answer.
    pub fn answer_with(&self, request: &str, status: &str, (media, body): (&str, &[u8])) {
        let mut answer = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {media}\r\nconnection: close\r\n\
             content-length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        answer.extend_from_slice(body);
        let mut rules = self.shared.rules.lock().expect("the relay's rules");
        rules.refusals.push((request.to_owned(), answer));
    }

    /// From now on, lets every request reach the server again.
    pub fn stop_refusing(&self) {
        self.shared
            .rules
            .lock()
            .expect("the relay's rules")
            .refusals
            .clear();
    }

    /// Sends `request`, such as `GET /spaces/<id>/devices`, through the relay without a session,
    /// so that a request it holds until a second one comes goes on.
    pub fn send_through(&self, request: &str) {
        let address = self.address();
        let mut stream = TcpStream::connect(address).expect("reach the relay");
        write!(
            stream,
            "{request} HTTP/1.1\r\nhost: {address}\r\nconnection: close\r\n\r\n"
        )
        .expect("send a request to the relay");
        let mut answer = Vec::new();
        let _ = stream.read_to_end(&mut answer);
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

    /// The bytes sent to the server, with each connection's reads reassembled in order.
    pub fn sent_streams(&self) -> HashMap<usize, Vec<u8>> {
        let reads = self.reads.lock().expect("the relay's reads");
        let mut streams = HashMap::<_, Vec<_>>::new();
        for read in reads.iter().filter(|read| read.to_server) {
            streams
                .entry(read.connection)
                .or_default()
                .extend(&read.bytes);
        }
        streams
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

impl Drop for Relay {
    /// Lets held requests go on: their machines end with the test.
    fn drop(&mut self) {
        let mut rules = self
            .shared
            .rules
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        rules.released = true;
        self.shared.came.notify_all();
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
    let (source, sink) = if to_server {
        ("machine", "server")
    } else {
        ("server", "machine")
    };
    thread::spawn(move || {
        let mut buffer = [0; 16 * 1024];
        loop {
            let count = match from.read(&mut buffer) {
                Ok(0) => break,
                // A front before the relay resets a connection it is done with.
                Err(err) if err.kind() == io::ErrorKind::ConnectionReset => break,
                Err(err) => {
                    eprintln!("relay: connection {connection}: cannot read the {source}: {err}");
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
                match shared.sending(connection, &buffer[..count]) {
                    Pass::Forward => {}
                    Pass::Drop => continue,
                    Pass::Answer(answer) => {
                        let _ = from.write_all(&answer);
                        let _ = to.shutdown(Shutdown::Both);
                        continue;
                    }
                }
            } else {
                match shared.answering(connection, &buffer[..count]) {
                    Fate::Forward => {}
                    Fate::Withhold | Fate::Keep => continue,
                    Fate::Close => {
                        let _ = to.shutdown(Shutdown::Both);
                        break;
                    }
                    Fate::Send(bytes) => {
                        if let Err(err) = to.write_all(&bytes) {
                            eprintln!(
                                "relay: connection {connection}: cannot write to the {sink}: {err}"
                            );
                            break;
                        }
                        continue;
                    }
                }
            }
            if let Err(err) = to.write_all(&buffer[..count]) {
                eprintln!("relay: connection {connection}: cannot write to the {sink}: {err}");
                break;
            }
        }
        // The machine left a connection the server has answered in full: it ends with a reset.
        if to_server && idle(&reads.lock().expect("the relay's reads"), connection) {
            reset(&to);
            return;
        }
        let _ = to.shutdown(Shutdown::Write);
    });
}

/// Whether the server has answered every request the machine sent on `connection` whole, without
/// upgrading the connection: nothing the machine sent waits for the server there.
fn idle(reads: &[Chunk], connection: usize) -> bool {
    let bytes = |to_server: bool| -> Vec<u8> {
        reads
            .iter()
            .filter(|read| read.connection == connection && read.to_server == to_server)
            .flat_map(|read| read.bytes.iter().copied())
            .collect()
    };
    answered_in_full(&bytes(true), &bytes(false))
}

/// Whether `answered`, what a server sent on a connection, answers every request of `sent`, what
/// the machine sent on it, whole, without upgrading the connection.
pub fn answered_in_full(sent: &[u8], answered: &[u8]) -> bool {
    answers(answered) == Some(request_starts(&String::from_utf8_lossy(sent)).len())
}

/// How many answers `bytes`, what a server sent on a connection, holds, if it ends where an answer
/// ends; `None` if not, or if an answer upgrades the connection. An informational answer, such as
/// `100 Continue`, counts for none.
fn answers(mut bytes: &[u8]) -> Option<usize> {
    let mut count = 0;
    while !bytes.is_empty() {
        let end = bytes.windows(4).position(|four| four == b"\r\n\r\n")?;
        let head = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
        let rest = &bytes[end + 4..];
        let status: u16 = head.split(' ').nth(1)?.parse().ok()?;
        let header = |name: &str| {
            head.split("\r\n").skip(1).find_map(|line| {
                let (field, value) = line.split_once(':')?;
                (field.trim() == name).then(|| value.trim().to_owned())
            })
        };
        let length = if status == 101 {
            return None;
        } else if status < 200 || status == 204 || status == 304 {
            0
        } else if header("transfer-encoding").is_some_and(|value| value.contains("chunked")) {
            dechunked(rest)?.1
        } else {
            header("content-length")?.parse().ok()?
        };
        bytes = rest.get(length..)?;
        count += usize::from(status >= 200);
    }
    Some(count)
}

/// Ends `stream`'s connection with a reset once every copy of it is dropped: unlike a close, it
/// leaves neither end in TIME_WAIT, which holds the closing end's port for 30 s on macOS and for
/// minutes on Windows, where a whole run of the tests used up the ports. Ends reading too, so
/// that a thread that reads the stream lets go of its copy.
pub fn reset(stream: &TcpStream) {
    let _ = SockRef::from(stream).set_linger(Some(Duration::ZERO));
    let _ = stream.shutdown(Shutdown::Read);
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
