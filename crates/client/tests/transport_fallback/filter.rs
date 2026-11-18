//! What a TLS-intercepting proxy does with the requests it reads inside TLS: a plain HTTP filter
//! before the relay, on a free local port, which the TLS endpoint forwards to. It takes one request
//! per connection and forwards it to the relay with `connection: close`, the server's answer
//! closing the connection, or tunnels a WebSocket upgrade both ways. It keeps each request's line
//! and what became of it.
//!
//! As a proxy that blocks WebSocket or streaming would, it can refuse WebSocket upgrades and the
//! mailbox's event stream with `403 Forbidden`, or hold the stream's answer back until it ends. As
//! a network would, it can take the server down for delivery, closing each connection that asks
//! for a challenge, a WebSocket or the mailbox, unanswered; hold the first envelope a device sends
//! over HTTPS, or the next long-poll, for a while before it goes on, or drop the envelope
//! unanswered; and cut the machine off once the server has sent it a
//! given envelope: what the server sends on the connections open then still reaches the machine,
//! but nothing the machine sends after it reaches the server, such as its ack. It can close the
//! WebSockets it tunnels, as a dropped connection, or hold what the machine sends next on one back
//! for a while.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::proxy::read_head;

/// The paths of the WebSocket, the mailbox's event stream, its long-poll and the HTTPS send, as the
/// OpenAPI spec names them.
pub const WEBSOCKET: &str = "/ws";
pub const EVENTS: &str = "/mailbox/events";
pub const POLL: &str = "/mailbox/poll";
pub const SEND: &str = "/mailbox/send";
/// The paths of the requests that need delivery: a challenge, the WebSocket and the mailbox's.
const DELIVERY: [&str; 2] = ["/session/challenge", "/mailbox/"];

/// The answer of a proxy that blocks a request.
const FORBIDDEN: &[u8] = b"HTTP/1.1 403 Forbidden\r\ncontent-type: text/plain\r\n\
    content-length: 23\r\nconnection: close\r\n\r\nblocked by the proxy.\r\n";

/// What the filter does with the mailbox's event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Forward,
    /// Answers its request with `403 Forbidden`.
    Refuse,
    /// Forwards its request and holds the answer back until the server ends it.
    Hold,
}

/// What became of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    /// The server answered with this status.
    Answered(u16),
    /// The filter answered `403 Forbidden`.
    Refused,
    /// The filter held the answer back; it never came before the machine left.
    Held,
    /// The filter closed the connection unanswered: the server was down for delivery, or the
    /// connections were cut.
    Closed,
}

/// A request the filter took.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    /// Its path, without the query.
    pub path: String,
    /// Its query, without the `?`.
    pub query: String,
    /// When its head came.
    pub at: Instant,
    /// When the filter started to give the machine its answer.
    pub answering: Option<Instant>,
    pub fate: Fate,
}

impl Seen {
    /// Whether it is `method` `path`.
    pub fn is(&self, method: &str, path: &str) -> bool {
        self.method == method && self.path == path
    }

    /// The value of its query parameter `name`, if it has one.
    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then_some(value)
        })
    }
}

/// The rules, and what the filter saw.
struct State {
    refuse_websocket: bool,
    stream: Stream,
    down: bool,
    /// How long the next `POST /mailbox/send` waits before it goes on.
    delay_send: Option<Duration>,
    /// Whether the next `POST /mailbox/send` is closed unanswered.
    drop_send: bool,
    /// How long the next `POST /mailbox/poll` waits before it goes on.
    delay_poll: Option<Duration>,
    /// How many long-polls waited so.
    delayed_polls: usize,
    /// The envelope whose bytes, once the server has sent them, cut the machine off.
    cut_after: Option<Vec<u8>>,
    /// Whether the machine is cut off.
    cut: bool,
    seen: Vec<Seen>,
    /// What the server sent on the event streams and the filter passed to the machine.
    streamed: Vec<u8>,
    /// The machine's end of each WebSocket it tunnels.
    tunnels: Vec<TcpStream>,
    /// How long what the machine sends next on a WebSocket waits before it goes on.
    delay_tunnel: Option<Duration>,
    /// How many of its reads waited so.
    delayed_tunnel: usize,
}

impl State {
    /// Whether `sent`, what the server sent on a connection so far, carries the envelope the cut
    /// waits for, as it is or in an event's base64url `data`.
    fn carries(&self, sent: &[u8]) -> bool {
        let Some(needle) = self.cut_after.as_deref() else {
            return false;
        };
        let holds = |hay: &[u8]| hay.windows(needle.len()).any(|window| window == needle);
        holds(sent) || data_of(sent).any(|bytes| holds(&bytes))
    }
}

