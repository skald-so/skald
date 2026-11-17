//! A device's mailbox over HTTPS requests: the proof each request carries, the requests and their
//! answers, and long-polls that wait in a thread of their own.

use std::num::NonZeroU64;
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_crypto::SigningKey;
use skald_proto::{
    Cbor, DeviceId, Frame, Header, PROTOCOL_VERSION, SpaceId, Stored, VERSION_HEADER,
};

use crate::common::spec::{operation_violations, spec};
use crate::common::{Response, Server, send};
use crate::common::{
    cbor::{array_of, byte_item, uint},
    device_flow::{DeviceFlow, json, text},
    devices::{CHALLENGE, Device, b64, unb64},
};
use crate::link::{Entry, Sent};
use crate::wire::{Answer, assert_stored_at, envelope, key_of, now, send_frame};
use crate::{api::Person, fixture::Fixture};

/// Sends an envelope: `POST`, the body a `send` frame.
pub const SEND: &str = "/mailbox/send";
/// Acks: `POST`, the body an `ack` frame.
pub const ACK: &str = "/mailbox/ack";
/// The long-poll: `POST /mailbox/poll?wait=<seconds>`, the body a `sync` frame.
pub const POLL: &str = "/mailbox/poll";
/// The request header that carries the proof.
pub const PROOF: &str = "skald-device-proof";
/// The label of the proof's signature.
pub const REQUEST_LABEL: &str = "skald request";
/// The label of the signature in a WebSocket's `auth`.
pub const AUTH_LABEL: &str = "skald auth";
/// The server setting of the longest wait of a long-poll, in seconds.
pub const MAX_WAIT: &str = "--long-poll-max-wait";
/// The media type of the frames in bodies.
pub const CBOR: &str = "application/cbor";

/// The refusals of a request's proof, each a problem `https://skald.so/problems/<name>`.
pub const AUTH_REQUIRED: &str = "auth-required";
pub const CHALLENGE_REFUSED: &str = "challenge-refused";
pub const DEVICE_REFUSED: &str = "device-refused";
pub const SIGNATURE_REFUSED: &str = "signature-refused";

/// How soon a waiting long-poll answers what is added, through any instance: at once.
pub const WAKE: Duration = Duration::from_secs(1);
/// How soon a long-poll with a wait of 0 answers.
pub const AT_ONCE: Duration = Duration::from_secs(1);
/// How late past its wait a long-poll may answer.
pub const SLACK: Duration = Duration::from_secs(2);
/// How early before its wait a long-poll may answer: the server counts in its own time.
pub const EARLY: Duration = Duration::from_millis(100);
/// How long a test lets a long-poll it started reach the server and wait there.
pub const SETTLE: Duration = Duration::from_millis(700);
/// The wait of a long-poll that something else must end first.
pub const LONG: u64 = 20;

/// The spec, read once.
fn the_spec() -> &'static Value {
    static SPEC: OnceLock<Value> = OnceLock::new();
    SPEC.get_or_init(spec)
}

/// Checks that `response` to `POST <path>` conforms to the spec.
#[track_caller]
pub fn conforms(path: &str, response: &Response) {
    let violations = operation_violations(the_spec(), "post", path, response);
    assert_eq!(violations, Vec::<String>::new(), "{}", text(response));
}

/// The proof's CBOR item: `[challenge: bstr, device: id, signature: bstr .size 64]`.
pub fn proof_bytes(challenge: &[u8], device: DeviceId, signature: &[u8]) -> Vec<u8> {
    array_of(&[
        byte_item(challenge),
        byte_item(device.as_bytes()),
        byte_item(signature),
    ])
}

/// The signature of a proof by `key` with `label` over `challenge` and `origin`.
pub fn signature(key: &SigningKey, label: &str, challenge: &[u8], origin: &str) -> [u8; 64] {
    key.sign(label, &[challenge, origin.as_bytes()])
}

/// The proof header's value: the proof of `device` on `challenge`, signed by `key` over `origin`
/// with the label of requests, in base64url without padding.
pub fn proof(challenge: &[u8], device: DeviceId, key: &SigningKey, origin: &str) -> String {
    labelled(REQUEST_LABEL, challenge, device, key, origin)
}

