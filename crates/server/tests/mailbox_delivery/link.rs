//! A device's connection as its mailbox's reader sees it: the frames it sends, and the entries,
//! answers and ends of catch-up the server sends it, in the order they come.

use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{
    Cbor, DeliveryStatus, DeviceId, Frame, Header, PROTOCOL_VERSION, Push, ServerStatus, Stored,
};
use tungstenite::Message;

use crate::api::Person;
use crate::common::device_flow::DeviceFlow;
use crate::common::{Server, TestDb};
use crate::fixture::Fixture;
use crate::wire::{
    Socket, array_of, assert_stored_at, byte_item, envelope, key_of, now, query, uint,
};

/// How long a test waits for the end of catch-up, an answer or a database change.
pub const WAIT: Duration = Duration::from_secs(15);
/// How soon the server pushes an entry once it is added: at once.
pub const PUSH: Duration = Duration::from_secs(3);
/// How long a test watches a connection to see that nothing comes.
pub const QUIET: Duration = Duration::from_secs(2);

/// A mailbox entry as the server sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A `message`: its `seq` and its envelope's bytes.
    Message(u64, Vec<u8>),
    /// A `status`: its `seq` and the server status.
    Status(u64, DeliveryStatus),
}

impl Entry {
    pub fn seq(&self) -> u64 {
        match self {
            Self::Message(seq, _) | Self::Status(seq, _) => *seq,
        }
    }
}

/// What a frame from the server says to a mailbox's reader.
#[derive(Debug)]
enum Read {
    Entry(Entry),
    /// An empty `batch`: catch-up has sent every entry the mailbox held at the first `sync`.
    End,
    Stored(Stored),
    Refused(String),
}

/// What `frame` says; nothing for a frame these tests do not read, such as a `space-notice`.
fn reads_of(frame: Frame) -> Vec<Read> {
    let entry = |push: Push| match push {
        Push::Message { seq, envelope } => Some(Read::Entry(Entry::Message(
            seq.get(),
            envelope.bytes().to_vec(),
        ))),
        Push::Status { seq, status } => Some(Read::Entry(Entry::Status(seq.get(), status))),
        Push::SpaceNotice { .. } | Push::Unknown(_) => None,
    };
    match frame {
        Frame::Push(push) => entry(push).into_iter().collect(),
        Frame::Batch(pushes) if pushes.is_empty() => vec![Read::End],
        Frame::Batch(pushes) => pushes.into_iter().filter_map(entry).collect(),
        Frame::Stored(stored) => vec![Read::Stored(stored)],
        Frame::SendRefused { problem, .. } => vec![Read::Refused(problem)],
        // A client ignores a frame of a type it does not know.
        Frame::Unknown(_) => Vec::new(),
        other => panic!("a frame a server does not send: {other:?}"),
    }
}

/// A device's connection, authenticated as the device, with what came but was not read yet.
pub struct Link {
    socket: Socket,
    pending: VecDeque<Read>,
}

impl Link {
    /// The connection of `person`'s certified laptop.
    pub fn of(flow: &DeviceFlow, person: &Person) -> Self {
        Self::stating(&flow.stand.server, flow, person, PROTOCOL_VERSION)
    }

    /// The connection of `person`'s certified laptop to `server`, an instance of `flow`'s server,
    /// of the protocol version `version`.
    pub fn stating(
        server: &Server,
        flow: &DeviceFlow,
        person: &Person,
        version: NonZeroU64,
    ) -> Self {
        Link {
            socket: Socket::stating(server, flow, person, &person.laptop, version),
            pending: VecDeque::new(),
        }
    }

    /// The connection of `person`'s certified laptop, which states `cursor` in its first `sync`
    /// and checks that catch-up sends `expected`.
    #[track_caller]
    pub fn synced(flow: &DeviceFlow, person: &Person, cursor: u64, expected: &[Entry]) -> Self {
        let mut link = Self::of(flow, person);
        link.sync(cursor);
        assert_eq!(link.catch_up(), expected, "catch-up after {cursor}");
        link
    }