/// The bytes of each event's `data` in `sent`, from base64url.
fn data_of(sent: &[u8]) -> impl Iterator<Item = Vec<u8>> + '_ {
    sent.split(|byte| *byte == b'\n').filter_map(|line| {
        let data = line.strip_prefix(b"data:")?;
        let data = data.strip_prefix(b" ").unwrap_or(data);
        let data = data.strip_suffix(b"\r").unwrap_or(data);
        URL_SAFE_NO_PAD.decode(data).ok()
    })
}

/// The filter, serving until the tests end.
#[derive(Clone)]
pub struct Filter {
    port: u16,
    state: Arc<Mutex<State>>,
}

impl Filter {
    /// Forwards to `target`, such as the relay's `127.0.0.1:41234`; at first it forwards every
    /// request.
    pub fn start(target: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().expect("the filter's address").port();
        let filter = Filter {
            port,
            state: Arc::new(Mutex::new(State {
                refuse_websocket: false,
                stream: Stream::Forward,
                down: false,
                delay_send: None,
                drop_send: false,
                delay_poll: None,
                delayed_polls: 0,
                cut_after: None,
                cut: false,
                seen: Vec::new(),
                streamed: Vec::new(),
                tunnels: Vec::new(),
                delay_tunnel: None,
                delayed_tunnel: 0,
            })),
        };
        let (serving, target) = (filter.clone(), target.to_owned());
        thread::spawn(move || {
            for machine in listener.incoming().flatten() {
                let (filter, target) = (serving.clone(), target.clone());
                thread::spawn(move || filter.serve(machine, &target));
            }
        });
        filter
    }