/// As [`proof`], signed with `label`.
pub fn labelled(
    label: &str,
    challenge: &[u8],
    device: DeviceId,
    key: &SigningKey,
    origin: &str,
) -> String {
    let signature = signature(key, label, challenge, origin);
    b64(&proof_bytes(challenge, device, &signature))
}

/// The `sync` frame of `cursor`.
pub fn sync_frame(cursor: u64) -> Vec<u8> {
    array_of(&[uint(4), uint(cursor)])
}

/// The `ack` frame of `seq`.
pub fn ack_frame(seq: u64) -> Vec<u8> {
    array_of(&[uint(7), uint(seq)])
}

/// The path and query of a long-poll that waits up to `wait` seconds.
pub fn poll_path(wait: u64) -> String {
    format!("{POLL}?wait={wait}")
}

/// The problem type of `name`.
pub fn problem(name: &str) -> String {
    format!("https://skald.so/problems/{name}")
}

/// An HTTPS request a test has built: sent now or in a thread of its own.
pub struct Request {
    /// The spec's path of its operation.
    path: &'static str,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    /// Sends it; checks the answer against the spec.
    pub fn call(&self) -> Response {
        let response = self.call_unchecked();
        conforms(self.path, &response);
        response
    }

    fn call_unchecked(&self) -> Response {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        send("post", &self.url, &headers, &self.body)
    }

    /// Sends it in a thread of its own, and lets it reach the server.
    pub fn start(self) -> Waiting {
        let started = Instant::now();
        let path = self.path;
        let thread = thread::spawn(move || {
            let response = self.call_unchecked();
            (response, Instant::now())
        });
        thread::sleep(SETTLE);
        Waiting {
            path,
            started,
            thread,
        }
    }
}

/// A request sent in a thread of its own.
pub struct Waiting {
    path: &'static str,
    /// When it was sent.
    pub started: Instant,
    thread: JoinHandle<(Response, Instant)>,
}

impl Waiting {
    /// Its answer, checked against the spec, and when it came.
    pub fn answer(self) -> (Response, Instant) {
        let (response, at) = self.thread.join().expect("the request's thread");
        conforms(self.path, &response);
        (response, at)
    }

    /// Checks that it has not been answered yet.
    #[track_caller]
    pub fn assert_waiting(&self) {
        assert!(!self.thread.is_finished(), "answered before its wait ended");
    }
}

/// A long-poll's answer: its entries in the order they came, and the spaces of its `space-notice`s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    pub entries: Vec<Entry>,
    pub notices: Vec<SpaceId>,
}

/// The `batch` that `response` answers, `200` with a `batch` frame.
#[track_caller]
pub fn batch_of(response: &Response) -> Batch {
    assert_frame(response);
    let frame = Frame::from_cbor(&response.body)
        .unwrap_or_else(|err| panic!("a frame that decodes: {err}: {:02x?}", response.body));
    let Frame::Batch(pushes) = frame else {
        panic!("a batch, not {frame:?}");
    };
    let mut batch = Batch::default();
    for push in pushes {
        match push {
            skald_proto::Push::SpaceNotice { space } => batch.notices.push(space),
            push => batch.entries.extend(Entry::of(push)),
        }
    }
    batch
}

/// The `stored` or `send-refused` that `response` answers, `200` with the frame.
#[track_caller]
pub fn answer_of(response: &Response) -> Answer {
    assert_frame(response);
    match Frame::from_cbor(&response.body) {
        Ok(Frame::Stored(stored)) => Answer::Stored(stored),
        Ok(Frame::SendRefused { send_key, problem }) => Answer::Refused(send_key, problem),
        other => panic!("stored or send-refused, not {other:?}"),
    }
}

/// Checks that `response` is `200` with a frame.
#[track_caller]
fn assert_frame(response: &Response) {
    assert_eq!(response.status, 200, "{}", text(response));
    assert!(
        response.content_type.starts_with(CBOR),
        "a frame, not {:?}",
        response.content_type
    );
}

/// Checks that `response` refuses with `status` and a problem details object, of the problem
/// `name` if given.
#[track_caller]
pub fn assert_problem(response: &Response, status: u16, name: Option<&str>) {
    assert_eq!(response.status, status, "{}", text(response));
    assert!(
        response
            .content_type
            .starts_with("application/problem+json"),
        "a problem, not {:?}: {}",
        response.content_type,
        text(response)
    );
    if let Some(name) = name {
        assert_eq!(json(response)["type"], problem(name), "{}", text(response));
    }
}

