//! What the tests share: revocations the tests sign, the requests that store and list them,
//! devices certified by another device or another master key as the database holds them, and how a
//! device's connections fare.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_crypto::{SigningKey, sign_revocation};
use skald_proto::{
    Cbor, DeviceId, DeviceKind, Frame, NonEmpty, REVOCATION_LABEL, Revocation, RevocationBody,
    SpaceId, Unknown,
};
use tungstenite::Message;
use tungstenite::protocol::frame::coding::CloseCode;

use crate::api::{Person, Session, assert_refused};
use crate::common::device_flow::{DeviceFlow, json, text};
use crate::common::devices::{DEVICES, Device, Key, b64, cert_of, unb64, write_devices};
use crate::common::{Closed, Response, Server, TestDb};
use crate::fixture::Fixture;
use crate::rosters::with_later_field;
use crate::wire::Socket;

/// Stores a revocation: `POST`.
pub const REVOCATIONS: &str = "/account/revocations";
/// The problem of a revocation the server does not store.
pub const INVALID_REVOCATION: &str = "https://skald.so/problems/invalid-revocation";
/// The reason a connection's proof is refused for its device.
pub const DEVICE_REFUSED: &str = "device-refused";
/// How soon a revoked device's connections close, on every instance.
pub const CLOSE: Duration = Duration::from_secs(1);
/// How long a test waits for a connection to be taken or refused.
const WAIT: Duration = Duration::from_secs(15);
/// The `timestamp` of the tests' revocations.
const TIMESTAMP: u64 = 1_794_000_000;

/// The revocation body of `device` on `person`'s account by `signer`, the master key if `None`,
/// keeping nothing.
pub fn body(person: &Person, device: DeviceId, signer: Option<DeviceId>) -> RevocationBody {
    RevocationBody {
        account: person.account,
        device,
        keep: None,
        signer,
        timestamp: TIMESTAMP,
        also: None,
        unknown: Unknown::default(),
    }
}

/// `body` signed by `key`, encoded.
pub fn signed(body: &RevocationBody, key: &SigningKey) -> Vec<u8> {
    sign_revocation(body, key)
        .expect("a revocation body the spec allows")
        .to_cbor()
}

/// `body` with a field of a later version, key 11, signed by `key` over the bytes with it,
/// encoded.
pub fn signed_with_later_field(body: &RevocationBody, key: &SigningKey) -> Vec<u8> {
    let bytes = with_later_field(&body.to_cbor());
    let signature = key.sign(REVOCATION_LABEL, &[&bytes]);
    Revocation::from_parts(bytes, signature)
        .expect("a revocation body with a later field decodes")
        .to_cbor()
}

/// Writes to the fixture's database a new `client` device of bob's account named `name`, certified
/// by bob's laptop; gives it.
pub fn by_bobs_laptop(f: &Fixture, name: &str) -> Device {
    let bob = &f.team.bob;
    bob.certified_by(f.db(), &bob.laptop, DeviceKind::Client, name)
}

/// The IDs `devices` as a revocation's `keep` or `also`.
pub fn ids(devices: &[DeviceId]) -> Option<NonEmpty<DeviceId>> {
    NonEmpty::from_vec(devices.to_vec())
}

impl Person {
    /// The revocation of `device` by this person's master key, keeping `keep`.
    pub fn by_master(&self, device: DeviceId, keep: &[DeviceId]) -> Vec<u8> {
        let mut body = body(self, device, None);
        body.keep = ids(keep);
        signed(&body, &self.master.key)
    }

    /// The revocation of `device` by `signer`, a device of this person's account.
    pub fn by_device(&self, signer: &Device, device: DeviceId) -> Vec<u8> {
        signed(&body(self, device, Some(signer.id)), &signer.signing.key)
    }

    /// Stores `revocation` from this person's session, which the server must take as new: `201`.
    #[track_caller]
    pub fn revoke(&self, revocation: &[u8]) {
        let response = self.session.revoke(revocation);
        assert_eq!(response.status, 201, "stored: {}", text(&response));
    }

    /// Writes to `db` a new device of `kind` on this person's account, certified by `issuer`, one of
    /// its devices; gives it.
    pub fn certified_by(
        &self,
        db: &TestDb,
        issuer: &Device,
        kind: DeviceKind,
        name: &str,
    ) -> Device {
        let device = Device::new(name);
        let mut cert = device.body_as(self.account, kind);
        cert.issuer = Some(issuer.id);
        let cert = cert_of(&cert, &issuer.signing.key, &device.signing.key);
        write_devices(db, self.account, kind, &[(&device, Some(&cert))]);
        device
    }

    /// Writes to `db` a new `client` device on this person's account, certified by `master`, such
    /// as another account's master key; gives it.
    pub fn certified_by_key(&self, db: &TestDb, master: &Key, name: &str) -> Device {
        let device = Device::new(name);
        let cert = device.cert_as(self.account, DeviceKind::Client, master);
        write_devices(
            db,
            self.account,
            DeviceKind::Client,
            &[(&device, Some(&cert))],
        );
        device
    }
}

