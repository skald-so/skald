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
//! redelivery. It keeps each `space-notice` the server sent a device, and can hold back the `batch`
//! that ends a catch-up until the server sends that connection a `space-notice`, which then comes
//! first. It can hold back a device's `auth`, and what the machine sends after it, until released.
//! It keeps each `history` frame a machine sent, and can close the connection, both ways, at the
//! next one that carries a link, or a wrap, of a channel, which the server never gets. It can lose the server's
//! `history-stored` answers or turn them into `history-refused`. It can lose only the `stored`
//! answers to `mls` envelopes of a channel from an MLS epoch on.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use skald_crypto::MlsFraming;
use skald_proto::{
    Cbor as _, ChannelId, DeviceId, Frame, HistoryItem, MessageId, Push, RawEnvelope, SpaceId,
};

use super::common::reach;
use super::relay::{answered_in_full, reset};

// Other helpers of the same test binary load it too.
#[path = "../common/local_listener.rs"]
#[allow(dead_code, clippy::duplicate_mod)]
mod local_listener;
use local_listener::local_listener;

/// The requests the front refuses while the server is down for delivery, as their lines start.
const DELIVERY: [&str; 2] = ["POST /session/challenge ", "GET /ws "];

/// An ack in an HTTPS request, as its line starts: a run off a WebSocket acks so.
const HTTPS_ACK: &str = "POST /mailbox/ack ";

/// The WebSocket opcode of a binary message.
const BINARY: u8 = 2;

/// What the front does with the server's `stored` answers, or with its `history-stored` ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answers {
    /// Forwards them.
    Forward,
    /// Loses them.
    Lose,
    /// Gives the machine a `send-refused` with this problem instead, for the same envelope, or a
    /// `history-refused`, for the same upload.
    Refuse(String),
}

/// What the front does with a machine's acks: an `ack`, or a `sync` after the connection's first,
/// on a WebSocket, or an ack in an HTTPS request, whose connection it closes both ways unless it
/// forwards it, as it cannot answer it.
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

/// A `history` frame a machine sent.
#[derive(Debug, Clone)]
pub struct Upload {
    pub space: SpaceId,
    pub channel: ChannelId,
    pub item: HistoryItem,
    /// Whether the front forwarded it to the server.
    pub forwarded: bool,
}

/// A queue key object a machine uploaded in a `history` frame, the item `[4, queue-key]`, read from
/// its bytes alone.
#[derive(Debug, Clone)]
pub struct QueueKeyUpload {
    pub space: SpaceId,
    pub channel: ChannelId,
    /// The object's CBOR bytes.
    pub object: Vec<u8>,
    /// Whether the front forwarded it to the server.
    pub forwarded: bool,
}

#[derive(Default)]
struct Delivery {
    sequences: Vec<NonZeroU64>,
    ack: Option<NonZeroU64>,
}

/// What came through so far, and the rules.
struct State {
    sent: Vec<Sent>,
    /// Every `history` frame the machines sent, in the order they came.
    uploads: Vec<Upload>,
    /// The channel at whose next link in a `history` frame it closes the connection.
    cut_link_of: Option<ChannelId>,
    /// The channel at whose next wrap in a `history` frame it closes the connection.
    cut_wrap_of: Option<ChannelId>,
    /// Every queue key object the machines uploaded, in the order they came.
    queue_keys: Vec<QueueKeyUpload>,
    /// The channel at whose next queue key object in a `history` frame it closes the connection.
    cut_queue_key_of: Option<ChannelId>,
    /// The server's `stored` and `send-refused` frames, as it sent them.
    answered: Vec<Frame>,
    answers: Answers,
    history_answers: Answers,
    lose_next_stored: bool,
    /// The channel and the MLS epoch from which on it loses the `stored` answers to the channel's
    /// `mls` envelopes, whatever `answers` says.
    lose_from: Option<(ChannelId, u64)>,
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
    /// Devices whose WebSocket `ack` frames it swallowed.
    swallowed_acks: BTreeSet<DeviceId>,
    watched_delivery: Option<(DeviceId, MessageId, Delivery)>,
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
    /// Whether it holds back the `batch` that ends catch-up until the connection gets a
    /// `space-notice`.
    hold_end_until_notice: bool,
    /// How many such `batch` frames it held back so far.
    held_ends: usize,
    /// Each `space-notice` the server sent: the connection's device and the space it names.
    notices: Vec<(Option<DeviceId>, SpaceId)>,
    /// The device whose `auth` it holds back, with what follows it, while set.
    hold_auth_of: Option<DeviceId>,
    /// How many `auth` frames it held back so far.
    held_auths: usize,
}

