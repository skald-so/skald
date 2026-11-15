//! A person's device played by the test over the delivery protocol, as its own client would: it
//! proves the device's key on a challenge of the machine's session, sends envelopes the test seals,
//! and reads and acks its mailbox (`docs/protocol/delivery.md`, "Connecting", "Send" and
//! "Mailbox"). It reaches the server at the relay, signing over the server's public address.

use std::net::TcpStream;
use std::num::NonZeroU64;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use skald_crypto::SigningKey;
use skald_proto::{AUTH_LABEL, Cbor as _, DeviceId, Frame, PROTOCOL_VERSION, Push, RawEnvelope};
use tungstenite::client::IntoClientRequest as _;
use tungstenite::http::HeaderValue;
use tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL;
use tungstenite::{Message, WebSocket};

use super::cloud::Cloud;
use super::world::{Person, post_as};

/// How long the peer waits for a frame from the server.
const READ: Duration = Duration::from_secs(30);

/// An open connection of a device.
pub struct Peer {
    socket: WebSocket<TcpStream>,
}

impl Peer {
    /// Connects as `person`'s machine's device, whose signing key is `key`, on the machine's
    /// session.
    pub fn connect(cloud: &Cloud, person: &Person, key: &SigningKey) -> Self {
        let (status, body) = post_as(cloud, &person.token, "/session/challenge", None);
        assert!(
            (200..300).contains(&status),
            "no challenge: {status} {body}"
        );
        let challenge = body["challenge"]
            .as_str()
            .and_then(|text| URL_SAFE_NO_PAD.decode(text).ok())
            .unwrap_or_else(|| panic!("a challenge in base64url: {body}"));
        let signature = key.sign(AUTH_LABEL, &[&challenge, cloud.public.as_bytes()]);
        let address = cloud.relay.url.trim_start_matches("http://").to_owned();
        let mut request = format!("ws://{address}/ws")
            .into_client_request()
            .expect("a WebSocket request");
        let offer = HeaderValue::from_str(&skald_proto::subprotocol(PROTOCOL_VERSION))
            .expect("a subprotocol header");
        request.headers_mut().insert(SEC_WEBSOCKET_PROTOCOL, offer);
        let stream = TcpStream::connect(&address).expect("reach the relay");
        stream.set_read_timeout(Some(READ)).expect("a read timeout");
        let (socket, _) = tungstenite::client(request, stream)
            .unwrap_or_else(|err| panic!("open a WebSocket: {err}"));
        let mut peer = Peer { socket };
        peer.write(&Frame::Auth {
            challenge,
            device: person.device,
            signature,
        });
        peer
    }

    fn write(&mut self, frame: &Frame) {
        self.socket
            .send(Message::Binary(frame.to_cbor().into()))
            .unwrap_or_else(|err| panic!("send {frame:?}: {err}"));
    }

    /// The next frame the server sends; fails if none comes within [`READ`].
    fn read(&mut self) -> Frame {
        loop {
            match self.socket.read() {
                Ok(Message::Binary(bytes)) => {
                    return Frame::from_cbor(&bytes).expect("a frame that decodes");
                }
                Ok(Message::Close(close)) => panic!("the server closed the connection: {close:?}"),
                Ok(_) => {}
                Err(err) => panic!("read a frame: {err}"),
            }
        }
    }

    /// Sends `envelope` with `report` true; gives the server's answer, `stored` or
    /// `send-refused`.
    pub fn send(&mut self, envelope: &RawEnvelope) -> Frame {
        self.write(&Frame::Send {
            envelope: envelope.clone(),
            report: true,
        });
        loop {
            if let answer @ (Frame::Stored(_) | Frame::SendRefused { .. }) = self.read() {
                return answer;
            }
        }
    }

    /// Sends `envelope`; fails unless the server stores it.
    #[track_caller]
    pub fn stored(&mut self, envelope: &RawEnvelope) {
        let answer = self.send(envelope);
        assert!(matches!(answer, Frame::Stored(_)), "not stored: {answer:?}");
    }

    /// States the cursor `cursor` in the connection's first `sync`; gives every entry of
    /// catch-up, until the empty `batch` that ends it.
    pub fn catch_up(&mut self, cursor: u64) -> Vec<Push> {
        self.write(&Frame::Sync { cursor });
        let mut entries = Vec::new();
        loop {
            match self.read() {
                Frame::Push(push) => entries.push(push),
                Frame::Batch(batch) if batch.is_empty() => return entries,
                Frame::Batch(batch) => entries.extend(batch),
                _ => {}
            }
        }
    }

    /// Acks every entry up to `seq`.
    pub fn ack(&mut self, seq: u64) {
        self.write(&Frame::Ack {
            seq: NonZeroU64::new(seq).expect("a seq above 0"),
        });
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
    }
}

/// The envelopes of the `message` entries of `entries` to `device`, in order.
pub fn messages(entries: &[Push], device: DeviceId) -> Vec<RawEnvelope> {
    entries
        .iter()
        .filter_map(|push| match push {
            Push::Message { envelope, .. }
                if envelope.envelope().header.recipients.as_deref() == Some(&[device][..]) =>
            {
                Some(envelope.clone())
            }
            _ => None,
        })
        .collect()
}

/// The highest `seq` of `entries`, if any.
pub fn last_seq(entries: &[Push]) -> Option<u64> {
    entries
        .iter()
        .filter_map(|push| match push {
            Push::Message { seq, .. } | Push::Status { seq, .. } => Some(seq.get()),
            _ => None,
        })
        .max()
}