impl Session {
    /// `POST /account/revocations` of `revocation`'s bytes.
    pub fn revoke(&self, revocation: &[u8]) -> Response {
        self.revoke_body(&json!({ "revocation": b64(revocation) }))
    }

    /// `POST /account/revocations` of `body`.
    pub fn revoke_body(&self, body: &Value) -> Response {
        self.call("post", REVOCATIONS, Some(body))
    }

    /// The revocations `GET /devices` gives this session's account, sorted.
    pub fn revocations(&self) -> Vec<Vec<u8>> {
        revocations_of(&self.call("get", DEVICES, None))
    }

    /// The revocations `GET /spaces/<space>/devices` gives this session, sorted.
    pub fn space_revocations(&self, space: SpaceId) -> Vec<Vec<u8>> {
        revocations_of(&self.devices_response(space))
    }
}

/// The revocations in `response`, a `GET /devices` or `GET /spaces/<space>/devices` answer, each
/// as its bytes, sorted.
pub fn revocations_of(response: &Response) -> Vec<Vec<u8>> {
    assert_eq!(response.status, 200, "{}", text(response));
    let body = json(response);
    let mut found: Vec<Vec<u8>> = body["revocations"]
        .as_array()
        .unwrap_or_else(|| panic!("no revocations in {body}"))
        .iter()
        .map(|revocation| unb64(revocation.as_str().expect("a revocation's bytes")))
        .collect();
    found.sort();
    found
}

/// `revocations`, sorted.
pub fn sorted(revocations: &[&[u8]]) -> Vec<Vec<u8>> {
    let mut sorted: Vec<Vec<u8>> = revocations.iter().map(|bytes| bytes.to_vec()).collect();
    sorted.sort();
    sorted
}

/// Checks that `response` refuses a revocation: `400` with the problem `invalid-revocation`;
/// `what` names the case.
#[track_caller]
pub fn assert_invalid(response: &Response, what: &str) {
    assert_refused(response, what);
    let problem = (response.status, json(response)["type"].clone());
    assert_eq!(
        problem,
        (400, Value::from(INVALID_REVOCATION)),
        "{what}: {}",
        text(response)
    );
}

/// The connection of `device` of `person` to `server`, an instance of `flow`'s server.
pub fn connect(server: &Server, flow: &DeviceFlow, person: &Person, device: &Device) -> Socket {
    Socket::to(server, flow, person, device)
}

/// Checks that the server took `socket`: its first `sync`, from cursor 0, gets catch-up up to its
/// end, an empty `batch`.
#[track_caller]
pub fn assert_open(socket: &mut Socket) {
    socket.sync(0);
    let deadline = Instant::now() + WAIT;
    loop {
        match socket.next(deadline) {
            Ok(Some(Message::Binary(bytes))) => {
                let frame = Frame::from_cbor(&bytes);
                if matches!(frame, Ok(Frame::Batch(pushes)) if pushes.is_empty()) {
                    return;
                }
            }
            Ok(Some(Message::Close(frame))) => panic!("closed, not taken: {frame:?}"),
            Ok(Some(_)) => {}
            Ok(None) => panic!("no end of catch-up within {WAIT:?}"),
            Err(err) => panic!("the connection failed, not taken: {err}"),
        }
    }
}

/// Checks that the server refuses `socket`'s proof for its device: it closes with code 1008 and
/// the reason `device-refused`.
#[track_caller]
pub fn assert_device_refused(socket: &mut Socket) {
    let deadline = Instant::now() + WAIT;
    loop {
        match socket.next(deadline) {
            Ok(Some(Message::Close(frame))) => {
                let expected = Closed::Frame(CloseCode::Policy, DEVICE_REFUSED.to_owned());
                assert_eq!(Closed::of(frame), expected, "the proof refused");
                return;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("still open after {WAIT:?}, not refused"),
            Err(err) => panic!("dropped without a close frame: {err}"),
        }
    }
}

/// Checks that the server closes `socket`, with a close frame or without, within [`CLOSE`] of
/// `since`.
#[track_caller]
pub fn assert_closes(socket: &mut Socket, since: Instant, what: &str) {
    assert_closes_within(socket, since, CLOSE, what);
}

/// Checks that the server closes `socket`, with a close frame or without, within `within` of
/// `since`.
#[track_caller]
pub fn assert_closes_within(socket: &mut Socket, since: Instant, within: Duration, what: &str) {
    let deadline = since + within;
    loop {
        match socket.next(deadline) {
            Ok(Some(Message::Close(_))) | Err(_) => return,
            Ok(Some(_)) => {}
            Ok(None) => panic!("{what}: still open {within:?} after the revocation"),
        }
    }
}