    /// States the cursor in `sync`.
    pub fn sync(&mut self, cursor: u64) {
        self.socket
            .frame(array_of(&[uint(4), uint(usize::try_from(cursor).unwrap())]));
    }

    /// Acks up to `seq` in `ack`.
    pub fn ack(&mut self, seq: u64) {
        self.socket
            .frame(array_of(&[uint(7), uint(usize::try_from(seq).unwrap())]));
    }

    /// Sends `envelope` in a `send` with `report`.
    pub fn send(&mut self, envelope: &[u8], report: bool) {
        let report = vec![if report { 0xf5 } else { 0xf4 }];
        self.socket
            .frame(array_of(&[uint(1), byte_item(envelope), report]));
    }

    /// The next reads from the server by `deadline`, none if nothing comes by then.
    fn receive(&mut self, deadline: Instant) -> Vec<Read> {
        loop {
            match self.socket.next(deadline) {
                Ok(Some(Message::Binary(bytes))) => {
                    let frame = Frame::from_cbor(&bytes)
                        .unwrap_or_else(|err| panic!("a frame that decodes: {err}: {bytes:02x?}"));
                    let reads = reads_of(frame);
                    if !reads.is_empty() {
                        return reads;
                    }
                }
                Ok(Some(Message::Close(frame))) => panic!("the server closed: {frame:?}"),
                Ok(Some(_)) => {}
                Ok(None) => return Vec::new(),
                Err(err) => panic!("the connection failed: {err}"),
            }
        }
    }

    /// The next read by `deadline`, what came earlier first.
    fn read(&mut self, deadline: Instant) -> Option<Read> {
        if self.pending.is_empty() {
            let reads = self.receive(deadline);
            self.pending.extend(reads);
        }
        self.pending.pop_front()
    }

    /// Sends `envelope` of `header` with `report` and checks that the server stores it; gives the
    /// `stored`. Entries that come meanwhile wait for the next read.
    #[track_caller]
    pub fn store(&mut self, header: &Header, envelope: &[u8], report: bool) -> Stored {
        let before = now();
        self.send(envelope, report);
        let deadline = Instant::now() + WAIT;
        let mut others = VecDeque::new();
        let stored = loop {
            let reads = self.receive(deadline);
            assert!(!reads.is_empty(), "no answer within {WAIT:?}");
            let mut answer = None;
            for read in reads {
                match read {
                    Read::Stored(stored) if answer.is_none() => answer = Some(stored),
                    Read::Refused(problem) => panic!("stored, not send-refused: {problem}"),
                    read => others.push_back(read),
                }
            }
            if let Some(stored) = answer {
                break stored;
            }
        };
        self.pending.extend(others);
        assert_eq!(stored.send_key, key_of(header));
        assert_stored_at(&stored, before);
        stored
    }

    /// The entries of catch-up, up to its empty `batch`.
    #[track_caller]
    pub fn catch_up(&mut self) -> Vec<Entry> {
        let deadline = Instant::now() + WAIT;
        let mut entries = Vec::new();
        loop {
            match self.read(deadline) {
                Some(Read::Entry(entry)) => entries.push(entry),
                Some(Read::End) => return entries,
                Some(other) => panic!("an entry or the end of catch-up, not {other:?}"),
                None => panic!("no end of catch-up within {WAIT:?}, after {entries:?}"),
            }
        }
    }

    /// The next entry, which comes within `PUSH`.
    #[track_caller]
    pub fn entry(&mut self) -> Entry {
        self.entry_within(PUSH)
    }

    /// The next entry, which comes within `within`.
    #[track_caller]
    pub fn entry_within(&mut self, within: Duration) -> Entry {
        match self.read(Instant::now() + within) {
            Some(Read::Entry(entry)) => entry,
            Some(other) => panic!("an entry, not {other:?}"),
            None => panic!("no entry within {within:?}"),
        }
    }