/// What the front knows of one connection.
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
    /// Whether the server has sent it a `space-notice`.
    noticed: AtomicBool,
    /// The end of catch-up it holds back until a `space-notice` comes, as it would forward it.
    held_end: Mutex<Option<Vec<u8>>>,
    /// What the machine sent and what the server answered on it, while it is no WebSocket.
    http: Mutex<(Vec<u8>, Vec<u8>)>,
}

impl Link {
    /// Keeps `bytes` of HTTP the machine sent, if `sent`, or the server answered.
    fn http(&self, bytes: &[u8], sent: bool) {
        let mut http = self.http.lock().expect("a connection's HTTP");
        let kept = if sent { &mut http.0 } else { &mut http.1 };
        kept.extend_from_slice(bytes);
    }

    /// Whether the server has answered every request the machine sent whole, as HTTP.
    fn answered_in_full(&self) -> bool {
        let http = self.http.lock().expect("a connection's HTTP");
        answered_in_full(&http.0, &http.1)
    }

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

impl State {
    fn saw_delivery(&mut self, link: &Link, seq: NonZeroU64, envelope: &RawEnvelope) {
        if let Some((device, message, delivery)) = self.watched_delivery.as_mut()
            && link.device() == Some(*device)
            && envelope.envelope().header.msg_id == *message
        {
            delivery.sequences.push(seq);
        }
    }

