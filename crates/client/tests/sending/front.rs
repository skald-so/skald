//! A front before the relay, listening at the server's public address: it forwards every
//! connection both ways and reads the WebSocket frames of each `GET /ws` connection, keeping each
//! `send` a machine sent and each `stored` and `send-refused` the server answered, as on the wire.
//! It can also, as a network or a compromised server would, lose the server's `stored` answers,
//! turn them into `send-refused`, swallow a machine's `send` before the server gets it, and take
//! the server down for delivery: close each connection that asks for a challenge or a WebSocket.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use skald_proto::{Cbor as _, Frame, RawEnvelope};

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
    let from_machine = {
        let (machine, server) = (clone(&machine), clone(&server));
        let (shared, upgrading) = (Arc::clone(shared), Arc::clone(&upgrading));
        thread::spawn(move || to_server(machine, server, &shared, &upgrading))
    };
    to_machine(clone(&server), clone(&machine), shared, &upgrading);
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
        while let Some((frame, length)) = ws_frame(pending) {
            let raw: Vec<u8> = pending.drain(..length).collect();
            if frame.is_binary()
                && let Ok(Frame::Send { envelope, report }) = Frame::from_cbor(&frame.payload)
            {
                let mut state = shared.state();
                state.sent.push(Sent { envelope, report });
                if state.swallow {
                    state.swallowed += 1;
                    continue;
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
            if !forward_answers(pending, &mut machine, shared) {
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
            if !forward_answers(&mut rest, &mut machine, shared) {
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
fn forward_answers(pending: &mut Vec<u8>, machine: &mut TcpStream, shared: &Shared) -> bool {
    while let Some((frame, length)) = ws_frame(pending) {
        let raw: Vec<u8> = pending.drain(..length).collect();
        let decoded = frame
            .is_binary()
            .then(|| Frame::from_cbor(&frame.payload).ok())
            .flatten();
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