/// The status and problem type of `response`, a refusal.
pub fn refusal_of(response: &Response) -> (u16, Value) {
    (response.status, json(response)["type"].clone())
}

/// A device's requests to one instance of the server, on a session of its account.
pub struct Https {
    base: String,
    token: String,
    device: DeviceId,
    /// The secret of the device's signing key.
    secret: [u8; 32],
    /// The server's public address, which proofs sign.
    public: String,
    version: NonZeroU64,
}

impl Https {
    /// The requests of `person`'s certified laptop to `flow`'s server.
    pub fn of(flow: &DeviceFlow, person: &Person) -> Self {
        Self::to(&flow.stand.server, flow, person, &person.laptop)
    }

    /// The requests of `device` of `person` to `server`, an instance of `flow`'s server, on
    /// `person`'s session.
    pub fn to(server: &Server, flow: &DeviceFlow, person: &Person, device: &Device) -> Self {
        Self::on_session(server, flow, person.session.token(), device)
    }

    /// The requests of `device` to `server`, an instance of `flow`'s server, on the session
    /// `token`.
    pub fn on_session(server: &Server, flow: &DeviceFlow, token: &str, device: &Device) -> Self {
        Https {
            base: server.base().to_owned(),
            token: token.to_owned(),
            device: device.id,
            secret: device.signing.secret,
            public: flow.stand.public.clone(),
            version: PROTOCOL_VERSION,
        }
    }

    /// These requests, each stating `version`.
    pub fn stating(mut self, version: NonZeroU64) -> Self {
        self.version = version;
        self
    }

    /// The server's public address, which proofs sign.
    pub fn public(&self) -> &str {
        &self.public
    }

