//! What the tests share: alice's and bob's accounts, a new device that has no session, the run it
//! opens and the requests of its side.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use skald_github_stand_in::User;

use crate::api::{Person, Session, assert_refused};
use crate::common::Response;
use crate::common::device_flow::{DeviceFlow, INTERVAL, json, text};
use crate::common::devices::{DEVICES, Device, assert_taken, b64, machine};
use crate::relay::{WAIT, fresh_session, hello, messages_of, reply, request};

/// Opens a run without a session: `POST`.
pub const OPEN: &str = "/links";
/// The run's later messages: `GET`; a later message of the run: `POST`. With the run's token.
pub const RUN_MESSAGES: &str = "/links/messages";
/// Pairs a run with the session's account: `POST`.
pub const PAIRING: &str = "/account/links/pairing";

/// A server of its own with alice's account, her laptop certified by her master key, and bob's.
pub struct Fresh {
    pub flow: DeviceFlow,
    pub alice: Person,
    pub bob: Person,
    /// The GitHub user ID of alice; bob's is the next.
    github: u64,
}

impl Fresh {
    /// The accounts on a server whose GitHub user IDs start at `github`.
    pub fn start(github: u64) -> Self {
        Self::with(github, &[])
    }

    /// As [`Fresh::start`], on a server with the extra arguments `args`.
    pub fn with(github: u64, args: &[&str]) -> Self {
        let flow = DeviceFlow::start_with(&[&[INTERVAL, "1"], args].concat());
        let alice = Person::new(&flow, github, "alice");
        let bob = Person::new(&flow, github + 1, "bob");
        Fresh {
            flow,
            alice,
            bob,
            github,
        }
    }

    /// Requests to the server without a session.
    pub fn nobody(&self) -> Session {
        Session::none(&self.flow)
    }

    /// `POST /links` of `hello`'s bytes naming `device`, without a session.
    pub fn send_open(&self, device: &Device, hello: &[u8]) -> Response {
        let body = json!({ "device": device.id.to_string(), "hello": b64(hello) });
        self.nobody().call("post", OPEN, Some(&body))
    }

    /// Opens a run of `device`, which has no session, with a fresh `session`; the server must take
    /// it.
    #[track_caller]
    pub fn open(&self, device: &Device) -> Run {
        let session = fresh_session();
        let hello = hello(&session);
        let opened = self.send_open(device, &hello);
        assert_eq!(opened.status, 201, "the run opens: {}", text(&opened));
        let body = json(&opened);
        let field = |name: &str| {
            body[name]
                .as_str()
                .unwrap_or_else(|| panic!("no {name} in {body}"))
                .to_owned()
        };
        Run {
            session,
            hello,
            pairing: field("pairing"),
            side: Session::of_token(&self.flow, &field("token")),
        }
    }

    /// alice's GitHub user.
    pub fn alice_user(&self) -> User {
        self.flow.stand.user(self.github, "alice-spaces-standin")
    }

    /// Signs a machine in to alice's account by the device flow, as her new device's machine does
    /// with the browser; gives its session.
    pub fn alice_signs_in(&self) -> Session {
        Session::of_token(&self.flow, &machine(&self.flow, &self.alice_user()).token)
    }
}

/// `POST /account/links/pairing` of `pairing` from `session`.
pub fn send_pairing(session: &Session, pairing: &str) -> Response {
    session.call("post", PAIRING, Some(&json!({ "pairing": pairing })))
}

/// The hello `session` gets for `pairing`, which pairs the run with its account; the server must
/// give it.
#[track_caller]
pub fn pair(session: &Session, pairing: &str) -> Vec<u8> {
    let paired = send_pairing(session, pairing);
    assert_eq!(paired.status, 200, "paired: {}", text(&paired));
    let body = json(&paired);
    let hello = body["hello"]
        .as_str()
        .unwrap_or_else(|| panic!("no hello in {body}"));
    crate::common::devices::unb64(hello)
}

/// Registers `device` on the account of `session`, the device's own machine signed in.
#[track_caller]
pub fn register(session: &Session, device: &Device) {
    assert_taken(&session.call("post", DEVICES, Some(&device.registration())));
}

/// A run of a new device without a session, as its side holds it.
pub struct Run {
    /// The run's `session`.
    pub session: [u8; 16],
    /// The hello that opened it.
    pub hello: Vec<u8>,
    /// What the person enters on the linked device.
    pub pairing: String,
    /// Requests with the run's token.
    pub side: Session,
}

impl Run {
    /// The same run's side, its requests to `server`, another instance on the same database.
    pub fn on(&self, server: &crate::common::Server) -> Run {
        Run {
            session: self.session,
            hello: self.hello.clone(),
            pairing: self.pairing.clone(),
            side: self.side.on(server),
        }
    }

    /// `GET /links/messages` with the run's token.
    pub fn messages_response(&self) -> Response {
        self.side.call("get", RUN_MESSAGES, None)
    }

    /// The run's later messages; the server must give them.
    #[track_caller]
    pub fn messages(&self) -> Vec<Vec<u8>> {
        let response = self.messages_response();
        assert_eq!(response.status, 200, "{}", text(&response));
        messages_of(&response)
    }

    /// The run's later messages once they hold `message`, which they must within [`WAIT`].
    #[track_caller]
    pub fn messages_with(&self, message: &[u8]) -> Vec<Vec<u8>> {
        within(&format!("the run's messages hold {message:?}"), || {
            let messages = self.messages();
            messages
                .iter()
                .any(|got| got == message)
                .then_some(messages)
        })
    }

    /// `POST /links/messages` of `message`'s bytes with the run's token.
    pub fn send(&self, message: &[u8]) -> Response {
        let body = json!({ "message": b64(message) });
        self.side.call("post", RUN_MESSAGES, Some(&body))
    }

    /// Waits until the server refuses the run's messages to its side, which it must within
    /// [`WAIT`]: the run has ended.
    #[track_caller]
    pub fn until_ended(&self, what: &str) {
        let refused = within(&format!("{what}: the run's messages are refused"), || {
            Some(self.messages_response()).filter(|response| response.status != 200)
        });
        assert_refused(&refused, what);
    }
}

/// Waits until the server refuses `message` from `session`, an issuing side, which it must within
/// [`WAIT`]: the run has ended for it. Nothing it refused is relayed.
#[track_caller]
pub fn until_refused(session: &Session, message: &[u8], what: &str) {
    let refused = within(&format!("{what}: refused"), || {
        Some(session.send_message(message)).filter(|response| response.status != 201)
    });
    crate::relay::assert_refused_link(&refused, what);
}

/// What `check` gives once it gives something, which it must within [`WAIT`]; `what` names it.
#[track_caller]
pub fn within<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(Instant::now() < deadline, "{what}: not within {WAIT:?}");
        thread::sleep(Duration::from_millis(100));
    }
}

/// The issuing side `issuer` replies on `run` and the new device sends its request; each side gets
/// the other's message. Gives the reply and the request.
#[track_caller]
pub fn reply_and_request(issuer: &Session, run: &Run) -> (Vec<u8>, Vec<u8>) {
    let reply = reply(&run.session);
    assert_taken(&issuer.send_message(&reply));
    run.messages_with(&reply);
    let request = request(&run.session);
    assert_taken(&run.send(&request));
    issuer.messages_with(&run.session, &request);
    (reply, request)
}
