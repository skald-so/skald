//! What the tests share: alice's account with a second session, that of the machine of her new
//! device, bob's account, the link messages the tests write byte by byte, and the requests that
//! send and get them.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_proto::{DeviceId, DeviceKind};

use crate::api::{Person, Session, assert_refused};
use crate::common::cbor::{array_of, byte_item, uint};
use crate::common::device_flow::{DeviceFlow, INTERVAL, json, text};
use crate::common::devices::{Device, account_id, assert_taken, b64, machine, random, unb64};
use crate::common::{Response, hex};

/// Opens a run with a hello: `POST`.
pub const LINKS: &str = "/account/links";
/// The account's latest hello: `GET`.
pub const HELLO: &str = "/account/links/hello";
/// A run's later message: `POST`; with `/<session>`, the run's later messages: `GET`.
pub const MESSAGES: &str = "/account/links/messages";
/// The account's devices with an open run: `GET`.
pub const WAITING: &str = "/account/links/waiting";
/// The setting of how long after its hello a run ends, in seconds.
pub const LIFETIME: &str = "--link-code-lifetime";
/// How long a test waits for the server to relay a message or end a run.
pub const WAIT: Duration = Duration::from_secs(20);

/// The accounts of the tests on a server of their own: alice, with her laptop certified and her
/// desktop registered without a certificate, her new device, whose machine has a session of its
/// own; and bob, with the same devices.
pub struct Link {
    pub flow: DeviceFlow,
    pub alice: Person,
    /// The session of the machine of alice's desktop, the new device.
    pub new_side: Session,
    pub bob: Person,
}

impl Link {
    /// The accounts on a server whose GitHub user IDs start at `github`.
    pub fn start(github: u64) -> Self {
        Self::with(github, &[])
    }

    /// As [`Link::start`], on a server with the extra arguments `args`.
    pub fn with(github: u64, args: &[&str]) -> Self {
        let flow = DeviceFlow::start_with(&[&[INTERVAL, "1"], args].concat());
        let (alice, bob) = (
            Person::new(&flow, github, "alice"),
            Person::new(&flow, github + 1, "bob"),
        );
        // The new device's machine signs in to alice's account on its own.
        let user = flow.stand.user(github, "alice-spaces-standin");
        let token = {
            let signed_in = machine(&flow, &user);
            assert_eq!(
                account_id(&signed_in.account),
                alice.account,
                "alice's account"
            );
            signed_in.token.clone()
        };
        let new_side = Session::of_token(&flow, &token);
        Link {
            flow,
            alice,
            new_side,
            bob,
        }
    }

    /// alice's desktop, the new device.
    pub fn desktop(&self) -> DeviceId {
        self.alice.uncertified.id
    }

    /// Opens a run of alice's desktop from the new device's session with a fresh `session`; gives
    /// the run's session and hello.
    #[track_caller]
    pub fn open(&self) -> ([u8; 16], Vec<u8>) {
        let session = fresh_session();
        let hello = hello(&session);
        assert_taken(&self.new_side.send_hello(self.desktop(), &hello));
        (session, hello)
    }

    /// Registers a new `client` device named `name` on alice's account from the new device's
    /// session, without a certificate; gives it.
    pub fn register(&self, name: &str) -> Device {
        let device = Device::new(name);
        let registered = self.new_side.call(
            "post",
            crate::common::devices::DEVICES,
            Some(&device.registration()),
        );
        assert_taken(&registered);
        device
    }

    /// Writes to the database a certificate of alice's desktop, issued by her master key.
    pub fn certify_desktop(&self) {
        let alice = &self.alice;
        let cert = alice
            .uncertified
            .cert_as(alice.account, DeviceKind::Client, &alice.master);
        self.flow.stand.db.execute(&format!(
            "UPDATE devices SET cert = '\\x{}'::bytea WHERE id = '{}'::uuid",
            hex(&cert),
            alice.uncertified.id
        ));
    }
}

/// A fresh random `session`.
pub fn fresh_session() -> [u8; 16] {
    random()[..16].try_into().unwrap()
}

/// A share: 32 random bytes, which the server relays as they are.
fn share() -> [u8; 32] {
    random()
}

/// `link-hello = [1, session, share]`.
pub fn hello(session: &[u8; 16]) -> Vec<u8> {
    array_of(&[uint(1), byte_item(session), byte_item(&share())])
}

/// `link-reply = [2, session, share]`.
pub fn reply(session: &[u8; 16]) -> Vec<u8> {
    array_of(&[uint(2), byte_item(session), byte_item(&share())])
}

/// `link-request = [3, session, sealed]`, with random bytes sealed.
pub fn request(session: &[u8; 16]) -> Vec<u8> {
    array_of(&[uint(3), byte_item(session), byte_item(&random())])
}

/// `link-grant = [4, session, sealed]`, with random bytes sealed.
pub fn grant(session: &[u8; 16]) -> Vec<u8> {
    let sealed = [random(), random(), random()].concat();
    array_of(&[uint(4), byte_item(session), byte_item(&sealed)])
}

