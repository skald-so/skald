//! What the tests send and read: a device's connection and its frames, envelopes, and the
//! mailboxes and rows of the server's database.

use std::net::TcpStream;
use std::num::NonZeroU64;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use skald_proto::{
    Cbor, ChannelId, DeviceId, Envelope, Header, HpkePayload, MessageId, NonEmpty,
    PROTOCOL_VERSION, Scheme, SendKey, SpaceId, Stored, Unknown,
};
use sqlx::{Connection as _, PgConnection};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;

use crate::api::Person;
use crate::common::device_flow::{DeviceFlow, json, text};
use crate::common::devices::{Device, random, unb64};
use crate::common::{Server, TestDb, WebSocket};

/// A device session's challenge: `POST`.
pub const CHALLENGE: &str = "/session/challenge";
/// The label of the signature in `auth`.
const AUTH_LABEL: &str = "skald auth";
/// How long a test waits for an answer, or for the server to close a connection.
const WAIT: Duration = Duration::from_secs(20);

pub const VERSION_MISMATCH: &str = "version-mismatch";
pub const NOT_A_MEMBER: &str = "not-a-member";
pub const UNKNOWN_RECIPIENT: &str = "unknown-recipient";

/// The server's answer to a `send`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Stored(Stored),
    Refused(SendKey, String),
}

/// The head of a CBOR item of major type `major` whose argument is `arg`, below 2^16, in the
/// shortest form.
fn head(major: u8, arg: usize) -> Vec<u8> {
    let top = major << 5;
    if let Ok(small) = u8::try_from(arg) {
        return if small < 24 {
            vec![top | small]
        } else {
            vec![top | 24, small]
        };
    }
    let wide = u16::try_from(arg).expect("an argument below 2^16");
    let mut out = vec![top | 25];
    out.extend(wide.to_be_bytes());
    out
}

/// `bytes` as a CBOR byte string.
pub fn byte_item(bytes: &[u8]) -> Vec<u8> {
    let mut item = head(2, bytes.len());
    item.extend_from_slice(bytes);
    item
}

/// The CBOR array of the encoded `items`.
pub fn array_of(items: &[Vec<u8>]) -> Vec<u8> {
    let mut array = head(4, items.len());
    for item in items {
        array.extend_from_slice(item);
    }
    array
}

/// The CBOR unsigned integer `n`.
pub fn uint(n: usize) -> Vec<u8> {
    head(0, n)
}

/// The `send` frame of `envelope`'s bytes, with `report` true.
pub fn send_frame(envelope: &[u8]) -> Vec<u8> {
    array_of(&[uint(1), byte_item(envelope), vec![0xf5]])
}

/// The protocol version `v`.
pub fn version(v: u64) -> NonZeroU64 {
    NonZeroU64::new(v).expect("a version from 1")
}

/// An `hpke` header in `channel` of `space` for `recipient`, of this protocol version, with a new
/// `msg_id`.
pub fn header(space: SpaceId, channel: ChannelId, recipient: DeviceId) -> Header {
    Header {
        v: PROTOCOL_VERSION,
        space,
        channel,
        msg_id: MessageId::generate(),
        scheme: Scheme::Hpke,
        recipients: Some(NonEmpty::new(recipient)),
        ttl: None,
        unknown: Unknown::default(),
    }
}

/// The envelope of `header` with a payload of its own, random bytes in the form of an `hpke`
/// payload, so that no two envelopes have the same bytes.
pub fn envelope(header: &Header) -> Vec<u8> {
    let mut ct = random().to_vec();
    ct.extend_from_slice(&random());
    let payload = HpkePayload { enc: random(), ct }.to_cbor();
    Envelope {
        header: header.clone(),
        payload,
    }
    .to_cbor()
}

/// The send-key of the `hpke` envelope of `header`: its `msg_id` and its one recipient.
pub fn key_of(header: &Header) -> SendKey {
    let recipients = header.recipients.as_ref().expect("an hpke header");
    assert_eq!(recipients.len(), 1, "one recipient");
    SendKey {
        msg_id: header.msg_id,
        recipient: Some(recipients[0]),
    }
}

/// The seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}

/// A challenge of `person`'s device session from `server`, the request stating `version`.
fn challenge(server: &Server, person: &Person, version: NonZeroU64) -> Vec<u8> {
    let session = person.session.on(server);
    let response = session.call_stating(version, "post", CHALLENGE, None);
    assert_eq!(response.status, 200, "a challenge: {}", text(&response));
    let body = json(&response);
    let challenge = body["challenge"].as_str();
    unb64(challenge.unwrap_or_else(|| panic!("no challenge in {body}")))
}

