//! A front before the relay, listening at the server's public address: it forwards every
//! connection both ways and reads the WebSocket frames of each `GET /ws` connection, keeping each
//! `send` a machine sent and each `stored` and `send-refused` the server answered, as on the wire.
//! It can also, as a network or a compromised server would, lose the server's `stored` answers,
//! turn them into `send-refused`, swallow a machine's `send` before the server gets it, and take
//! the server down for delivery: close each connection that asks for a challenge or a WebSocket.
//! For delivery, it knows each connection's device by its `auth` and can swallow a machine's acks,
//! an `ack` or a `sync` after the connection's first, or cut the connection at the first; hold back
//! the server's `message` pushes or the empty `batch` that ends catch-up; and put envelopes into a
//! device's catch-up as `message` pushes before that `batch`, numbered after every `seq` the
//! connection stated or carried, or numbered as the cursor its first `sync` stated, as a
//! redelivery.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use skald_proto::{Cbor as _, DeviceId, Frame, Push, RawEnvelope};

/// The requests the front refuses while the server is down for delivery, as their lines start.
const DELIVERY: [&str; 2] = ["POST /session/challenge ", "GET /ws "];

/// The WebSocket opcode of a binary message.
const BINARY: u8 = 2;

/// What the front does with the server's `stored` answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answers {
    /// Forwards them.
    Forward,
    /// Loses them.
    Lose,
    /// Gives the machine a `send-refused` with this problem instead, for the same envelope.
    Refuse(String),
}

/// What the front does with a machine's acks: an `ack`, or a `sync` after the connection's first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acks {
    /// Forwards them.
    Forward,
    /// Swallows them, so the server never gets them.
    Swallow,
    /// Closes the connection, both ways, at the first, which the server never gets.
    Cut,
}

/// A `send` a machine sent.
#[derive(Debug, Clone)]
pub struct Sent {
    pub envelope: RawEnvelope,
    pub report: bool,
}

/// What came through so far, and the rules.
struct State {
    sent: Vec<Sent>,
    /// The server's `stored` and `send-refused` frames, as it sent them.
    answered: Vec<Frame>,
    answers: Answers,
    /// How many `stored` answers were lost.
    lost: usize,
    swallow: bool,
    /// How many `send` frames were swallowed.
    swallowed: usize,
    /// How many connections were closed for asking for delivery while the server was down.
    refused: usize,
    acks: Acks,
    /// How many acks the machines sent.
    acked: usize,
    /// Whether it holds back the server's `message` pushes and the `batch` that ends catch-up.
    hold_messages: bool,
    /// How many `message` pushes it held back.
    held_messages: usize,
    /// Whether it holds back the `batch` that ends catch-up.
    hold_end: bool,
    /// The envelopes to put into the next catch-up of each device, in order.
    injections: Vec<(DeviceId, Vec<RawEnvelope>)>,
    /// The envelopes to put into the next catch-up of each device that states a cursor above 0,
    /// numbered as that cursor.
    redeliveries: Vec<(DeviceId, RawEnvelope)>,
    /// How many envelopes it put into catch-ups.
    injected: usize,
}

/// What the front knows of one WebSocket connection.
#[derive(Default)]
struct Link {
    /// The device its `auth` names.
    device: Mutex<Option<DeviceId>>,
    /// Whether the machine has sent its first `sync`.
    synced: AtomicBool,
    /// The cursor its first `sync` stated.
    cursor: Mutex<Option<u64>>,
    /// The highest `seq` the machine stated or the server pushed on it.
    last_seq: Mutex<u64>,
}

impl Link {
    fn device(&self) -> Option<DeviceId> {
        *self.device.lock().expect("a connection's device")
    }

    fn saw(&self, seq: u64) {
        let mut last = self.last_seq.lock().expect("a connection's seq");
        *last = (*last).max(seq);
    }

