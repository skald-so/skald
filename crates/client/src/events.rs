//! The mailbox's event stream on a connection the client opened: server-sent events in an HTTP/1.1
//! answer, its body chunked, of known length, or up to the connection's end. Each event's `data` is
//! one frame, its CBOR in base64url without padding (`docs/protocol/delivery.md`, "Frames"); a
//! comment, such as a keep-alive, and every other field are skipped.

use std::collections::VecDeque;
use std::io::{self, Read as _, Write as _};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use tungstenite::http::{HeaderMap, HeaderName, HeaderValue, header};
use tungstenite::stream::MaybeTlsStream;

use crate::connection::{Error, io_error, timed_out, uninterrupted};

/// The longest answer head, and the longest body of a refusal, the client reads.
const LIMIT: usize = 64 << 10;

/// How the answer's body is delimited.
#[derive(Debug, PartialEq)]
enum Framing {
    /// Chunked, at this point of its chunks.
    Chunked(Chunk),
    /// This many bytes left.
    Length(usize),
    /// Up to the connection's end.
    Close,
}

/// Where a chunked body is.
#[derive(Debug, PartialEq)]
enum Chunk {
    /// At a chunk's size line.
    Size,
    /// In a chunk's data, with this many bytes left.
    Data(usize),
    /// At the line end after a chunk's data.
    End,
}

/// An event stream: the answer to its request.
pub struct Events {
    stream: MaybeTlsStream<TcpStream>,
    /// What came and is not decoded yet: of the head, or of the body as it came.
    raw: Vec<u8>,
    /// How the body is delimited, once the head has come.
    framing: Option<Framing>,
    /// The body, decoded, not yet split into lines.
    text: Vec<u8>,
    /// The `data` of the event being read, once it has some.
    data: Option<String>,
    /// The frames of the events read, not yet taken.
    frames: VecDeque<Vec<u8>>,
    /// Whether the body has ended.
    ended: bool,
}

impl Events {
    /// The stream on `stream`, a connection to the server, before its request is sent.
    pub fn new(stream: MaybeTlsStream<TcpStream>) -> Self {
        Events {
            stream,
            raw: Vec::new(),
            framing: None,
            text: Vec::new(),
            data: None,
            frames: VecDeque::new(),
            ended: false,
        }
    }

    /// Sends `request`, failing after `timeout`.
    pub fn send(&mut self, request: &[u8], timeout: Duration) -> io::Result<()> {
        self.socket().set_write_timeout(Some(timeout))?;
        self.stream.write_all(request)?;
        self.stream.flush()
    }