/// `link-abort = [5, session]`.
pub fn abort(session: &[u8; 16]) -> Vec<u8> {
    array_of(&[uint(5), byte_item(session)])
}

/// A device with an open run, as the account's sessions get it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Waiting {
    pub device: String,
    pub kind: String,
    pub name: String,
}

impl Waiting {
    /// The `client` device `device`, as it registered.
    pub fn of(device: &Device) -> Self {
        Waiting {
            device: device.id.to_string(),
            kind: "client".to_owned(),
            name: device.name.clone(),
        }
    }
}

impl Session {
    /// `POST /account/links` of `hello`'s bytes naming `device`.
    pub fn send_hello(&self, device: DeviceId, hello: &[u8]) -> Response {
        self.send_hello_body(&json!({ "device": device.to_string(), "hello": b64(hello) }))
    }

    /// `POST /account/links` of `body`.
    pub fn send_hello_body(&self, body: &Value) -> Response {
        self.call("post", LINKS, Some(body))
    }

    /// The hello `GET /account/links/hello` gives, if any.
    pub fn hello(&self) -> Option<Vec<u8>> {
        let response = self.call("get", HELLO, None);
        assert_eq!(response.status, 200, "{}", text(&response));
        let body = json(&response);
        match &body["hello"] {
            Value::Null => None,
            Value::String(hello) => Some(unb64(hello)),
            _ => panic!("no hello in {body}"),
        }
    }

    /// `POST /account/links/messages` of `message`'s bytes.
    pub fn send_message(&self, message: &[u8]) -> Response {
        self.send_message_body(&json!({ "message": b64(message) }))
    }

    /// `POST /account/links/messages` of `body`.
    pub fn send_message_body(&self, body: &Value) -> Response {
        self.call("post", MESSAGES, Some(body))
    }

    /// `GET /account/links/messages/<session>`.
    pub fn messages_response(&self, session: &[u8; 16]) -> Response {
        self.call("get", &format!("{MESSAGES}/{}", b64(session)), None)
    }

    /// The later messages of the run `session` this session gets, in order.
    pub fn messages(&self, session: &[u8; 16]) -> Vec<Vec<u8>> {
        let response = self.messages_response(session);
        assert_eq!(response.status, 200, "{}", text(&response));
        messages_of(&response)
    }

    /// The later messages of the run `session` once they hold `message`, which they must within
    /// [`WAIT`].
    #[track_caller]
    pub fn messages_with(&self, session: &[u8; 16], message: &[u8]) -> Vec<Vec<u8>> {
        let deadline = Instant::now() + WAIT;
        loop {
            let messages = self.messages(session);
            if messages.iter().any(|got| got == message) {
                return messages;
            }
            assert!(
                Instant::now() < deadline,
                "the run's messages {messages:?} lack {message:?} after {WAIT:?}"
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// The devices `GET /account/links/waiting` gives, sorted.
    pub fn waiting(&self) -> Vec<Waiting> {
        let response = self.call("get", WAITING, None);
        assert_eq!(response.status, 200, "{}", text(&response));
        let body = json(&response);
        let mut devices: Vec<Waiting> = body["devices"]
            .as_array()
            .unwrap_or_else(|| panic!("no devices in {body}"))
            .iter()
            .map(|device| Waiting {
                device: device["device"].as_str().expect("a device ID").to_owned(),
                kind: device["kind"].as_str().expect("a kind").to_owned(),
                name: device["name"].as_str().expect("a name").to_owned(),
            })
            .collect();
        devices.sort();
        devices
    }

    /// Waits until the waiting devices this session gets no longer name `device`, which they must
    /// within [`WAIT`].
    #[track_caller]
    pub fn until_not_waiting(&self, device: DeviceId) {
        let deadline = Instant::now() + WAIT;
        let id = device.to_string();
        loop {
            let waiting = self.waiting();
            if !waiting.iter().any(|entry| entry.device == id) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{id} still waits after {WAIT:?}: {waiting:?}"
            );
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The messages in `response`, a `GET /account/links/messages/<session>` answer.
pub fn messages_of(response: &Response) -> Vec<Vec<u8>> {
    let body = json(response);
    body["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("no messages in {body}"))
        .iter()
        .map(|message| unb64(message.as_str().expect("a message's bytes")))
        .collect()
}

/// Checks that `response` refuses what it answers: a `4xx` other than `401`, with a problem
/// details object; `what` names the case.
#[track_caller]
pub fn assert_refused_link(response: &Response, what: &str) {
    assert_refused(response, what);
    assert_ne!(
        response.status,
        401,
        "{what}: the session is valid: {}",
        text(response)
    );
}

/// The position of `message` in `messages`, which must hold it.
#[track_caller]
pub fn position(messages: &[Vec<u8>], message: &[u8]) -> usize {
    messages
        .iter()
        .position(|got| got == message)
        .unwrap_or_else(|| panic!("{messages:?} lack {message:?}"))
}
