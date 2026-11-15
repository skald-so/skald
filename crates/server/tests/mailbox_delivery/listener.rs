//! A device's connection as a reader of notices sees it: it states its cursor, reads its catch-up
//! to the end, then the `space-notice` frames the server sends it.

use std::time::Instant;

use skald_proto::{Cbor as _, CertBody, DeviceKind, Frame, Push, SpaceId, Unknown};
use tungstenite::Message;

use crate::api::Person;
use crate::common::TestDb;
use crate::common::device_flow::DeviceFlow;
use crate::common::devices::{Device, cert_of};
use crate::link::{QUIET, WAIT};
use crate::wire::{Socket, array_of, uint};

/// What a frame from the server says to a reader of notices.
enum Read {
    /// The spaces of the `space-notice` frames it carries.
    Notices(Vec<SpaceId>),
    /// An empty `batch`: the end of catch-up.
    End,
    /// Another frame.
    Other,
}

fn read_of(bytes: &[u8]) -> Read {
    let notice = |push: &Push| match push {
        Push::SpaceNotice { space } => Some(*space),
        _ => None,
    };
    match Frame::from_cbor(bytes) {
        Ok(Frame::Push(push)) => Read::Notices(notice(&push).into_iter().collect()),
        Ok(Frame::Batch(pushes)) if pushes.is_empty() => Read::End,
        Ok(Frame::Batch(pushes)) => Read::Notices(pushes.iter().filter_map(notice).collect()),
        Ok(_) => Read::Other,
        Err(err) => panic!("a frame that decodes: {err}: {bytes:02x?}"),
    }
}

/// A connection of a device, past its catch-up.
pub struct Listener {
    socket: Socket,
    /// Whose device it is, for messages.
    who: String,
}

impl Listener {
    /// The connection of `person`'s certified laptop, past its catch-up.
    #[track_caller]
    pub fn of(flow: &DeviceFlow, person: &Person) -> Self {
        Self::device(flow, person, &person.laptop)
    }

    /// The connection of `device`, a certified device of `person`, on a challenge of `person`'s
    /// session: it states the cursor 0 and reads its catch-up, which holds no notice, to its end.
    #[track_caller]
    pub fn device(flow: &DeviceFlow, person: &Person, device: &Device) -> Self {
        let socket = Socket::to(&flow.stand.server, flow, person, device);
        let mut listener = Listener {
            socket,
            who: format!("{}'s {}", person.handle, device.name),
        };
        listener.socket.frame(array_of(&[uint(4), uint(0)]));
        let deadline = Instant::now() + WAIT;
        loop {
            match listener.read(deadline) {
                Some(Read::End) => return listener,
                Some(Read::Notices(spaces)) if !spaces.is_empty() => {
                    panic!("{}: a notice in catch-up: {spaces:?}", listener.who)
                }
                Some(_) => {}
                None => panic!("{}: no end of catch-up within {WAIT:?}", listener.who),
            }
        }
    }

    /// The next read by `deadline`, if any.
    fn read(&mut self, deadline: Instant) -> Option<Read> {
        loop {
            match self.socket.next(deadline) {
                Ok(Some(Message::Binary(bytes))) => return Some(read_of(&bytes)),
                Ok(Some(Message::Close(frame))) => {
                    panic!("{}: the server closed: {frame:?}", self.who)
                }
                Ok(Some(_)) => {}
                Ok(None) => return None,
                Err(err) => panic!("{}: the connection failed: {err}", self.who),
            }
        }
    }

    /// The spaces of the notices that come by `deadline`, at least one if `one` and then no more.
    fn notices_by(&mut self, deadline: Instant, one: bool) -> Vec<SpaceId> {
        loop {
            match self.read(deadline) {
                Some(Read::Notices(spaces)) if !spaces.is_empty() => {
                    if one {
                        return spaces;
                    }
                    panic!("{}: no notice, not {spaces:?}", self.who);
                }
                Some(Read::End) => panic!("{}: a second end of catch-up", self.who),
                Some(_) => {}
                None => return Vec::new(),
            }
        }
    }

    /// Checks that a notice of `space` comes within [`WAIT`].
    #[track_caller]
    pub fn assert_notice(&mut self, space: SpaceId) {
        let deadline = Instant::now() + WAIT;
        loop {
            let spaces = self.notices_by(deadline, true);
            assert!(
                !spaces.is_empty(),
                "{}: no notice of {space} within {WAIT:?}",
                self.who
            );
            if spaces.contains(&space) {
                return;
            }
        }
    }

    /// Checks that no notice comes within [`QUIET`].
    #[track_caller]
    pub fn assert_no_notice(&mut self) {
        self.notices_by(Instant::now() + QUIET, false);
    }
}

/// A second `client` device of `person`'s account, named `name`, with a certificate issued by the
/// account's master key, written to the server's database.
pub fn second_device(db: &TestDb, person: &Person, name: &str) -> Device {
    let device = Device::new(name);
    let body = CertBody {
        account: person.account,
        device: device.id,
        kind: DeviceKind::Client,
        signing: device.signing.public(),
        hpke: device.hpke,
        issuer: None,
        issued: 1_794_000_000,
        ssh: None,
        name: name.to_owned(),
        unknown: Unknown::default(),
    };
    let cert = cert_of(&body, &person.master.key, &device.signing.key);
    let bytes = |bytes: &[u8]| {
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        format!("'\\x{hex}'::bytea")
    };
    db.execute(&format!(
        "INSERT INTO devices (id, account_id, kind, signing, hpke, name, cert) \
         VALUES ('{}'::uuid, '{}'::uuid, 'client', {}, {}, {}, {})",
        device.id,
        person.account,
        bytes(&device.signing.public()),
        bytes(&device.hpke),
        bytes(name.as_bytes()),
        bytes(&cert),
    ));
    device
}
