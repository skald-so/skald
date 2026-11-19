//! A device's connection to one instance, as its reader sees it: mailbox entries, `space-notice`s,
//! `link-notice`s, the end of catch-up and the server's close, each awaited until a deadline.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use skald_proto::{Cbor, Frame, Push, SpaceId};
use tungstenite::Message;

use crate::api::Person;
use crate::common::device_flow::DeviceFlow;
use crate::common::{Closed, Server};
use crate::link::{Entry, QUIET, WAIT};
use crate::wire::Socket;

/// What came from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Came {
    Entry(Entry),
    /// A `space-notice` for the space.
    Notice(SpaceId),
    /// A `link-notice`.
    LinkNotice,
    /// An empty `batch`: the end of catch-up.
    End,
}

/// A connection of a person's certified laptop to one instance, with what came but was not taken
/// yet.
pub struct Watch {
    socket: Socket,
    came: VecDeque<Came>,
    closed: Option<Closed>,
}

impl Watch {
    /// The connection of `person`'s laptop to `server`, which states no cursor yet.
    pub fn on(server: &Server, flow: &DeviceFlow, person: &Person) -> Self {
        Watch {
            socket: Socket::to(server, flow, person, &person.laptop),
            came: VecDeque::new(),
            closed: None,
        }
    }

    /// The connection of `person`'s laptop to `server` from `cursor`, whose catch-up sends
    /// `expected`.
    #[track_caller]
    pub fn from(
        server: &Server,
        flow: &DeviceFlow,
        person: &Person,
        cursor: u64,
        expected: &[Entry],
    ) -> Self {
        let mut watch = Self::on(server, flow, person);
        watch.sync(cursor);
        assert_eq!(watch.catch_up(), expected, "catch-up after {cursor}");
        watch
    }

    /// States `cursor` in `sync`.
    pub fn sync(&mut self, cursor: u64) {
        self.socket.sync(cursor);
    }

    /// Acks up to `seq` in `ack`.
    pub fn ack(&mut self, seq: u64) {
        self.socket.ack(seq);
    }

    /// Reads frames until something comes, the server closes the connection or `deadline` passes.
    fn fill(&mut self, deadline: Instant) {
        while self.came.is_empty() && self.closed.is_none() {
            let bytes = match self.socket.next(deadline) {
                Ok(Some(Message::Binary(bytes))) => bytes,
                Ok(Some(Message::Close(frame))) => {
                    self.closed = Some(Closed::of(frame));
                    return;
                }
                Ok(Some(_)) => continue,
                Ok(None) => return,
                Err(_) => {
                    self.closed = Some(Closed::Dropped);
                    return;
                }
            };
            let frame = Frame::from_cbor(&bytes)
                .unwrap_or_else(|err| panic!("a frame that decodes: {err}: {bytes:02x?}"));
            match frame {
                Frame::Push(push) => self.came.extend(came_of(push)),
                Frame::Batch(pushes) if pushes.is_empty() => self.came.push_back(Came::End),
                Frame::Batch(pushes) => self.came.extend(pushes.into_iter().filter_map(came_of)),
                // Answers to sends, which these connections do not make, and frames of later
                // versions.
                _ => {}
            }
        }
    }

    /// What comes next by `deadline`, if anything; panics if the server closes the connection.
    #[track_caller]
    fn next(&mut self, deadline: Instant) -> Option<Came> {
        self.fill(deadline);
        if let Some(came) = self.came.pop_front() {
            return Some(came);
        }
        if let Some(closed) = &self.closed {
            panic!("the server closed the connection: {closed:?}");
        }
        None
    }

    /// The entries of catch-up, up to its end.
    #[track_caller]
    pub fn catch_up(&mut self) -> Vec<Entry> {
        let deadline = Instant::now() + WAIT;
        let mut entries = Vec::new();
        loop {
            match self.next(deadline) {
                Some(Came::Entry(entry)) => entries.push(entry),
                Some(Came::End) => return entries,
                other => {
                    panic!("an entry or the end of catch-up, not {other:?}, after {entries:?}")
                }
            }
        }
    }

    /// The next `count` entries, all of them within `within` from now.
    #[track_caller]
    pub fn entries_within(&mut self, count: usize, within: Duration) -> Vec<Entry> {
        let deadline = Instant::now() + within;
        let mut entries = Vec::new();
        while entries.len() < count {
            match self.next(deadline) {
                Some(Came::Entry(entry)) => entries.push(entry),
                other => {
                    panic!("{count} entries within {within:?}, not {other:?} after {entries:?}")
                }
            }
        }
        entries
    }

    /// The next entry, which comes within `within` from now.
    #[track_caller]
    pub fn entry_within(&mut self, within: Duration) -> Entry {
        self.entries_within(1, within).remove(0)
    }

    /// The next thing to come, which is a `space-notice` within `within` from now; gives its
    /// space.
    #[track_caller]
    pub fn notice_within(&mut self, within: Duration) -> SpaceId {
        match self.next(Instant::now() + within) {
            Some(Came::Notice(space)) => space,
            other => panic!("a space-notice within {within:?}, not {other:?}"),
        }
    }

    /// Checks that the next thing to come is a `link-notice`, within `within` from now.
    #[track_caller]
    pub fn link_notice_within(&mut self, within: Duration) {
        match self.next(Instant::now() + within) {
            Some(Came::LinkNotice) => {}
            other => panic!("a link-notice within {within:?}, not {other:?}"),
        }
    }

    /// Checks that nothing comes within `QUIET`.
    #[track_caller]
    pub fn assert_quiet(&mut self) {
        if let Some(came) = self.next(Instant::now() + QUIET) {
            panic!("nothing, not {came:?}");
        }
    }

    /// How the server ends this connection within `within` from now, skipping what comes before;
    /// `None` if it is still open by then.
    pub fn closed_within(&mut self, within: Duration) -> Option<Closed> {
        let deadline = Instant::now() + within;
        loop {
            self.came.clear();
            self.fill(deadline);
            if self.closed.is_some() || self.came.is_empty() {
                return self.closed.clone();
            }
        }
    }
}

/// What `push` says to a reader; nothing for a `batch` entry of a type this version does not know.
fn came_of(push: Push) -> Option<Came> {
    match push {
        Push::SpaceNotice { space } => Some(Came::Notice(space)),
        Push::LinkNotice => Some(Came::LinkNotice),
        push => Entry::of(push).map(Came::Entry),
    }
}