    /// The `seq` after the highest it saw, which it then holds.
    fn next_seq(&self) -> NonZeroU64 {
        let mut last = self.last_seq.lock().expect("a connection's seq");
        *last += 1;
        NonZeroU64::new(*last).expect("above 0")
    }
}

struct Shared {
    /// Where connections go: the relay, set once the cloud has started.
    target: OnceLock<String>,
    state: Mutex<State>,
    down: AtomicBool,
}

impl Shared {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("the front's state")
    }
}

/// The front, serving until the tests end.
pub struct Front {
    port: u16,
    shared: Arc<Shared>,
}

impl Front {
    /// Listens on a free local port; connections wait until [`Front::forward_to`] names a target.
    pub fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().expect("the front's address").port();
        let shared = Arc::new(Shared {
            target: OnceLock::new(),
            state: Mutex::new(State {
                sent: Vec::new(),
                answered: Vec::new(),
                answers: Answers::Forward,
                lost: 0,
                swallow: false,
                swallowed: 0,
                refused: 0,
                acks: Acks::Forward,
                acked: 0,
                hold_messages: false,
                held_messages: 0,
                hold_end: false,
                injections: Vec::new(),
                redeliveries: Vec::new(),
                injected: 0,
            }),
            down: AtomicBool::new(false),
        });
        let accepting = Arc::clone(&shared);
        thread::spawn(move || {
            for (connection, machine) in listener.incoming().enumerate() {
                let Ok(machine) = machine else { continue };
                let shared = Arc::clone(&accepting);
                thread::spawn(move || serve(connection, machine, &shared));
            }
        });
        Front { port, shared }
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Forwards every connection to `target`, such as `127.0.0.1:41234`.
    pub fn forward_to(&self, target: &str) {
        self.shared
            .target
            .set(target.to_owned())
            .expect("one target");
    }

    /// Every `send` the machines sent so far, in the order they came, swallowed ones included.
    pub fn sent(&self) -> Vec<Sent> {
        self.shared.state().sent.clone()
    }

    /// The server's `stored` and `send-refused` frames so far, as it sent them.
    pub fn answered(&self) -> Vec<Frame> {
        self.shared.state().answered.clone()
    }

    /// From now on, does `answers` with the server's `stored` answers.
    pub fn answer(&self, answers: Answers) {
        self.shared.state().answers = answers;
    }

    /// How many `stored` answers it lost.
    pub fn lost(&self) -> usize {
        self.shared.state().lost
    }

    /// From now on, swallows each `send` if `swallow`, so the server never gets it.
    pub fn swallow(&self, swallow: bool) {
        self.shared.state().swallow = swallow;
    }

    /// How many `send` frames it swallowed.
    pub fn swallowed(&self) -> usize {
        self.shared.state().swallowed
    }

    /// From now on, closes each connection that asks for a challenge or a WebSocket if `down`.
    pub fn down(&self, down: bool) {
        self.shared.down.store(down, Ordering::SeqCst);
    }

    /// How many connections it closed for asking for delivery while down.
    pub fn refused(&self) -> usize {
        self.shared.state().refused
    }

    /// From now on, does `acks` with the machines' acks.
    pub fn acks(&self, acks: Acks) {
        self.shared.state().acks = acks;
    }

    /// How many acks the machines sent so far, forwarded or not.
    pub fn acked(&self) -> usize {
        self.shared.state().acked
    }

    /// From now on, holds back the server's `message` pushes and the `batch` that ends catch-up if
    /// `hold`.
    pub fn hold_messages(&self, hold: bool) {
        self.shared.state().hold_messages = hold;
    }

    /// How many `message` pushes it held back so far.
    pub fn held_messages(&self) -> usize {
        self.shared.state().held_messages
    }

    /// From now on, holds back the `batch` that ends catch-up if `hold`.
    pub fn hold_end(&self, hold: bool) {
        self.shared.state().hold_end = hold;
    }