/// A device's connection to the server, authenticated as the device.
pub struct Socket {
    socket: WebSocket,
}

impl Socket {
    /// The connection of `person`'s certified laptop to `flow`'s server.
    pub fn of(flow: &DeviceFlow, person: &Person) -> Self {
        Self::to(&flow.stand.server, flow, person, &person.laptop)
    }

    /// The connection of `device` of `person` to `server`, an instance of `flow`'s server, on a
    /// challenge of `person`'s session: it opens `/ws` and sends `auth`, signed over the server's
    /// public address.
    pub fn to(server: &Server, flow: &DeviceFlow, person: &Person, device: &Device) -> Self {
        Self::stating(server, flow, person, device, PROTOCOL_VERSION)
    }

    /// As [`Socket::to`], the connection of the protocol version `version`: its challenge request
    /// states it, and its opening request offers its subprotocol.
    pub fn stating(
        server: &Server,
        flow: &DeviceFlow,
        person: &Person,
        device: &Device,
        version: NonZeroU64,
    ) -> Self {
        let challenge = challenge(server, person, version);
        let offer = skald_proto::subprotocol(version);
        let (mut socket, _) = server
            .open_websocket("/ws", Some(&offer))
            .unwrap_or_else(|response| panic!("/ws refused: {}", text(&response)));
        if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
            TcpStream::set_read_timeout(stream, Some(Duration::from_millis(200)))
                .expect("a read timeout");
        }
        let origin = flow.stand.public.as_bytes();
        let signature = device.signing.key.sign(AUTH_LABEL, &[&challenge, origin]);
        let auth = array_of(&[
            uint(10),
            byte_item(&challenge),
            byte_item(device.id.as_bytes()),
            byte_item(&signature),
        ]);
        socket
            .send(Message::Binary(auth.into()))
            .expect("send auth");
        Socket { socket }
    }

    /// Sends `bytes` as one binary frame.
    pub fn frame(&mut self, bytes: Vec<u8>) {
        self.socket
            .send(Message::Binary(bytes.into()))
            .expect("send a frame");
    }

    /// Sends `envelope` in a `send`.
    pub fn send(&mut self, envelope: &[u8]) {
        self.frame(send_frame(envelope));
    }

    /// The next frame the server sends by `deadline`: `Ok(None)` if none comes by then.
    pub fn next(&mut self, deadline: Instant) -> Result<Option<Message>, tungstenite::Error> {
        use std::io::ErrorKind::{TimedOut, WouldBlock};
        loop {
            let read = self.socket.read();
            let waiting = matches!(&read, Err(tungstenite::Error::Io(err))
                if [WouldBlock, TimedOut].contains(&err.kind()));
            match (waiting, Instant::now() > deadline) {
                (false, _) => return read.map(Some),
                (true, true) => return Ok(None),
                (true, false) => {}
            }
        }
    }

    /// The next `stored` or `send-refused`; other frames, of later versions, are skipped.
    pub fn answer(&mut self) -> Answer {
        let deadline = Instant::now() + WAIT;
        loop {
            match self.next(deadline) {
                Ok(Some(Message::Binary(bytes))) => {
                    if let Some(answer) = answer_in(&bytes) {
                        return answer;
                    }
                }
                Ok(Some(Message::Close(frame))) => panic!("closed instead of an answer: {frame:?}"),
                Ok(Some(_)) => {}
                Ok(None) => panic!("no answer within {WAIT:?}"),
                Err(err) => panic!("the connection failed instead of an answer: {err}"),
            }
        }
    }

    /// The next `count` answers.
    pub fn answers(&mut self, count: usize) -> Vec<Answer> {
        (0..count).map(|_| self.answer()).collect()
    }

    /// Sends `envelope` of `header` and checks that the server stores it: `stored` for its
    /// send-key, at about the time of the send; gives the `stored`.
    #[track_caller]
    pub fn store(&mut self, header: &Header, envelope: &[u8]) -> Stored {
        let before = now();
        self.send(envelope);
        let answer = self.answer();
        let Answer::Stored(stored) = answer else {
            panic!("stored, not {answer:?}");
        };
        assert_eq!(stored.send_key, key_of(header));
        assert_stored_at(&stored, before);
        stored
    }

    /// Sends `envelope` and gives the problem it is refused with, checking that the refusal names
    /// `msg_id`.
    #[track_caller]
    pub fn refusal(&mut self, msg_id: MessageId, envelope: &[u8]) -> (SendKey, String) {
        self.send(envelope);
        let answer = self.answer();
        let Answer::Refused(key, problem) = answer else {
            panic!("send-refused, not {answer:?}");
        };
        assert_eq!(key.msg_id, msg_id, "the refusal's msg_id");
        (key, problem)
    }

    /// Checks that the server closes this connection, with a close frame or without.
    #[track_caller]
    pub fn assert_closes(&mut self) {
        let deadline = Instant::now() + WAIT;
        loop {
            match self.next(deadline) {
                Ok(Some(Message::Close(_))) | Err(_) => return,
                Ok(Some(_)) => {}
                Ok(None) => panic!("still open after {WAIT:?}"),
            }
        }
    }
}

