//! A device's mailbox stream over server-sent events: opening it, and what comes on it, read in a
//! thread of its own so that a test can wait for each event with a deadline.

use std::io::{BufRead, BufReader, Read};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::{Cbor, DeviceId, Frame, PROTOCOL_VERSION, Push, SpaceId, VERSION_HEADER};

use crate::api::Person;
use crate::common::agent::{agent, get};
use crate::common::device_flow::{DeviceFlow, text};
use crate::common::devices::Device;
use crate::common::spec::{operation_violations, spec};
use crate::common::{Response, Server};
use crate::fixture::Fixture;
use crate::https::{Https, PROOF};
use crate::link::{Entry, Link, Sent, WAIT, store_to};

/// The stream: `GET`, the cursor in `Last-Event-ID`.
pub const EVENTS: &str = "/mailbox/events";
/// The media type of the stream.
pub const EVENT_STREAM: &str = "text/event-stream";
/// The request header that states the cursor.
pub const LAST_EVENT_ID: &str = "last-event-id";
/// The server setting of the keep-alive interval, in seconds.
pub const KEEP_ALIVE: &str = "--sse-keep-alive";

/// What comes on a stream, in the order it comes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A `message` or `status`, in an event whose `id` is its `seq`.
    Entry(Entry),
    /// The empty `batch`: the stream has sent every entry the mailbox held at its opening.
    End,
    /// A `space-notice` of the space.
    Notice(SpaceId),
    /// A comment line.
    KeepAlive,
    /// The server ended the response, or its connection.
    Closed,
    /// An event that is not one frame of those a stream sends.
    Bad(String),
}

/// The spec, read once.
fn the_spec() -> &'static Value {
    static SPEC: OnceLock<Value> = OnceLock::new();
    SPEC.get_or_init(spec)
}

/// Checks that `response` to the opening of a stream conforms to the spec; an open stream's body
/// is not read here.
#[track_caller]
fn conforms(response: &Response) {
    let violations = operation_violations(the_spec(), "get", EVENTS, response);
    assert_eq!(violations, Vec::<String>::new(), "{}", text(response));
}

/// The streams of a device on one instance of the server, on a session of its account; its
/// requests over HTTPS on that instance and session prove it.
pub struct Sse {
    https: Https,
    url: String,
    token: String,
}

impl Sse {
    /// The streams of `person`'s certified laptop on `flow`'s server.
    pub fn of(flow: &DeviceFlow, person: &Person) -> Self {
        Self::to(&flow.stand.server, flow, person, &person.laptop)
    }

    /// The streams of `device` of `person` on `server`, an instance of `flow`'s server, on
    /// `person`'s session.
    pub fn to(server: &Server, flow: &DeviceFlow, person: &Person, device: &Device) -> Self {
        Self::on_session(server, flow, person.session.token(), device)
    }

    /// The streams of `device` on `server`, an instance of `flow`'s server, on the session `token`.
    pub fn on_session(server: &Server, flow: &DeviceFlow, token: &str, device: &Device) -> Self {
        Sse {
            https: Https::on_session(server, flow, token, device),
            url: format!("{}{EVENTS}", server.base()),
            token: token.to_owned(),
        }
    }

    /// The device's requests over HTTPS on the same instance and session.
    pub fn https(&self) -> &Https {
        &self.https
    }

