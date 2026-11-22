//! A person's device played by the test over the delivery protocol, as its own client would: it
//! proves the device's key on a challenge of the machine's session, sends envelopes the test seals,
//! and reads and acks its mailbox (`docs/protocol/delivery.md`, "Connecting", "Send" and
//! "Mailbox"). It reaches the server at the relay, signing over the server's public address.

use std::net::TcpStream;
use std::num::NonZeroU64;
use std::time::Duration;

use skald_crypto::SigningKey;
use skald_proto::{
    AUTH_LABEL, Cbor as _, ChannelId, DeviceId, Frame, HistoryItem, PROTOCOL_VERSION, Push,
    RawEnvelope, Scheme, SpaceId,
};
use tungstenite::client::IntoClientRequest as _;
use tungstenite::http::HeaderValue;
use tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL;
use tungstenite::{Message, WebSocket};

use super::cloud::{Cloud, unb64};
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
        Self::connect_on(cloud, &person.token, person.device, key)
    }

    /// Connects as `device`, whose signing key is `key`, on a challenge of the bearer token
    /// `token`: a device session's, or a `service` device's token.
    pub fn connect_on(cloud: &Cloud, token: &str, device: DeviceId, key: &SigningKey) -> Self {
        let (status, body) = post_as(cloud, token, "/session/challenge", None);
        assert!(
            (200..300).contains(&status),
            "no challenge: {status} {body}"
        );
        let challenge = body["challenge"]
            .as_str()
            .map(unb64)
            .unwrap_or_else(|| panic!("a challenge in base64url: {body}"));
        let signature = key.sign(AUTH_LABEL, &[&challenge, cloud.public.as_bytes()]);
        let address = cloud.relay.address();
        let mut request = format!("ws://{address}/ws")
            .into_client_request()
            .expect("a WebSocket request");
        let offer = HeaderValue::from_str(&skald_proto::subprotocol(PROTOCOL_VERSION))
            .expect("a subprotocol header");
        request.headers_mut().insert(SEC_WEBSOCKET_PROTOCOL, offer);
        let stream = TcpStream::connect(address).expect("reach the relay");
        stream.set_read_timeout(Some(READ)).expect("a read timeout");
        let (socket, _) = tungstenite::client(request, stream)
            .unwrap_or_else(|err| panic!("open a WebSocket: {err}"));
        let mut peer = Peer { socket };
        peer.write(&Frame::Auth {
            challenge,
            device,
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

    /// Uploads `item` of `channel` in `space` in a `history` frame numbered `upload`; gives the
    /// server's answer to it, `history-stored` or `history-refused`.
    pub fn upload(
        &mut self,
        upload: u64,
        (space, channel): (SpaceId, ChannelId),
        item: HistoryItem,
    ) -> Frame {
        self.write(&Frame::History {
            upload,
            space,
            channel,
            item,
        });
        self.answer_to(upload)
    }

    /// Uploads `object`, a queue key object's bytes, as the `history` item `[4, queue-key]`
    /// (`docs/protocol/queues.md`, "Queue key object"), in a frame built from bytes; gives the
    /// server's answer, `history-stored` or `history-refused`.
    pub fn upload_queue_key(
        &mut self,
        upload: u64,
        (space, channel): (SpaceId, ChannelId),
        object: &[u8],
    ) -> Frame {
        let mut frame = vec![0x85, 0x0b];
        frame.extend(cbor_head(0, upload));
        for id in [space.as_bytes(), channel.as_bytes()] {
            frame.push(0x50);
            frame.extend_from_slice(id);
        }
        frame.extend([0x82, 0x04]);
        let length = u64::try_from(object.len()).expect("a length within 64 bits");
        frame.extend(cbor_head(2, length));
        frame.extend_from_slice(object);
        self.socket
            .send(Message::Binary(frame.into()))
            .unwrap_or_else(|err| panic!("send a queue key object: {err}"));
        self.answer_to(upload)
    }

    /// The server's answer to the upload `upload`, `history-stored` or `history-refused`.
    fn answer_to(&mut self, upload: u64) -> Frame {
        loop {
            let answer = self.read();
            if let Frame::HistoryStored { upload: of } | Frame::HistoryRefused { upload: of, .. } =
                answer
                && of == upload
            {
                return answer;
            }
        }
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

/// The envelopes of the `message` entries of `entries` under `hpke` to `device`, in order: the
/// channels' Welcomes and Commits left out.
pub fn messages(entries: &[Push], device: DeviceId) -> Vec<RawEnvelope> {
    entries
        .iter()
        .filter_map(|push| match push {
            Push::Message { envelope, .. }
                if envelope.envelope().header.scheme == Scheme::Hpke
                    && envelope.envelope().header.recipients.as_deref() == Some(&[device][..]) =>
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

/// The head of a CBOR item of major type `major` with the argument `argument`.
fn cbor_head(major: u8, argument: u64) -> Vec<u8> {
    let major = major << 5;
    match argument {
        short @ 0..=23 => vec![major | u8::try_from(short).expect("below 24")],
        byte @ 24..=0xff => vec![major | 24, u8::try_from(byte).expect("8 bits")],
        half @ 0x100..=0xffff => {
            let mut head = vec![major | 25];
            head.extend(u16::try_from(half).expect("16 bits").to_be_bytes());
            head
        }
        word @ 0x1_0000..=0xffff_ffff => {
            let mut head = vec![major | 26];
            head.extend(u32::try_from(word).expect("32 bits").to_be_bytes());
            head
        }
        long => {
            let mut head = vec![major | 27];
            head.extend(long.to_be_bytes());
            head
        }
    }
}