    /// Puts `envelopes` into the next catch-up of `device`, in order, as `message` pushes before
    /// the `batch` that ends it.
    pub fn inject(&self, device: DeviceId, envelopes: Vec<RawEnvelope>) {
        self.shared.state().injections.push((device, envelopes));
    }

    /// Puts `envelope` into the next catch-up of `device` that states a cursor above 0, as a
    /// `message` push with that cursor as its `seq`, before the `batch` that ends it.
    pub fn redeliver(&self, device: DeviceId, envelope: RawEnvelope) {
        self.shared.state().redeliveries.push((device, envelope));
    }

    /// How many envelopes it put into catch-ups so far.
    pub fn injected(&self) -> usize {
        self.shared.state().injected
    }
}

/// Forwards `machine`'s connection, number `connection`, to the target, both ways.
fn serve(connection: usize, machine: TcpStream, shared: &Arc<Shared>) {
    let target = loop {
        if let Some(target) = shared.target.get() {
            break target.clone();
        }
        thread::sleep(Duration::from_millis(10));
    };
    let Ok(server) = TcpStream::connect(&target) else {
        eprintln!("front: connection {connection}: cannot reach {target}");
        return;
    };
    let upgrading = Arc::new(AtomicBool::new(false));
    let link = Arc::new(Link::default());
    let from_machine = {
        let (machine, server) = (clone(&machine), clone(&server));
        let (shared, upgrading) = (Arc::clone(shared), Arc::clone(&upgrading));
        let link = Arc::clone(&link);
        thread::spawn(move || to_server(machine, server, &shared, &upgrading, &link))
    };
    to_machine(clone(&server), clone(&machine), shared, &upgrading, &link);
    let _ = from_machine.join();
}

fn clone(stream: &TcpStream) -> TcpStream {
    stream.try_clone().expect("clone a stream")
}

/// Copies what the machine sends to the server: HTTP as it comes, then, once the request of a
/// WebSocket is through, its frames one by one.
fn to_server(
    mut machine: TcpStream,
    mut server: TcpStream,
    shared: &Shared,
    upgrading: &AtomicBool,
    link: &Link,
) {
    let mut frames = None::<Vec<u8>>;
    let mut buffer = [0; 16 * 1024];
    loop {
        let count = match machine.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        let chunk = &buffer[..count];
        let Some(pending) = frames.as_mut() else {
            let text = String::from_utf8_lossy(chunk);
            if shared.down.load(Ordering::SeqCst)
                && DELIVERY.iter().any(|request| text.contains(request))
            {
                shared.state().refused += 1;
                let _ = machine.shutdown(Shutdown::Both);
                let _ = server.shutdown(Shutdown::Both);
                return;
            }
            if text.starts_with(DELIVERY[1])
                && let Some(end) = text.find("\r\n\r\n")
            {
                upgrading.store(true, Ordering::SeqCst);
                let head = end + 4;
                if server.write_all(&chunk[..head]).is_err() {
                    break;
                }
                frames = Some(chunk[head..].to_vec());
                continue;
            }
            if server.write_all(chunk).is_err() {
                break;
            }
            continue;
        };
        pending.extend_from_slice(chunk);
        while let Some((decoded, raw)) = next_frame(pending) {
            let ack = match decoded {
                Some(Frame::Send { envelope, report }) => {
                    let mut state = shared.state();
                    state.sent.push(Sent { envelope, report });
                    if state.swallow {
                        state.swallowed += 1;
                        continue;
                    }
                    false
                }
                Some(Frame::Auth { device, .. }) => {
                    *link.device.lock().expect("a connection's device") = Some(device);
                    false
                }
                Some(Frame::Sync { cursor }) => {
                    link.saw(cursor);
                    let later = link.synced.swap(true, Ordering::SeqCst);
                    if !later {
                        *link.cursor.lock().expect("a connection's cursor") = Some(cursor);
                    }
                    later
                }
                Some(Frame::Ack { seq }) => {
                    link.saw(seq.get());
                    true
                }
                _ => false,
            };
            if ack {
                let acks = {
                    let mut state = shared.state();
                    state.acked += 1;
                    state.acks
                };
                match acks {
                    Acks::Forward => {}
                    Acks::Swallow => continue,
                    Acks::Cut => {
                        let _ = machine.shutdown(Shutdown::Both);
                        let _ = server.shutdown(Shutdown::Both);
                        return;
                    }
                }
            }
            if server.write_all(&raw).is_err() {
                return;
            }
        }
    }
    let _ = server.shutdown(Shutdown::Write);
}