    /// Its address, `127.0.0.1:<port>`, for the TLS endpoint to forward to.
    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("the filter's state")
    }

    /// From now on, refuses WebSocket upgrades if `refuse`, and does with the event stream what
    /// `stream` says.
    pub fn block(&self, refuse: bool, stream: Stream) {
        let mut state = self.state();
        state.refuse_websocket = refuse;
        state.stream = stream;
    }

    /// Takes the server down for delivery, or back up.
    pub fn down(&self, down: bool) {
        self.state().down = down;
    }

    /// Holds the next `POST /mailbox/send` for `delay` before it goes on.
    pub fn delay_next_send(&self, delay: Duration) {
        self.state().delay_send = Some(delay);
    }

    /// Holds the next `POST /mailbox/poll` for `delay` before it goes on.
    pub fn delay_next_poll(&self, delay: Duration) {
        self.state().delay_poll = Some(delay);
    }

    /// How many long-polls it held so far.
    pub fn delayed_polls(&self) -> usize {
        self.state().delayed_polls
    }

    /// Closes the next `POST /mailbox/send` unanswered, as a dropped connection.
    pub fn drop_next_send(&self) {
        self.state().drop_send = true;
    }

    /// Cuts the machine off once the server has sent it `envelope`: from then on, nothing the
    /// machine sends reaches the server, its new connections closed unanswered, while what the
    /// server sends on the connections open then still reaches it.
    pub fn cut_after(&self, envelope: &[u8]) {
        let mut state = self.state();
        state.cut_after = Some(envelope.to_vec());
        state.cut = false;
    }

    /// Holds what the machine sends next on a WebSocket for `delay` before it goes on, and all
    /// that it sends after on it meanwhile.
    pub fn delay_next_tunnel_write(&self, delay: Duration) {
        self.state().delay_tunnel = Some(delay);
    }

    /// How many of the machine's writes on a WebSocket it held so far.
    pub fn delayed_tunnel_writes(&self) -> usize {
        self.state().delayed_tunnel
    }

    /// Closes each WebSocket it tunnels.
    pub fn close_tunnels(&self) {
        for tunnel in std::mem::take(&mut self.state().tunnels) {
            let _ = tunnel.shutdown(Shutdown::Both);
        }
    }

    /// Whether the machine was cut off.
    pub fn was_cut(&self) -> bool {
        self.state().cut
    }

    /// Stops cutting the machine off.
    pub fn stop_cutting(&self) {
        let mut state = self.state();
        state.cut_after = None;
        state.cut = false;
    }

    /// The frames of the events the filter passed to the machine on its event streams.
    pub fn streamed(&self) -> Vec<Vec<u8>> {
        data_of(&self.state().streamed).collect()
    }

    /// How many requests it took so far: a mark for [`Filter::seen_since`].
    pub fn mark(&self) -> usize {
        self.state().seen.len()
    }

    /// The requests it took from the mark `from` on, in the order their heads came.
    pub fn seen_since(&self, from: usize) -> Vec<Seen> {
        self.state().seen[from..].to_vec()
    }

    /// Keeps `seen`; gives its index.
    fn keep(&self, seen: Seen) -> usize {
        let mut state = self.state();
        state.seen.push(seen);
        state.seen.len() - 1
    }

    fn settle(&self, index: usize, fate: Fate, answering: Option<Instant>) {
        let mut state = self.state();
        state.seen[index].fate = fate;
        state.seen[index].answering = state.seen[index].answering.or(answering);
    }

    /// Takes one request of `machine` and forwards it to `target` as the rules say.
    fn serve(&self, mut machine: TcpStream, target: &str) {
        let Some((head, body)) = read_request(&mut machine) else {
            return;
        };
        let mut words = head.split_whitespace();
        let (method, target_path) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
        let (path, query) = target_path.split_once('?').unwrap_or((target_path, ""));
        let (path, query) = (path.to_owned(), query.to_owned());
        let upgrade = header(&head, "upgrade").is_some_and(|value| value.contains("websocket"));
        let index = self.keep(Seen {
            method: method.to_owned(),
            path: path.clone(),
            query,
            at: Instant::now(),
            answering: None,
            fate: Fate::Closed,
        });
        let (refused, held, closed, delay) = {
            let mut state = self.state();
            let delivery = path == WEBSOCKET || DELIVERY.iter().any(|at| path.starts_with(at));
            let events = method == "GET" && path == EVENTS;
            let sending = method == "POST" && path == SEND;
            let polling = method == "POST" && path == POLL;
            let delay = match (sending, polling) {
                (true, _) => state.delay_send.take(),
                (_, true) => state.delay_poll.take(),
                _ => None,
            };
            state.delayed_polls += usize::from(polling && delay.is_some());
            let dropped = sending && std::mem::take(&mut state.drop_send);
            (
                (upgrade && state.refuse_websocket) || (events && state.stream == Stream::Refuse),
                events && state.stream == Stream::Hold,
                state.cut || (state.down && delivery) || dropped,
                delay,
            )
        };
        if closed {
            return close(machine);
        }
        if refused {
            self.settle(index, Fate::Refused, Some(Instant::now()));
            let _ = machine.write_all(FORBIDDEN);
            return close(machine);
        }
        if let Some(delay) = delay {
            thread::sleep(delay);
        }
        let Ok(mut server) = TcpStream::connect(target) else {
            return close(machine);
        };
        let head = if upgrade { head } else { closing(&head) };
        if server.write_all(head.as_bytes()).is_err() || server.write_all(&body).is_err() {
            return close(machine);
        }
        if upgrade {
            self.tunnel(machine, server, index);
        } else if held {
            self.hold(machine, server, index);
        } else {
            self.answer(machine, server, index);
        }
    }

    /// Gives `machine` the answer `server` sends as it comes, until the server closes.
    fn answer(&self, machine: TcpStream, server: TcpStream, index: usize) {
        watch_leaving(&machine, &server);
        self.pass_answer(machine, server, index);
    }

    /// Holds back what `server` answers until it closes, then gives it to `machine`, if still
    /// there; the server's stream ends once the machine leaves.
    fn hold(&self, mut machine: TcpStream, mut server: TcpStream, index: usize) {
        self.settle(index, Fate::Held, None);
        watch_leaving(&machine, &server);
        let mut answer = Vec::new();
        let _ = server.read_to_end(&mut answer);
        if !self.cut_now() {
            let _ = machine.write_all(&answer);
        }
        close(machine);
    }

    /// Forwards a WebSocket upgrade both ways until either side closes; once the machine is cut
    /// off, nothing more of it.
    fn tunnel(&self, machine: TcpStream, server: TcpStream, index: usize) {
        let (Ok(mut from_machine), Ok(mut to_server), Ok(kept)) =
            (machine.try_clone(), server.try_clone(), machine.try_clone())
        else {
            return close(machine);
        };
        self.state().tunnels.push(kept);
        let filter = self.clone();
        thread::spawn(move || {
            let mut buffer = [0; 16 * 1024];
            while let Ok(count @ 1..) = from_machine.read(&mut buffer) {
                if let Some(delay) = filter.tunnel_delay() {
                    thread::sleep(delay);
                }
                if filter.cut_now() || to_server.write_all(&buffer[..count]).is_err() {
                    break;
                }
            }
            let _ = to_server.shutdown(Shutdown::Both);
        });
        self.pass_answer(machine, server, index);
    }

    /// Gives `machine` what `server` sends as it comes, noting it, until the server closes or the
    /// machine leaves; then ends both.
    fn pass_answer(&self, mut machine: TcpStream, mut server: TcpStream, index: usize) {
        let mut sent = Vec::new();
        let mut buffer = [0; 16 * 1024];
        while let Ok(count @ 1..) = server.read(&mut buffer) {
            sent.extend_from_slice(&buffer[..count]);
            self.note(&sent, index);
            if machine.write_all(&buffer[..count]).is_err() {
                break;
            }
            self.passed(index, &buffer[..count]);
        }
        let _ = server.shutdown(Shutdown::Both);
        close(machine);
    }

    /// Notes the status of the answer `sent` so far begins with, before the machine gets any of
    /// it; cuts the machine off if it carries the envelope the cut waits for.
    fn note(&self, sent: &[u8], index: usize) {
        let mut state = self.state();
        if state.carries(sent) {
            state.cut = true;
        }
        let status = status_of(sent);
        let seen = &mut state.seen[index];
        if let Some(status) = status {
            seen.fate = Fate::Answered(status);
        }
        seen.answering.get_or_insert_with(Instant::now);
    }

    /// Keeps `bytes`, which the filter passed to the machine as the answer to the request at
    /// `index`, if that is an event stream.
    fn passed(&self, index: usize, bytes: &[u8]) {
        let mut state = self.state();
        if state.seen[index].is("GET", EVENTS) {
            state.streamed.extend_from_slice(bytes);
        }
    }

    fn cut_now(&self) -> bool {
        self.state().cut
    }

    /// How long a write on a WebSocket waits, if it is to, counting it.
    fn tunnel_delay(&self) -> Option<Duration> {
        let mut state = self.state();
        let delay = state.delay_tunnel.take();
        state.delayed_tunnel += usize::from(delay.is_some());
        delay
    }
}