    /// The answer's status and headers, each read waiting up to `timeout`.
    pub fn head(&mut self, timeout: Duration) -> Result<(u16, HeaderMap), Error> {
        let deadline = Instant::now() + timeout;
        let end = loop {
            if let Some(at) = self.raw.windows(4).position(|four| four == b"\r\n\r\n") {
                break at;
            }
            if self.raw.len() > LIMIT {
                return Err(Error::Unexpected(
                    "an answer head that is too long".to_owned(),
                ));
            }
            if !self.read(deadline, timeout)? {
                return Err(Error::Timeout(timeout));
            }
        };
        let head: Vec<u8> = self.raw.drain(..end + 4).collect();
        let head = String::from_utf8_lossy(&head);
        let mut lines = head.split("\r\n");
        let line = lines.next().unwrap_or_default();
        let mut words = line.splitn(3, ' ');
        let status = (words.next())
            .filter(|version| version.starts_with("HTTP/1."))
            .and(words.next())
            .and_then(|status| status.parse::<u16>().ok())
            .ok_or_else(|| Error::Unexpected("an answer that is not HTTP".to_owned()))?;
        let mut headers = HeaderMap::new();
        for line in lines.filter(|line| !line.is_empty()) {
            let (name, value) = line.split_once(':').unwrap_or((line, ""));
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.trim().as_bytes()),
                HeaderValue::from_str(value.trim()),
            ) {
                headers.append(name, value);
            }
        }
        let has = |name: header::HeaderName, value: &str| {
            headers.get_all(name).iter().any(|it| {
                it.to_str()
                    .is_ok_and(|it| it.to_ascii_lowercase().contains(value))
            })
        };
        self.framing = Some(if has(header::TRANSFER_ENCODING, "chunked") {
            Framing::Chunked(Chunk::Size)
        } else if let Some(length) = (headers.get(header::CONTENT_LENGTH))
            .and_then(|length| length.to_str().ok()?.trim().parse().ok())
        {
            Framing::Length(length)
        } else {
            Framing::Close
        });
        self.unframe()?;
        Ok((status, headers))
    }

    /// The whole body, up to [`LIMIT`] bytes, each read waiting up to `timeout`: that of a refusal.
    pub fn body(&mut self, timeout: Duration) -> Result<Vec<u8>, Error> {
        let deadline = Instant::now() + timeout;
        while !self.ended && self.text.len() <= LIMIT {
            if !self.read(deadline, timeout)? {
                return Err(Error::Timeout(timeout));
            }
        }
        Ok(std::mem::take(&mut self.text))
    }

    /// The next frame of the stream, waiting for it up to `wait`: `None` if none came by then. Fails
    /// once the stream has ended, or its connection broke, on its frames all taken.
    pub fn next(&mut self, wait: Duration) -> Result<Option<Vec<u8>>, Error> {
        let deadline = Instant::now() + wait;
        loop {
            self.split();
            if let Some(frame) = self.frames.pop_front() {
                return Ok(Some(frame));
            }
            if self.ended {
                return Err(Error::Unreachable("the server ended the stream".to_owned()));
            }
            if !self.read(deadline, wait)? {
                return Ok(None);
            }
        }
    }

    /// The connection's socket.
    fn socket(&self) -> &TcpStream {
        match &self.stream {
            MaybeTlsStream::Rustls(stream) => &stream.sock,
            MaybeTlsStream::Plain(stream) => stream,
            _ => unreachable!("the client opens no other stream"),
        }
    }

    /// Reads what comes until `deadline`, at least a millisecond, and decodes it; whether anything
    /// came. The connection's end ends the body. A failure is of a wait of `wait`.
    fn read(&mut self, deadline: Instant, wait: Duration) -> Result<bool, Error> {
        let left = deadline.saturating_duration_since(Instant::now());
        (self.socket())
            .set_read_timeout(Some(left.max(Duration::from_millis(1))))
            .map_err(|err| io_error(err, wait))?;
        let mut buffer = [0; 16 << 10];
        match uninterrupted(|| self.stream.read(&mut buffer)) {
            Ok(0) => {
                if self.framing != Some(Framing::Close) {
                    return Err(Error::Unreachable(
                        "the connection ended in the middle of an answer".to_owned(),
                    ));
                }
                self.ended = true;
                Ok(true)
            }
            Ok(count) => {
                self.raw.extend_from_slice(&buffer[..count]);
                self.unframe()?;
                Ok(true)
            }
            Err(err) if timed_out(&err) => Ok(false),
            Err(err) => Err(io_error(err, wait)),
        }
    }

    /// Moves what came of the body from `raw` to `text`, as its framing says, once the head has
    /// come. The body ends at its last chunk, or once its length has come.
    fn unframe(&mut self) -> Result<(), Error> {
        loop {
            let Some(framing) = &mut self.framing else {
                return Ok(());
            };
            match framing {
                Framing::Close => {
                    self.text.append(&mut self.raw);
                    return Ok(());
                }
                Framing::Length(left) => {
                    let count = (*left).min(self.raw.len());
                    self.text.extend(self.raw.drain(..count));
                    *left -= count;
                    self.ended = *left == 0;
                    return Ok(());
                }
                Framing::Chunked(Chunk::Size) => {
                    let Some(at) = self.raw.windows(2).position(|two| two == b"\r\n") else {
                        return Ok(());
                    };
                    let line: Vec<u8> = self.raw.drain(..at + 2).collect();
                    let line = String::from_utf8_lossy(&line[..at]);
                    let size = line.split(';').next().unwrap_or_default().trim();
                    let size = usize::from_str_radix(size, 16)
                        .map_err(|_| Error::Unexpected("a malformed chunked answer".to_owned()))?;
                    if size == 0 {
                        self.ended = true;
                        self.framing = Some(Framing::Length(0));
                        return Ok(());
                    }
                    *framing = Framing::Chunked(Chunk::Data(size));
                }
                Framing::Chunked(Chunk::Data(left)) => {
                    let count = (*left).min(self.raw.len());
                    if count == 0 {
                        return Ok(());
                    }
                    self.text.extend(self.raw.drain(..count));
                    *left -= count;
                    if *left == 0 {
                        *framing = Framing::Chunked(Chunk::End);
                    }
                }
                Framing::Chunked(Chunk::End) => {
                    if self.raw.len() < 2 {
                        return Ok(());
                    }
                    self.raw.drain(..2);
                    *framing = Framing::Chunked(Chunk::Size);
                }
            }
        }
    }

    /// Splits the complete lines of `text` into events, and takes the frame of each one with
    /// `data` that decodes from base64url.
    fn split(&mut self) {
        while let Some(at) = self.text.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.text.drain(..=at).collect();
            let line = String::from_utf8_lossy(&line[..at]);
            let line = line.strip_suffix('\r').unwrap_or(&line);
            if line.is_empty() {
                if let Some(data) = self.data.take()
                    && let Ok(frame) = URL_SAFE_NO_PAD.decode(data)
                {
                    self.frames.push_back(frame);
                }
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            if field == "data" {
                match &mut self.data {
                    Some(data) => {
                        data.push('\n');
                        data.push_str(value);
                    }
                    None => self.data = Some(value.to_owned()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    /// A stream on a connection over this machine, and the connection's peer.
    fn pair() -> (Events, TcpStream) {
        let (stream, peer) = crate::connection::tests::pair();
        (Events::new(MaybeTlsStream::Plain(stream)), peer)
    }

    /// A stream on a connection whose peer answers `answer`, in the pieces it is split into, then
    /// closes.
    fn answered(pieces: &[&[u8]]) -> Events {
        let (events, mut peer) = pair();
        let pieces: Vec<Vec<u8>> = pieces.iter().map(|piece| piece.to_vec()).collect();
        thread::spawn(move || {
            for piece in pieces {
                peer.write_all(&piece).unwrap();
                peer.flush().unwrap();
                thread::sleep(Duration::from_millis(5));
            }
        });
        events
    }

    /// An event of `frame`, its `data` in base64url.
    fn event(frame: &[u8]) -> String {
        format!("data: {}\n\n", URL_SAFE_NO_PAD.encode(frame))
    }

    /// The frames of a chunked stream come whole, however its chunks and events are split; a
    /// comment, another field and an event without `data` are skipped.
    #[test]
    fn chunked_stream_gives_each_events_frame() {
        let body = format!(
            ": keep-alive\n\nid: 7\nevent: x\n{}retry: 5\n\n{}",
            event(b"one"),
            event(b"two").replace('\n', "\r\n")
        );
        let (first, rest) = body.as_bytes().split_at(20);
        let chunk = |bytes: &[u8]| {
            let mut chunk = format!("{:x};ext=1\r\n", bytes.len()).into_bytes();
            chunk.extend_from_slice(bytes);
            chunk.extend_from_slice(b"\r\n");
            chunk
        };
        let head = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
            Transfer-Encoding: chunked\r\n\r\n";
        let rest = chunk(rest);
        let (second, third) = rest.split_at(7);
        let mut events = answered(&[head, &chunk(first), second, third, b"0\r\n\r\n"]);

        let (status, headers) = events.head(SECOND).expect("the head");

        assert_eq!(status, 200);
        assert_eq!(headers[header::CONTENT_TYPE], "text/event-stream");
        assert_eq!(events.next(SECOND).unwrap(), Some(b"one".to_vec()));
        assert_eq!(events.next(SECOND).unwrap(), Some(b"two".to_vec()));
        let error = events.next(SECOND).expect_err("ended");
        assert!(matches!(error, Error::Unreachable(_)), "{error:?}");
    }

    /// A stream with nothing more to give gives nothing within its wait, and keeps what came of an
    /// event for the next read.
    #[test]
    fn stream_waits_for_an_event_and_keeps_its_part() {
        let (mut events, mut peer) = pair();
        peer.write_all(b"HTTP/1.1 200 OK\r\n\r\ndata: b25").unwrap();
        events.head(SECOND).expect("the head");

        assert_eq!(events.next(Duration::from_millis(50)).unwrap(), None);
        peer.write_all(b"l\n\n").unwrap();
        assert_eq!(events.next(SECOND).unwrap(), Some(b"one".to_vec()));
    }

    /// A refusal's body is read by its length; a malformed chunked answer or a connection that
    /// ends in the middle of one fails.
    #[test]
    fn refusal_body_is_read_by_its_length_and_broken_answers_fail() {
        let mut events = answered(&[b"HTTP/1.1 403 Forbidden\r\ncontent-length: 7\r\n\r\nblocked"]);
        assert_eq!(events.head(SECOND).expect("the head").0, 403);
        assert_eq!(events.body(SECOND).expect("the body"), b"blocked");

        let mut events =
            answered(&[b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nzz\r\n"]);
        let error = events.head(SECOND).expect_err("malformed");
        assert!(matches!(error, Error::Unexpected(_)), "{error:?}");

        let mut events = answered(&[b"HTTP/1.1 200 OK\r\ncontent-length: 9\r\n\r\ndata"]);
        events.head(SECOND).expect("the head");
        let error = events.next(SECOND).expect_err("cut short");
        assert!(matches!(error, Error::Unreachable(_)), "{error:?}");

        let mut events = answered(&[b"SSH-2.0-OpenSSH\r\n\r\n"]);
        let error = events.head(SECOND).expect_err("not HTTP");
        assert!(matches!(error, Error::Unexpected(_)), "{error:?}");
    }

    /// An event's `data` split across two chunks comes whole, also when a read ends right after a
    /// chunk's data, before its line end.
    #[test]
    fn data_split_across_chunks_comes_whole() {
        let mut events = answered(&[
            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n9\r\ndata: b25",
            b"\r\n3\r\nl\n\n\r\n",
            b"0\r\n\r\n",
        ]);
        events.head(SECOND).expect("the head");

        assert_eq!(events.next(SECOND).unwrap(), Some(b"one".to_vec()));
        let error = events.next(SECOND).expect_err("ended");
        assert!(matches!(error, Error::Unreachable(_)), "{error:?}");
    }

    /// A refusal's body that comes after its head, in a later write, is read: by its length, or up
    /// to the connection's end without one.
    #[test]
    fn refusal_body_after_its_head_is_read() {
        let mut events = answered(&[
            b"HTTP/1.1 403 Forbidden\r\ncontent-length: 7\r\n\r\n",
            b"blocked",
        ]);
        events.head(SECOND).expect("the head");
        assert_eq!(events.body(SECOND).expect("the body"), b"blocked");

        let mut events = answered(&[b"HTTP/1.1 403 Forbidden\r\n\r\n", b"blocked"]);
        events.head(SECOND).expect("the head");
        assert_eq!(events.body(SECOND).expect("the body"), b"blocked");
    }

    /// A head longer than [`LIMIT`] fails; of a longer refusal body, a little over [`LIMIT`] is
    /// read, without waiting for the rest.
    #[test]
    fn head_and_refusal_body_are_read_up_to_their_limit() {
        let long = format!("HTTP/1.1 200 OK\r\nx: {}", "a".repeat(LIMIT + 1));
        let mut events = answered(&[long.as_bytes()]);
        let error = events.head(SECOND).expect_err("too long");
        assert!(matches!(error, Error::Unexpected(_)), "{error:?}");

        let (mut events, mut peer) = pair();
        let head = format!(
            "HTTP/1.1 403 Forbidden\r\ncontent-length: {}\r\n\r\n",
            4 * LIMIT
        );
        peer.write_all(head.as_bytes()).unwrap();
        peer.write_all(&vec![b'a'; LIMIT + 1]).unwrap();
        events.head(SECOND).expect("the head");
        let body = events.body(10 * SECOND).expect("the body up to its limit");
        assert!(
            (LIMIT + 1..4 * LIMIT).contains(&body.len()),
            "{}",
            body.len()
        );
    }

    /// A read that fails other than by its wait fails at once: here a peer that does not speak TLS.
    #[test]
    fn failed_read_fails_at_once() {
        let (stream, mut peer) = crate::connection::tests::pair();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let tls = rustls::ClientConnection::new(crate::tls::Roots::load().rustls(), name).unwrap();
        let mut events = Events::new(MaybeTlsStream::Rustls(rustls::StreamOwned::new(
            tls, stream,
        )));
        peer.write_all(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();

        let error = events.head(10 * SECOND).expect_err("not TLS");

        assert!(matches!(error, Error::Unreachable(_)), "{error:?}");
    }
}