/// Copies what the server sends to the machine: HTTP as it comes, then, once a WebSocket's
/// `101 Switching Protocols` is through, its frames one by one.
fn to_machine(
    mut server: TcpStream,
    mut machine: TcpStream,
    shared: &Shared,
    upgrading: &AtomicBool,
    link: &Link,
) {
    let mut head = Vec::new();
    let mut frames = None::<Vec<u8>>;
    let mut buffer = [0; 16 * 1024];
    loop {
        let count = match server.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        let chunk = &buffer[..count];
        if let Some(pending) = frames.as_mut() {
            pending.extend_from_slice(chunk);
            if !forward_answers(pending, &mut machine, shared, link) {
                break;
            }
            continue;
        }
        if !upgrading.load(Ordering::SeqCst) {
            if machine.write_all(chunk).is_err() {
                break;
            }
            continue;
        }
        head.extend_from_slice(chunk);
        let Some(end) = head.windows(4).position(|four| four == b"\r\n\r\n") else {
            continue;
        };
        let mut rest = head.split_off(end + 4);
        let answer = std::mem::take(&mut head);
        if machine.write_all(&answer).is_err() {
            break;
        }
        if answer.starts_with(b"HTTP/1.1 101") {
            if !forward_answers(&mut rest, &mut machine, shared, link) {
                break;
            }
            frames = Some(rest);
        } else {
            upgrading.store(false, Ordering::SeqCst);
            if machine.write_all(&rest).is_err() {
                break;
            }
        }
    }
    let _ = machine.shutdown(Shutdown::Write);
}

/// Forwards to `machine` each whole frame in `pending`, the server's, as the rules say; whether
/// the machine is still there.
fn forward_answers(
    pending: &mut Vec<u8>,
    machine: &mut TcpStream,
    shared: &Shared,
    link: &Link,
) -> bool {
    while let Some((decoded, raw)) = next_frame(pending) {
        let out = match decoded {
            Some(Frame::Stored(stored)) => {
                let mut state = shared.state();
                state.answered.push(Frame::Stored(stored.clone()));
                match state.answers.clone() {
                    Answers::Forward => Some(raw),
                    Answers::Lose => {
                        state.lost += 1;
                        None
                    }
                    Answers::Refuse(problem) => Some(server_frame(
                        &Frame::SendRefused {
                            send_key: stored.send_key,
                            problem,
                        }
                        .to_cbor(),
                    )),
                }
            }
            Some(refused @ Frame::SendRefused { .. }) => {
                shared.state().answered.push(refused);
                Some(raw)
            }
            Some(Frame::Push(Push::Message { seq, .. })) => {
                link.saw(seq.get());
                let mut state = shared.state();
                if state.hold_messages {
                    state.held_messages += 1;
                    None
                } else {
                    Some(raw)
                }
            }
            Some(Frame::Push(Push::Status { seq, .. })) => {
                link.saw(seq.get());
                Some(raw)
            }
            Some(Frame::Batch(entries)) if entries.is_empty() => end_of_catch_up(raw, shared, link),
            _ => Some(raw),
        };
        if let Some(out) = out
            && machine.write_all(&out).is_err()
        {
            return false;
        }
    }
    true
}