    /// Opens a stream from `cursor`, in `Last-Event-ID` if given, stating the version if `stated`,
    /// with `proof` if given: the stream, or the response that refuses it.
    pub fn request(
        &self,
        cursor: Option<u64>,
        proof: Option<&str>,
        stated: bool,
    ) -> Result<Stream, Response> {
        let version = PROTOCOL_VERSION.to_string();
        let bearer = format!("Bearer {}", self.token);
        let cursor = cursor.map(|cursor| cursor.to_string());
        let response = get(|| {
            let mut request = agent()
                .get(&self.url)
                .header("accept", EVENT_STREAM)
                .header("authorization", &bearer);
            if stated {
                request = request.header(VERSION_HEADER, &version);
            }
            if let Some(cursor) = &cursor {
                request = request.header(LAST_EVENT_ID, cursor);
            }
            if let Some(proof) = proof {
                request = request.header(PROOF, proof);
            }
            request.call()
        })
        .unwrap_or_else(|err| panic!("GET {}: {err}", self.url));
        let (parts, body) = response.into_parts();
        let mut head = Response {
            status: parts.status.as_u16(),
            content_type: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
        };
        for (name, value) in &parts.headers {
            let value = value.to_str().expect("header values are ASCII").to_owned();
            if name == "content-type" {
                head.content_type = value;
            } else {
                head.headers.push((name.as_str().to_owned(), value));
            }
        }
        if head.status == 200 && head.content_type.starts_with(EVENT_STREAM) {
            conforms(&head);
            return Ok(Stream::reading(body.into_reader()));
        }
        let mut body = body;
        head.body = body.read_to_vec().expect("read body");
        conforms(&head);
        Err(head)
    }

    /// Opens a stream from `cursor` with the device's proof, as its client does.
    #[track_caller]
    pub fn open(&self, cursor: Option<u64>) -> Stream {
        self.open_on(cursor, &self.https.proof())
    }

    /// Opens a stream from `cursor` with `proof`, which the server takes.
    #[track_caller]
    pub fn open_on(&self, cursor: Option<u64>, proof: &str) -> Stream {
        self.request(cursor, Some(proof), true)
            .unwrap_or_else(|refused| {
                panic!("a stream, not {}: {}", refused.status, text(&refused))
            })
    }

    /// The response that refuses a stream from `cursor` with `proof`.
    #[track_caller]
    pub fn refused(&self, cursor: Option<u64>, proof: Option<&str>) -> Response {
        match self.request(cursor, proof, true) {
            Ok(_) => panic!("a stream opened"),
            Err(response) => response,
        }
    }
}

/// An open stream: the events read so far but not taken, each with when it came.
pub struct Stream {
    events: Receiver<(Event, Instant)>,
    closed: bool,
}

impl Stream {
    fn reading(body: impl Read + Send + 'static) -> Self {
        let (sender, events) = mpsc::channel();
        thread::spawn(move || read_events(BufReader::new(body), &sender));
        Stream {
            events,
            closed: false,
        }
    }

    /// The next event within `within`, with when it came; `None` if none came.
    #[track_caller]
    pub fn next(&mut self, within: Duration) -> Option<(Event, Instant)> {
        assert!(!self.closed, "the stream is closed");
        match self.events.recv_timeout(within) {
            Ok((Event::Bad(bad), _)) => panic!("{bad}"),
            Ok((event, at)) => {
                self.closed = event == Event::Closed;
                Some((event, at))
            }
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => panic!("the stream's reader ended"),
        }
    }

    /// The next event but keep-alives that comes before `deadline`; `None` if none does.
    #[track_caller]
    fn data_until(&mut self, deadline: Instant) -> Option<Event> {
        loop {
            match self.next(deadline.saturating_duration_since(Instant::now()))? {
                (Event::KeepAlive, _) => {}
                (event, _) => return Some(event),
            }
        }
    }

    /// The next event within `within` but keep-alives.
    #[track_caller]
    pub fn data_within(&mut self, within: Duration) -> Event {
        self.data_until(Instant::now() + within)
            .unwrap_or_else(|| panic!("nothing but keep-alives within {within:?}"))
    }

    /// The entries of catch-up, up to the empty `batch` that ends it.
    #[track_caller]
    pub fn catch_up(&mut self) -> Vec<Entry> {
        let mut entries = Vec::new();
        loop {
            match self.data_within(WAIT) {
                Event::Entry(entry) => entries.push(entry),
                Event::End => return entries,
                other => {
                    panic!("an entry or the end of catch-up, not {other:?}, after {entries:?}")
                }
            }
        }
    }