    /// Whether it loses the `stored` answer to the envelope `msg_id`: one of the channel, at or
    /// after the epoch, that [`Front::lose_stored_from`] names.
    fn loses(&self, msg_id: MessageId) -> bool {
        let Some((channel, from)) = self.lose_from else {
            return false;
        };
        self.sent.iter().any(|sent| {
            let envelope = sent.envelope.envelope();
            envelope.header.msg_id == msg_id
                && envelope.header.channel == channel
                && matches!(
                    MlsFraming::read(&envelope.payload),
                    Some(MlsFraming::Message { epoch, .. }) if epoch >= from
                )
        })
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
        let listener = local_listener();
        let port = listener.local_addr().expect("the front's address").port();
        let shared = Arc::new(Shared {
            target: OnceLock::new(),
            state: Mutex::new(State {
                sent: Vec::new(),
                uploads: Vec::new(),
                cut_link_of: None,
                cut_wrap_of: None,
                queue_keys: Vec::new(),
                cut_queue_key_of: None,
                answered: Vec::new(),
                answers: Answers::Forward,
                history_answers: Answers::Forward,
                lose_next_stored: false,
                lose_from: None,
                lost: 0,
                swallow: false,
                swallowed: 0,
                refused: 0,
                acks: Acks::Forward,
                acked: 0,
                swallowed_acks: BTreeSet::new(),
                watched_delivery: None,
                hold_messages: false,
                held_messages: 0,
                hold_end: false,
                injections: Vec::new(),
                redeliveries: Vec::new(),
                injected: 0,
                hold_end_until_notice: false,
                held_ends: 0,
                notices: Vec::new(),
                hold_auth_of: None,
                held_auths: 0,
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

    /// Every `history` frame the machines sent so far, in the order they came, cut ones included.
    pub fn uploads(&self) -> Vec<Upload> {
        self.shared.state().uploads.clone()
    }

    /// Closes the connection, both ways, at the next `history` frame a machine sends that carries a
    /// link of `channel`; the server never gets it.
    pub fn cut_next_link(&self, channel: ChannelId) {
        self.shared.state().cut_link_of = Some(channel);
    }

    /// Every queue key object the machines uploaded so far, in the order they came, cut ones
    /// included.
    pub fn queue_key_uploads(&self) -> Vec<QueueKeyUpload> {
        self.shared.state().queue_keys.clone()
    }

    /// Closes the connection, both ways, at the next `history` frame a machine sends that carries a
    /// queue key object of `channel`; the server never gets it.
    pub fn cut_next_queue_key(&self, channel: ChannelId) {
        self.shared.state().cut_queue_key_of = Some(channel);
    }

    /// Closes the connection, both ways, at the next `history` frame a machine sends that carries a
    /// wrap of `channel`; the server never gets it.
    pub fn cut_next_wrap(&self, channel: ChannelId) {
        self.shared.state().cut_wrap_of = Some(channel);
    }

    /// The server's `stored` and `send-refused` frames so far, as it sent them.
    pub fn answered(&self) -> Vec<Frame> {
        self.shared.state().answered.clone()
    }

    /// From now on, does `answers` with the server's `stored` answers.
    pub fn answer(&self, answers: Answers) {
        self.shared.state().answers = answers;
    }

    /// Drops the next `stored` frame only.
    pub fn lose_next_stored(&self) {
        self.shared.state().lose_next_stored = true;
    }

    /// From now on, loses the server's `stored` answers to the `mls` envelopes of `channel` of the
    /// MLS epoch `epoch` and later ones, as a network would, and does with the others as
    /// [`Front::answer`] says; with `None`, loses none of them so.
    pub fn lose_stored_from(&self, from: Option<(ChannelId, u64)>) {
        self.shared.state().lose_from = from;
    }

    /// From now on, does `answers` with the server's `history-stored` answers.
    pub fn answer_history(&self, answers: Answers) {
        self.shared.state().history_answers = answers;
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

    /// Whether it swallowed a WebSocket `ack` frame from `device`.
    pub fn swallowed_ack_from(&self, device: DeviceId) -> bool {
        self.shared.state().swallowed_acks.contains(&device)
    }

    /// Records pushes of `message` to `device` and acknowledgements from that device.
    pub fn watch_delivery(&self, device: DeviceId, message: MessageId) {
        self.shared.state().watched_delivery = Some((device, message, Delivery::default()));
    }

    /// The observed sequence numbers for the watched message and whether the device acked them.
    pub fn delivery(&self) -> (Vec<NonZeroU64>, bool) {
        let state = self.shared.state();
        let (_, _, delivery) = state.watched_delivery.as_ref().expect("watch a delivery");
        let acked = delivery
            .sequences
            .iter()
            .any(|seq| Some(*seq) <= delivery.ack);
        (delivery.sequences.clone(), acked)
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

    /// From now on, if `hold`, holds back the `batch` that ends each catch-up until the server
    /// sends that connection a `space-notice`, then forwards the notice and, after it, the `batch`.
    pub fn hold_end_until_notice(&self, hold: bool) {
        self.shared.state().hold_end_until_notice = hold;
    }

    /// How many `batch` frames that end a catch-up it held back until a notice so far.
    pub fn held_ends(&self) -> usize {
        self.shared.state().held_ends
    }

    /// From now on, holds back the `auth` of `device`, if any, and what its machine sends after it,
    /// until called with another device or `None`.
    pub fn hold_auth_of(&self, device: Option<DeviceId>) {
        self.shared.state().hold_auth_of = device;
    }

    /// How many `auth` frames it held back so far.
    pub fn held_auths(&self) -> usize {
        self.shared.state().held_auths
    }

    /// The spaces of the `space-notice` frames the server sent `device` so far, in order.
    pub fn notices_to(&self, device: DeviceId) -> Vec<SpaceId> {
        self.shared
            .state()
            .notices
            .iter()
            .filter(|(to, _)| *to == Some(device))
            .map(|(_, space)| *space)
            .collect()
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
    let server = match reach(&format!("front: connection {connection}"), &target) {
        Ok(server) => server,
        Err(err) => {
            eprintln!("front: connection {connection}: cannot reach {target}: {err}");
            return;
        }
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
            Ok(0) | Err(_) => {
                // The machine left a connection the server has answered in full: it ends with a
                // reset.
                if frames.is_none() && link.answered_in_full() {
                    return reset(&server);
                }
                break;
            }
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
            if text.starts_with(HTTPS_ACK) {
                let acks = {
                    let mut state = shared.state();
                    state.acked += 1;
                    state.acks
                };
                if acks != Acks::Forward {
                    let _ = machine.shutdown(Shutdown::Both);
                    let _ = server.shutdown(Shutdown::Both);
                    return;
                }
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
            link.http(chunk, true);
            if server.write_all(chunk).is_err() {
                break;
            }
            continue;
        };
        pending.extend_from_slice(chunk);
        while let Some((decoded, raw)) = next_frame(pending) {
            if let Some((space, channel, object)) = queue_key_upload(&raw) {
                let mut state = shared.state();
                let cut = state.cut_queue_key_of == Some(channel);
                if cut {
                    state.cut_queue_key_of = None;
                }
                state.queue_keys.push(QueueKeyUpload {
                    space,
                    channel,
                    object,
                    forwarded: !cut,
                });
                drop(state);
                if cut {
                    let _ = machine.shutdown(Shutdown::Both);
                    let _ = server.shutdown(Shutdown::Both);
                    return;
                }
                if server.write_all(&raw).is_err() {
                    return;
                }
                continue;
            }
            let ack_device = match &decoded {
                Some(Frame::Ack { .. }) => link.device(),
                _ => None,
            };
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
                Some(Frame::History {
                    space,
                    channel,
                    item,
                    ..
                }) => {
                    let mut state = shared.state();
                    let cut_link =
                        state.cut_link_of == Some(channel) && matches!(item, HistoryItem::Link(_));
                    let cut_wrap =
                        state.cut_wrap_of == Some(channel) && matches!(item, HistoryItem::Wrap(_));
                    if cut_link {
                        state.cut_link_of = None;
                    }
                    if cut_wrap {
                        state.cut_wrap_of = None;
                    }
                    let cut = cut_link || cut_wrap;
                    state.uploads.push(Upload {
                        space,
                        channel,
                        item,
                        forwarded: !cut,
                    });
                    if cut {
                        drop(state);
                        let _ = machine.shutdown(Shutdown::Both);
                        let _ = server.shutdown(Shutdown::Both);
                        return;
                    }
                    false
                }
                Some(Frame::Auth { device, .. }) => {
                    *link.device.lock().expect("a connection's device") = Some(device);
                    let held = |state: &State| state.hold_auth_of == Some(device);
                    let mut state = shared.state();
                    if held(&state) {
                        state.held_auths += 1;
                    }
                    while held(&state) {
                        drop(state);
                        thread::sleep(Duration::from_millis(10));
                        state = shared.state();
                    }
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
                    if let Some((device, _, delivery)) = shared.state().watched_delivery.as_mut()
                        && link.device() == Some(*device)
                    {
                        delivery.ack = Some(seq).max(delivery.ack);
                    }
                    true
                }
                _ => false,
            };
            if ack {
                let acks = {
                    let mut state = shared.state();
                    state.acked += 1;
                    if let (Acks::Swallow, Some(device)) = (state.acks, ack_device) {
                        state.swallowed_acks.insert(device);
                    }
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
            link.http(chunk, false);
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
                let lose_next = std::mem::take(&mut state.lose_next_stored);
                let answers = if lose_next || state.loses(stored.send_key.msg_id) {
                    Answers::Lose
                } else {
                    state.answers.clone()
                };
                match answers {
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
            Some(Frame::HistoryStored { upload }) => match shared.state().history_answers.clone() {
                Answers::Forward => Some(raw),
                Answers::Lose => None,
                Answers::Refuse(problem) => Some(server_frame(
                    &Frame::HistoryRefused { upload, problem }.to_cbor(),
                )),
            },
            Some(Frame::Push(Push::Message { seq, envelope })) => {
                link.saw(seq.get());
                let mut state = shared.state();
                if state.hold_messages {
                    state.held_messages += 1;
                    None
                } else {
                    state.saw_delivery(link, seq, &envelope);
                    Some(raw)
                }
            }
            Some(Frame::Push(Push::Status { seq, .. })) => {
                link.saw(seq.get());
                Some(raw)
            }
            Some(Frame::Batch(entries)) if entries.is_empty() => end_of_catch_up(raw, shared, link),
            Some(Frame::Push(Push::SpaceNotice { space })) => {
                Some(noticed(&[space], raw, shared, link))
            }
            Some(Frame::Batch(entries)) => {
                let mut state = shared.state();
                for entry in &entries {
                    if let Push::Message { seq, envelope } = entry {
                        state.saw_delivery(link, *seq, envelope);
                    }
                }
                drop(state);
                let spaces: Vec<SpaceId> = entries
                    .iter()
                    .filter_map(|entry| match entry {
                        Push::SpaceNotice { space } => Some(*space),
                        _ => None,
                    })
                    .collect();
                Some(noticed(&spaces, raw, shared, link))
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

/// `raw`, a frame from the server carrying a `space-notice` for each of `spaces`, kept, and after
/// it the end of catch-up the connection held back until a notice, if any.
fn noticed(spaces: &[SpaceId], mut raw: Vec<u8>, shared: &Shared, link: &Link) -> Vec<u8> {
    if spaces.is_empty() {
        return raw;
    }
    let device = link.device();
    shared
        .state()
        .notices
        .extend(spaces.iter().map(|space| (device, *space)));
    link.noticed.store(true, Ordering::SeqCst);
    if let Some(held) = link.held_end.lock().expect("a held end").take() {
        raw.extend(held);
    }
    raw
}

/// The `batch` `raw` that ends a catch-up, as the rules say: held back, or after the envelopes to
/// put into the catch-up of the connection's device, as `message` pushes numbered after the
/// highest `seq` it saw; held back until a `space-notice`, if the connection has had none yet.
fn end_of_catch_up(raw: Vec<u8>, shared: &Shared, link: &Link) -> Option<Vec<u8>> {
    let out = catch_up_end(raw, shared, link)?;
    let mut state = shared.state();
    if state.hold_end_until_notice && !link.noticed.load(Ordering::SeqCst) {
        state.held_ends += 1;
        *link.held_end.lock().expect("a held end") = Some(out);
        return None;
    }
    Some(out)
}

/// The `batch` `raw` that ends a catch-up, as [`end_of_catch_up`] says, but for a hold until a
/// `space-notice`.
fn catch_up_end(raw: Vec<u8>, shared: &Shared, link: &Link) -> Option<Vec<u8>> {
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

/// The space, the channel and the object of a `history` frame whose item is a queue key object,
/// `[11, upload, space, channel, [4, queue-key]]`, in the WebSocket frame `raw`; read from the bytes,
/// whatever the frames the code under test decodes.
fn queue_key_upload(raw: &[u8]) -> Option<(SpaceId, ChannelId, Vec<u8>)> {
    let (frame, _) = ws_frame(raw)?;
    let rest = frame.payload.strip_prefix(&[0x85, 0x0b])?;
    let upload = *rest.first()?;
    let rest = rest.get(head_width(upload, 0)?..)?;
    let (space, rest) = cbor_id(rest)?;
    let (channel, rest) = cbor_id(rest)?;
    let rest = rest.strip_prefix(&[0x82, 0x04])?;
    // The object, a byte string, ends the frame.
    let object = rest.get(head_width(*rest.first()?, 2)?..)?;
    let ids = (SpaceId::from_bytes(space), ChannelId::from_bytes(channel));
    Some((ids.0, ids.1, object.to_vec()))
}

/// How many bytes a CBOR head takes whose first byte is `first`, if it is of major type `major`.
fn head_width(first: u8, major: u8) -> Option<usize> {
    if first >> 5 != major {
        return None;
    }
    match first & 0x1f {
        0..=23 => Some(1),
        24 => Some(2),
        25 => Some(3),
        26 => Some(5),
        27 => Some(9),
        _ => None,
    }
}

/// A 16-byte string at the start of `bytes`, and what follows it.
fn cbor_id(bytes: &[u8]) -> Option<([u8; 16], &[u8])> {
    let rest = bytes.strip_prefix(&[0x50])?;
    let id = rest.get(..16)?.try_into().ok()?;
    Some((id, &rest[16..]))
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