/// Checks that `stored` was committed at `before` or later, by the clock of this machine, and not
/// after now.
#[track_caller]
pub fn assert_stored_at(stored: &Stored, before: u64) {
    let range = before.saturating_sub(2)..=now() + 2;
    assert!(
        range.contains(&stored.timestamp),
        "stored at {}, not in {range:?}",
        stored.timestamp
    );
}

/// The answer in the frame `bytes`; `None` for a frame of another type.
fn answer_in(bytes: &[u8]) -> Option<Answer> {
    match bytes {
        [0x83, 0x02, ..] => Some(Answer::Stored(
            Stored::from_cbor(bytes).unwrap_or_else(|err| panic!("a stored that decodes: {err}")),
        )),
        [0x83, 0x03, rest @ ..] => {
            // A send-key of one or two IDs, each 17 bytes encoded, then the problem.
            let key_len = match rest.first() {
                Some(0x81) => 18,
                Some(0x82) => 35,
                other => panic!("a send-key in send-refused, not {other:?}: {bytes:02x?}"),
            };
            let (key, problem) = rest.split_at(key_len);
            let key = SendKey::from_cbor(key)
                .unwrap_or_else(|err| panic!("a send-key that decodes: {err}"));
            let text = match problem {
                [len @ 0x60..=0x77, text @ ..] if text.len() == usize::from(len - 0x60) => text,
                [0x78, len, text @ ..] if text.len() == usize::from(*len) => text,
                _ => panic!("a problem in send-refused: {bytes:02x?}"),
            };
            let problem = String::from_utf8(text.to_vec()).expect("a UTF-8 problem");
            Some(Answer::Refused(key, problem))
        }
        _ => None,
    }
}

/// Runs `work` on a connection to `db`.
pub fn query<T>(
    db: &TestDb,
    work: impl AsyncFnOnce(&mut PgConnection) -> Result<T, sqlx::Error>,
) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime")
        .block_on(async {
            let mut conn = PgConnection::connect(&db.url()).await?;
            let found = work(&mut conn).await?;
            conn.close().await?;
            Ok::<_, sqlx::Error>(found)
        })
        .unwrap_or_else(|err| panic!("read the mailboxes: {err}"))
}

/// The entries of `device`'s mailbox in `seq` order: each `seq` and its envelope's bytes.
pub fn mailbox(db: &TestDb, device: DeviceId) -> Vec<(i64, Vec<u8>)> {
    let device = device.to_string();
    query(db, async |conn| {
        sqlx::query_as(
            "SELECT seq::bigint, envelope FROM mailbox_entries
             WHERE device_id::text = $1 ORDER BY seq",
        )
        .bind(device)
        .fetch_all(conn)
        .await
    })
}

/// The envelopes of `device`'s mailbox in `seq` order, checking that their `seq`s are 1, 2, and
/// so on.
#[track_caller]
pub fn envelopes_of(db: &TestDb, device: DeviceId) -> Vec<Vec<u8>> {
    let entries = mailbox(db, device);
    let seqs: Vec<i64> = entries.iter().map(|(seq, _)| *seq).collect();
    let expected: Vec<i64> = (1..).take(entries.len()).collect();
    assert_eq!(seqs, expected, "the seqs of {device}'s mailbox");
    entries.into_iter().map(|(_, envelope)| envelope).collect()
}

/// Checks that no row of `db` holds `envelope`'s bytes: the server stored nothing of it.
#[track_caller]
pub fn assert_nowhere(db: &TestDb, envelope: &[u8]) {
    let hex: String = envelope.iter().map(|b| format!("{b:02x}")).collect();
    let rows = db.rows();
    let holding: Vec<&String> = rows.iter().filter(|row| row.contains(&hex)).collect();
    assert!(holding.is_empty(), "the envelope is stored: {holding:?}");
}
