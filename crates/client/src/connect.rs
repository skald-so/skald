//! The machine's connection to the server as its device: a WebSocket whose first frame proves the
//! device's key (`docs/protocol/delivery.md`, "Connecting"). A device without a certificate opens
//! none.

use std::path::Path;
use std::time::Duration;

use skald_proto::VersionRange;

use crate::connection::WebSocket;
use crate::device;
use crate::devices::{signed_in_with_secrets, text, to_session};
use crate::secrets;
use crate::state::Link;

/// What a run that connects as its device says on a machine whose device holds no certificate.
pub const NOT_LINKED: &str = "this device is not linked yet: it holds no certificate signed by the \
                              account's master key, so it opens no connection to the server";

/// A WebSocket at `/ws` to the server of the session of the machine whose state is in `dir`, its
/// secrets in `storage`, as the machine's device; [`NOT_LINKED`] if the device holds no
/// certificate.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "skald send and skald recv connect with it")
)]
pub fn connect(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
) -> Result<WebSocket, String> {
    let (state, server, token, stored) = signed_in_with_secrets(dir, storage)?;
    let device = state.device().map_err(text)?;
    let Some(device) = device.filter(|device| matches!(device.link, Link::Certified { .. })) else {
        return Err(NOT_LINKED.to_owned());
    };
    let keys = device::known_keys(&stored, Some(&device))
        .map_err(text)?
        .ok_or_else(|| text(device::Error::MissingKeys))?;
    to_session(&server, speaks, timeout)?
        .open_websocket_as("/ws", &token, keys.id(), keys.signing())
        .map_err(text)
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use skald_proto::{AUTH_LABEL, Cbor as _, Frame};
    use tungstenite::Message;
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

    use super::*;
    use crate::device::Keys;
    use crate::secrets::{Map, Secrets};
    use crate::sign_in::{SESSION_SERVER, SESSION_TOKEN};
    use crate::state::State;
    use crate::test_dir::TestDir;

    const TOKEN: &str = "token-1";
    const TIMEOUT: Duration = Duration::from_secs(10);

    /// What the stand-in server saw: the challenge request's line and headers, and the WebSocket's
    /// first frame.
    struct Seen {
        request: Vec<String>,
        first: Vec<u8>,
    }

    /// A stand-in server that answers one challenge request with `challenge`, then takes one
    /// WebSocket, selecting the subprotocol it offers, and reads its first frame.
    fn stand_in(challenge: &[u8]) -> (String, thread::JoinHandle<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let url = format!("http://{}", listener.local_addr().expect("the address"));
        let body = format!(r#"{{"challenge":"{}"}}"#, URL_SAFE_NO_PAD.encode(challenge));
        let served = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the challenge request");
            let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
            let mut request = Vec::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("a request line");
                if line == "\r\n" {
                    break;
                }
                request.push(line.trim_end().to_ascii_lowercase());
            }
            write!(
                &stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("answer");
            drop(stream);
            let (stream, _) = listener.accept().expect("the WebSocket");
            let mut socket = tungstenite::accept_hdr(stream, select).expect("a WebSocket");
            let first = match socket.read().expect("a frame") {
                Message::Binary(bytes) => bytes.to_vec(),
                other => panic!("not a binary frame: {other:?}"),
            };
            Seen { request, first }
        });
        (url, served)
    }

    /// Selects the subprotocol `request` offers.
    #[expect(clippy::result_large_err, reason = "tungstenite's callback")]
    fn select(request: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
        let offered = request.headers()["sec-websocket-protocol"].clone();
        response
            .headers_mut()
            .insert("sec-websocket-protocol", offered);
        Ok(response)
    }

    /// A machine signed in to `server` whose device is linked as `link`; its keys.
    fn machine(dir: &Path, server: &str, link: &Link) -> Keys {
        let keys = Keys::generate();
        let state = State::open(dir).unwrap();
        state.start_session(server).unwrap();
        state.add_device(keys.id(), "Laptop", "account-1").unwrap();
        state.set_link(link).unwrap();
        let mut stored = Map::from([
            (SESSION_TOKEN.to_owned(), TOKEN.to_owned()),
            (SESSION_SERVER.to_owned(), server.to_owned()),
        ]);
        keys.store(&mut stored);
        Secrets::new(dir, secrets::Storage::File)
            .save(&stored)
            .unwrap();
        keys
    }

    fn certified() -> Link {
        Link::Certified {
            master: [7; 32],
            cert: vec![0x80],
        }
    }

    /// A certified device asks for a challenge with its session's token, then sends `auth` first:
    /// its device, signed with its key over the challenge and the server's origin, without the
    /// path of its URL.
    #[test]
    fn certified_device_proves_its_key_on_a_challenge() {
        let dir = TestDir::new("connect-certified");
        let challenge = [0x3c; 32];
        let (origin, served) = stand_in(&challenge);
        let server = format!("{origin}/skald");
        let keys = machine(&dir, &server, &certified());

        let socket = connect(
            &dir,
            secrets::Storage::File,
            VersionRange::default(),
            TIMEOUT,
        )
        .expect("a connection");

        let seen = served.join().unwrap();
        drop(socket);
        assert_eq!(seen.request[0], "post /skald/session/challenge http/1.1");
        assert!(
            seen.request
                .contains(&format!("authorization: bearer {TOKEN}")),
            "{:?}",
            seen.request
        );
        let Frame::Auth {
            challenge: signed,
            device,
            signature,
        } = Frame::from_cbor(&seen.first).expect("a frame")
        else {
            panic!("not auth: {:?}", seen.first);
        };
        assert_eq!(
            (signed.as_slice(), device),
            (challenge.as_slice(), keys.id())
        );
        skald_crypto::verify(
            &keys.signing().public_key(),
            AUTH_LABEL,
            &[&challenge, origin.as_bytes()],
            &signature,
        )
        .expect("signed over the challenge and the origin");
    }

    /// A device without a certificate opens no connection and says it is not linked yet.
    #[test]
    fn device_without_a_certificate_opens_none() {
        for (name, link) in [
            ("new", Link::New),
            ("waiting", Link::Waiting),
            (
                "sent",
                Link::Sent {
                    master: [7; 32],
                    cert: vec![0x80],
                },
            ),
        ] {
            let dir = TestDir::new(&format!("connect-{name}"));
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
            listener.set_nonblocking(true).unwrap();
            let server = format!("http://{}", listener.local_addr().unwrap());
            machine(&dir, &server, &link);

            let refused = connect(
                &dir,
                secrets::Storage::File,
                VersionRange::default(),
                TIMEOUT,
            );

            assert_eq!(refused.err().as_deref(), Some(NOT_LINKED), "{name}");
            assert!(listener.accept().is_err(), "{name}: the server was reached");
        }
    }
}