/// The `batch` `raw` that ends a catch-up, as the rules say: held back, or after the envelopes to
/// put into the catch-up of the connection's device, as `message` pushes numbered after the
/// highest `seq` it saw.
fn end_of_catch_up(raw: Vec<u8>, shared: &Shared, link: &Link) -> Option<Vec<u8>> {
    let mut state = shared.state();
    if state.hold_messages || state.hold_end {
        return None;
    }
    let device = link.device();
    let mut out = Vec::new();
    let cursor = link
        .cursor
        .lock()
        .expect("a connection's cursor")
        .and_then(NonZeroU64::new);
    if let Some(cursor) = cursor
        && let Some(at) = state
            .redeliveries
            .iter()
            .position(|(to, _)| Some(*to) == device)
    {
        let (_, envelope) = state.redeliveries.remove(at);
        state.injected += 1;
        let push = Push::Message {
            seq: cursor,
            envelope,
        };
        out.extend(server_frame(&Frame::Push(push).to_cbor()));
    }
    let envelopes = match state
        .injections
        .iter()
        .position(|(to, _)| Some(*to) == device)
    {
        Some(at) => state.injections.remove(at).1,
        None => Vec::new(),
    };
    state.injected += envelopes.len();
    for envelope in envelopes {
        let push = Push::Message {
            seq: link.next_seq(),
            envelope,
        };
        out.extend(server_frame(&Frame::Push(push).to_cbor()));
    }
    out.extend(raw);
    Some(out)
}

/// Takes the whole WebSocket frame at the start of `pending`, once it has come: the delivery frame
/// it carries, if it is a binary one that decodes, and its bytes on the wire.
fn next_frame(pending: &mut Vec<u8>) -> Option<(Option<Frame>, Vec<u8>)> {
    let (frame, length) = ws_frame(pending)?;
    let raw: Vec<u8> = pending.drain(..length).collect();
    let decoded = frame
        .is_binary()
        .then(|| Frame::from_cbor(&frame.payload).ok())
        .flatten();
    Some((decoded, raw))
}

/// A WebSocket frame: its opcode and its payload, unmasked.
struct WsFrame {
    opcode: u8,
    payload: Vec<u8>,
}

impl WsFrame {
    fn is_binary(&self) -> bool {
        self.opcode == BINARY
    }
}

/// The frame at the start of `bytes` and its length on the wire, once it has come whole.
fn ws_frame(bytes: &[u8]) -> Option<(WsFrame, usize)> {
    let (&first, &second) = (bytes.first()?, bytes.get(1)?);
    let masked = second & 0x80 != 0;
    let (length, mut at) = match second & 0x7f {
        126 => (
            u64::from(u16::from_be_bytes(bytes.get(2..4)?.try_into().ok()?)),
            4,
        ),
        127 => (u64::from_be_bytes(bytes.get(2..10)?.try_into().ok()?), 10),
        short => (u64::from(short), 2),
    };
    let mask: Option<[u8; 4]> = if masked {
        let mask = bytes.get(at..at + 4)?.try_into().ok()?;
        at += 4;
        Some(mask)
    } else {
        None
    };
    let end = at + usize::try_from(length).ok()?;
    let mut payload = bytes.get(at..end)?.to_vec();
    if let Some(mask) = mask {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    let frame = WsFrame {
        opcode: first & 0x0f,
        payload,
    };
    Some((frame, end))
}

/// `payload` as one unmasked binary frame, as a server sends it.
fn server_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x80 | BINARY];
    match payload.len() {
        short @ 0..=125 => frame.push(u8::try_from(short).expect("below 126")),
        medium @ 126..=0xffff => {
            frame.push(126);
            frame.extend_from_slice(&u16::try_from(medium).expect("16 bits").to_be_bytes());
        }
        long => {
            frame.push(127);
            frame.extend_from_slice(&u64::try_from(long).expect("64 bits").to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    frame
}