    /// The next entry, which comes within `within`.
    #[track_caller]
    pub fn entry_within(&mut self, within: Duration) -> Entry {
        match self.data_within(within) {
            Event::Entry(entry) => entry,
            other => panic!("an entry, not {other:?}"),
        }
    }

    /// Checks that nothing but keep-alives comes for `quiet`, and that the stream stays open.
    #[track_caller]
    pub fn assert_quiet(&mut self, quiet: Duration) {
        if let Some(event) = self.data_until(Instant::now() + quiet) {
            panic!("{event:?} on a quiet stream");
        }
    }

    /// Checks that the stream closes within `within`, after nothing but keep-alives.
    #[track_caller]
    pub fn assert_closes_within(&mut self, within: Duration) {
        assert_eq!(self.data_within(within), Event::Closed);
    }
}

/// Reads the events of `body` to `sender` until the stream closes or no one reads them.
fn read_events(mut body: impl BufRead, sender: &Sender<(Event, Instant)>) {
    let mut id = None;
    let mut data: Option<String> = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        let event = match body.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => Some(Event::Closed),
            Ok(_) => {
                let text = String::from_utf8_lossy(&line);
                let text = text.strip_suffix('\n').unwrap_or(&text);
                let text = text.strip_suffix('\r').unwrap_or(text);
                if text.is_empty() {
                    data.take().map(|data| event_of(id.take(), &data))
                } else if text.starts_with(':') {
                    Some(Event::KeepAlive)
                } else {
                    let (field, value) = text.split_once(':').unwrap_or((text, ""));
                    let value = value.strip_prefix(' ').unwrap_or(value).to_owned();
                    match field {
                        "id" => id = Some(value),
                        "data" => match &mut data {
                            Some(data) => {
                                data.push('\n');
                                data.push_str(&value);
                            }
                            None => data = Some(value),
                        },
                        _ => {}
                    }
                    None
                }
            }
        };
        if let Some(event) = event {
            let closed = event == Event::Closed;
            if sender.send((event, Instant::now())).is_err() || closed {
                return;
            }
        }
    }
}

/// The event of `data` with `id`.
fn event_of(id: Option<String>, data: &str) -> Event {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let Ok(bytes) = URL_SAFE_NO_PAD.decode(data) else {
        return Event::Bad(format!(
            "data that is not base64url without padding: {data:?}"
        ));
    };
    let Ok(frame) = Frame::from_cbor(&bytes) else {
        return Event::Bad(format!("data that is not a frame: {data:?}"));
    };
    match frame {
        Frame::Batch(pushes) if pushes.is_empty() => Event::End,
        Frame::Push(Push::SpaceNotice { space }) => Event::Notice(space),
        Frame::Push(push) => match Entry::of(push) {
            Some(entry) if id == Some(entry.seq().to_string()) => Event::Entry(entry),
            Some(entry) => Event::Bad(format!("{entry:?} in an event whose id is {id:?}")),
            None => Event::Bad(format!("a push a server does not send: {data:?}")),
        },
        other => Event::Bad(format!(
            "one message, status or space-notice per event, or an empty batch, not {other:?}"
        )),
    }
}

/// Stores `count` envelopes from alice's laptop to `recipient` in `f`'s channel over a WebSocket,
/// with `report` true.
#[track_caller]
pub fn sent_by_alice(f: &Fixture, recipient: DeviceId, count: usize) -> Vec<Sent> {
    let mut alice = Link::of(&f.team.flow, &f.team.alice);
    store_to(&mut alice, f, recipient, count, true)
}

/// Stores an envelope from alice's laptop to bob's in `f`'s channel; gives it.
#[track_caller]
pub fn sent_to_bobs_laptop(f: &Fixture) -> Vec<u8> {
    let (_, sent) = sent_by_alice(f, f.team.bob.laptop.id, 1).remove(0);
    sent
}