/// Shuts `server` down once `machine` leaves: closes its connection or sends anything more, which
/// the filter does not take.
fn watch_leaving(machine: &TcpStream, server: &TcpStream) {
    let (Ok(mut machine), Ok(server)) = (machine.try_clone(), server.try_clone()) else {
        return;
    };
    thread::spawn(move || {
        let mut byte = [0];
        let _ = machine.read(&mut byte);
        let _ = server.shutdown(Shutdown::Both);
    });
}

/// Ends the machine's connection: no more from the filter, and whatever else the machine sends is
/// read and dropped until it closes, so that what it was given reaches it whole.
fn close(mut machine: TcpStream) {
    let _ = machine.shutdown(Shutdown::Write);
    let _ = machine.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = io::copy(&mut machine, &mut io::sink());
}

/// `head` with `connection: close` in place of any `connection` header it has.
fn closing(head: &str) -> String {
    let mut lines: Vec<&str> = head.trim_end_matches("\r\n").split("\r\n").collect();
    lines.retain(|line| {
        !line
            .split(':')
            .next()
            .is_some_and(|name| name.trim().eq_ignore_ascii_case("connection"))
    });
    lines.push("connection: close");
    format!("{}\r\n\r\n", lines.join("\r\n"))
}

/// The value of the header `name` of `head`, in lowercase.
fn header(head: &str, name: &str) -> Option<String> {
    head.split("\r\n").skip(1).find_map(|line| {
        let (field, value) = line.split_once(':')?;
        field
            .trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_lowercase())
    })
}

/// The status of the answer `sent` begins with, once its status line has come.
fn status_of(sent: &[u8]) -> Option<u16> {
    let line = sent.split(|byte| *byte == b'\r').next()?;
    let line = std::str::from_utf8(line).ok()?;
    line.strip_prefix("HTTP/1.1 ")?.get(..3)?.parse().ok()
}

/// The head of the request on `machine`, with its empty line, and its body, by its
/// `content-length` or chunked as it came; `None` if it never comes whole.
fn read_request(machine: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let head = read_head(machine)?;
    let chunked = header(&head, "transfer-encoding").is_some_and(|value| value.contains("chunked"));
    let body = if chunked {
        read_chunks(machine)?
    } else {
        let length = header(&head, "content-length").map_or(Some(0), |value| value.parse().ok())?;
        let mut body = vec![0; length];
        machine.read_exact(&mut body).ok()?;
        body
    };
    Some((head, body))
}

/// A chunked body on `machine` as it came, up to its last chunk and the empty line after it.
fn read_chunks(machine: &mut TcpStream) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let start = body.len();
        let mut byte = [0];
        while !body[start..].ends_with(b"\r\n") {
            if machine.read(&mut byte).ok()? == 0 {
                return None;
            }
            body.extend_from_slice(&byte);
        }
        let line = String::from_utf8_lossy(&body[start..body.len() - 2]).into_owned();
        let size = usize::from_str_radix(line.split(';').next()?.trim(), 16).ok()?;
        let mut chunk = vec![0; size + 2];
        machine.read_exact(&mut chunk).ok()?;
        body.extend_from_slice(&chunk);
        if size == 0 {
            return Some(body);
        }
    }
}