    /// The device of these requests.
    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// The device's signing key.
    pub fn key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.secret)
    }

    /// A new challenge of the session.
    #[track_caller]
    pub fn challenge(&self) -> Vec<u8> {
        let version = self.version.to_string();
        let bearer = format!("Bearer {}", self.token);
        let headers = [
            (VERSION_HEADER, version.as_str()),
            ("authorization", bearer.as_str()),
        ];
        let url = format!("{}{CHALLENGE}", self.base);
        let response = send("post", &url, &headers, &[]);
        let violations = operation_violations(the_spec(), "post", CHALLENGE, &response);
        assert_eq!(violations, Vec::<String>::new(), "{}", text(&response));
        assert_eq!(response.status, 200, "a challenge: {}", text(&response));
        let body = json(&response);
        unb64(
            body["challenge"]
                .as_str()
                .unwrap_or_else(|| panic!("no challenge in {body}")),
        )
    }

    /// The device's proof on a new challenge, as its client makes it.
    #[track_caller]
    pub fn proof(&self) -> String {
        self.proof_on(&self.challenge())
    }

    /// The device's proof on `challenge`, as its client makes it.
    pub fn proof_on(&self, challenge: &[u8]) -> String {
        proof(challenge, self.device, &self.key(), &self.public)
    }

    /// `POST <path>` of `body` with the session, stating the version if `stated`, with `proof` if
    /// given; `operation` is the spec's path.
    pub fn request(
        &self,
        operation: &'static str,
        path: &str,
        body: Vec<u8>,
        proof: Option<String>,
        stated: bool,
    ) -> Request {
        let mut headers = vec![
            ("authorization".to_owned(), format!("Bearer {}", self.token)),
            ("content-type".to_owned(), CBOR.to_owned()),
        ];
        if stated {
            headers.push((VERSION_HEADER.to_owned(), self.version.to_string()));
        }
        if let Some(proof) = proof {
            headers.push((PROOF.to_owned(), proof));
        }
        Request {
            path: operation,
            url: format!("{}{path}", self.base),
            headers,
            body,
        }
    }

    /// The `send` of `envelope` with `report` and `proof`.
    pub fn send_request(&self, envelope: &[u8], report: bool, proof: Option<String>) -> Request {
        self.request(SEND, SEND, send_frame(envelope, report), proof, true)
    }

    /// The long-poll from `cursor` waiting up to `wait` seconds, with `proof`.
    pub fn poll_request(&self, cursor: u64, wait: u64, proof: Option<String>) -> Request {
        self.request(POLL, &poll_path(wait), sync_frame(cursor), proof, true)
    }

    /// The `ack` of `seq` with `proof`.
    pub fn ack_request(&self, seq: u64, proof: Option<String>) -> Request {
        self.request(ACK, ACK, ack_frame(seq), proof, true)
    }

    /// Sends `envelope` with `report`, proved; gives the answer.
    #[track_caller]
    pub fn send(&self, envelope: &[u8], report: bool) -> Answer {
        answer_of(
            &self
                .send_request(envelope, report, Some(self.proof()))
                .call(),
        )
    }

    /// Sends `envelope` of `header` with `report` true and checks that the server stores it:
    /// `stored` for its send-key, at about the time of the send; gives the `stored`.
    #[track_caller]
    pub fn store(&self, header: &Header, envelope: &[u8]) -> Stored {
        let before = now();
        let answer = self.send(envelope, true);
        let Answer::Stored(stored) = answer else {
            panic!("stored, not {answer:?}");
        };
        assert_eq!(stored.send_key, key_of(header));
        assert_stored_at(&stored, before);
        stored
    }

    /// The long-poll from `cursor` waiting up to `wait` seconds, proved; gives its `batch`.
    #[track_caller]
    pub fn poll(&self, cursor: u64, wait: u64) -> Batch {
        batch_of(&self.poll_request(cursor, wait, Some(self.proof())).call())
    }

    /// The long-poll from `cursor` with a wait of 0; gives its `batch`. Only the tests of answers at
    /// once time it, with [`Https::poll_at_once`]: on a loaded machine any request may take longer
    /// than [`AT_ONCE`].
    #[track_caller]
    pub fn poll_now(&self, cursor: u64) -> Batch {
        batch_of(&self.poll_request(cursor, 0, Some(self.proof())).call())
    }

    /// The long-poll from `cursor` with a wait of 0, which answers within [`AT_ONCE`]; gives its
    /// `batch`.
    #[track_caller]
    pub fn poll_at_once(&self, cursor: u64) -> Batch {
        let request = self.poll_request(cursor, 0, Some(self.proof()));
        let sent = Instant::now();
        let response = request.call();
        let took = sent.elapsed();
        assert!(took < AT_ONCE, "a wait of 0 answered after {took:?}");
        batch_of(&response)
    }

    /// Starts the long-poll from `cursor` waiting up to `wait` seconds, proved.
    #[track_caller]
    pub fn start_poll(&self, cursor: u64, wait: u64) -> Waiting {
        self.poll_request(cursor, wait, Some(self.proof())).start()
    }

    /// Acks up to `seq`, proved; the server takes it with `204`.
    #[track_caller]
    pub fn ack(&self, seq: u64) {
        let response = self.ack_request(seq, Some(self.proof())).call();
        assert_eq!(response.status, 204, "{}", text(&response));
    }
}

/// The `batch` that `waiting` answers within `within` of `since`.
#[track_caller]
pub fn answered_within(waiting: Waiting, since: Instant, within: Duration) -> Batch {
    let (response, at) = waiting.answer();
    let took = at.saturating_duration_since(since);
    assert!(
        took <= within,
        "answered {took:?} after, not within {within:?}"
    );
    batch_of(&response)
}

impl Https {
    /// The entries of the first long-poll from `cursor` that answers some, within `within`; the
    /// long-polls that answer only notices are skipped.
    #[track_caller]
    pub fn entries_within(&self, cursor: u64, within: Duration) -> Vec<Entry> {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now()).as_secs();
            let batch = self.poll(cursor, left.clamp(1, LONG));
            if !batch.entries.is_empty() {
                return batch.entries;
            }
            assert!(
                Instant::now() < deadline,
                "no entry after {cursor} within {within:?}"
            );
        }
    }
}

/// Stores `count` envelopes to `recipient` in `fixture`'s channel through `sender`, with `report`
/// true.
#[track_caller]
pub fn stored_through(
    sender: &Https,
    fixture: &Fixture,
    recipient: DeviceId,
    count: usize,
) -> Vec<Sent> {
    (0..count)
        .map(|_| {
            let header = fixture.to(recipient);
            let sent = envelope(&header);
            sender.store(&header, &sent);
            (header, sent)
        })
        .collect()
}