    /// The next `count` entries, each pushed within `PUSH` of the one before.
    #[track_caller]
    pub fn entries(&mut self, count: usize) -> Vec<Entry> {
        (0..count).map(|_| self.entry()).collect()
    }

    /// Checks that no entry and no end of catch-up comes within `QUIET`.
    #[track_caller]
    pub fn assert_quiet(&mut self) {
        if let Some(read) = self.read(Instant::now() + QUIET) {
            panic!("nothing, not {read:?}");
        }
    }
}

/// A send: its header and its envelope's bytes.
pub type Sent = (Header, Vec<u8>);

/// Stores `count` envelopes on `link` to `recipient` in `fixture`'s channel, with `report`.
#[track_caller]
pub fn store_to(
    link: &mut Link,
    fixture: &Fixture,
    recipient: DeviceId,
    count: usize,
    report: bool,
) -> Vec<Sent> {
    (0..count)
        .map(|_| {
            let header = fixture.to(recipient);
            let sent = envelope(&header);
            link.store(&header, &sent, report);
            (header, sent)
        })
        .collect()
}

/// The envelopes of `sends`.
pub fn bytes(sends: &[Sent]) -> Vec<Vec<u8>> {
    sends.iter().map(|(_, sent)| sent.clone()).collect()
}

/// The `message` entries of `envelopes`, from `seq` `first` on.
pub fn messages(first: u64, envelopes: &[Vec<u8>]) -> Vec<Entry> {
    (first..)
        .zip(envelopes)
        .map(|(seq, envelope)| Entry::Message(seq, envelope.clone()))
        .collect()
}

/// Checks that `entry` is a `status` of `delivered` for the message of `header` to `recipient`,
/// added at `before` or later and not after now; gives its `seq`.
#[track_caller]
pub fn assert_delivered(entry: &Entry, header: &Header, recipient: DeviceId, before: u64) -> u64 {
    assert_status(entry, ServerStatus::Delivered, header, recipient, before)
}

/// Checks that `entry` is a `status` of `server_status` for the message of `header` to
/// `recipient`, which happened at `before` or later and not after now; gives its `seq`.
#[track_caller]
pub fn assert_status(
    entry: &Entry,
    server_status: ServerStatus,
    header: &Header,
    recipient: DeviceId,
    before: u64,
) -> u64 {
    let Entry::Status(seq, status) = entry else {
        panic!("a status, not {entry:?}");
    };
    assert_eq!(status.status, server_status, "{status:?}");
    assert_eq!(status.msg_id, header.msg_id, "the status's msg_id");
    assert_eq!(status.device, recipient, "the status's recipient device");
    let range = before.saturating_sub(2)..=now() + 2;
    assert!(
        range.contains(&status.timestamp),
        "{server_status:?} at {}, not in {range:?}",
        status.timestamp
    );
    *seq
}

/// The `seq`s of `device`'s mailbox, `message` and `status` entries alike, in order.
pub fn seqs(db: &TestDb, device: DeviceId) -> Vec<i64> {
    let device = device.to_string();
    query(db, async |conn| {
        sqlx::query_scalar(
            "SELECT seq::bigint FROM mailbox_entries WHERE device_id::text = $1 ORDER BY seq",
        )
        .bind(device)
        .fetch_all(conn)
        .await
    })
}

/// Waits until the `seq`s of `device`'s mailbox are `expected`, as after an ack, which has no
/// answer.
#[track_caller]
pub fn await_seqs(db: &TestDb, device: DeviceId, expected: &[i64]) {
    await_seqs_within(db, device, expected, WAIT);
}

/// As [`await_seqs`], waiting up to `within`.
#[track_caller]
pub fn await_seqs_within(db: &TestDb, device: DeviceId, expected: &[i64], within: Duration) {
    let deadline = Instant::now() + within;
    loop {
        let found = seqs(db, device);
        if found == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{device}'s mailbox holds {found:?}, not {expected:?}, after {within:?}"
        );
        thread::sleep(Duration::from_millis(100));
    }
}
