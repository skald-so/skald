//! Connections authenticated as a device: a device session gets a challenge, and a WebSocket is
//! taken only once its first frame proves, on that challenge, the key of a `client` device with a
//! certificate of the session's account. Every HTTP response is checked against the OpenAPI spec
//! in `api/openapi.json`.
//!
//! The API and the frame, as these tests use them:
//! - `POST /session/challenge`, stating the protocol version, with `Authorization: Bearer <token>`
//!   of a device session: `200` with `{"challenge": <bytes>}`, base64url without padding. Without a
//!   valid device session, such as none, a web session's cookie or token, or an ended or unknown
//!   one: `401` with a problem details object.
//! - The client's first frame on `/ws`: `auth = [10, challenge: bstr, device: id, signature: bstr
//!   .size 64]`, one CBOR item in a binary frame. `signature` is by the device's signing key over
//!   `["skald auth", challenge, origin]`, `origin` the server's public address as an origin
//!   (RFC 6454), such as `http://skald.test:41234`, as a byte string.
//! - The server takes the connection and keeps it open. Otherwise it closes it with code 1008
//!   (policy violation) and the reason:
//!   - `auth-required`: the first frame is not `auth`, or none comes within the wait, a few
//!     seconds;
//!   - `challenge-refused`: the challenge is unknown, was used, is a minute old or older, or its
//!     session has ended;
//!   - `device-refused`: the device is not a `client` device with a certificate on the server of
//!     the challenge's account;
//!   - `signature-refused`: the signature does not verify with the device's signing key over the
//!     challenge and the server's origin.
//! - A challenge works once, on any instance of the server. A connection closes when its session
//!   ends.

mod common;

use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

use common::device_flow::{DeviceFlow, json, text};
use common::devices::{
    Device, Key, Machine, account_id, assert_refused, assert_taken, b64, cert_of, machine, random,
    unb64,
};
use common::sign_in::{CLIENT_SECRET, SECRET_ENV, github_settings};
use common::{Response, Server, WebSocket, send};
use skald_crypto::SigningKey;
use skald_proto::{CertBody, DeviceId, DeviceKind, PROTOCOL_VERSION, VERSION_HEADER};
use tungstenite::Message;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;

/// A device session's challenge: `POST`.
const CHALLENGE: &str = "/session/challenge";
/// The label of the signature in `auth`.
const AUTH_LABEL: &str = "skald auth";
/// The type of the `auth` frame.
const AUTH: u8 = 10;

const AUTH_REQUIRED: &str = "auth-required";
const CHALLENGE_REFUSED: &str = "challenge-refused";
const DEVICE_REFUSED: &str = "device-refused";
const SIGNATURE_REFUSED: &str = "signature-refused";

/// How long a test waits for the server to close a connection; longer than the proof wait.
const CLOSE_WITHIN: Duration = Duration::from_secs(15);
/// Past the lifetime of a challenge: a minute.
const PAST_LIFETIME: Duration = Duration::from_secs(62);

/// A machine signed in to an account whose first device, with the master key, holds a
/// certificate on the server.
struct Linked<'f> {
    machine: Machine<'f>,
    device: Device,
    master: Key,
}

/// Signs a machine in as the GitHub user `id`, `login`, registers its device and gives the
/// account's master key with the device's certificate.
fn linked<'f>(flow: &'f DeviceFlow, id: u64, login: &str) -> Linked<'f> {
    let user = flow.stand.user(id, login);
    let machine = machine(flow, &user);
    let device = Device::new("Laptop");
    let master = Key::generate();
    machine.register_device(&device);
    assert_taken(&machine.give_master(&master, &device.cert(&machine.account, &master)));
    Linked {
        machine,
        device,
        master,
    }
}

/// `POST /session/challenge` with the request headers `headers`, stating the protocol version if
/// `stated`; checked against the spec.
fn challenge_response(flow: &DeviceFlow, headers: &[(&str, &str)], stated: bool) -> Response {
    let version = PROTOCOL_VERSION.to_string();
    let mut all = headers.to_vec();
    if stated {
        all.push((VERSION_HEADER, &version));
    }
    let url = format!("{}{CHALLENGE}", flow.stand.server.base());
    let response = send("post", &url, &all, &[]);
    flow.conforms("post", CHALLENGE, &response);
    response
}

/// A challenge for the device session `token`.
fn challenge(flow: &DeviceFlow, token: &str) -> Vec<u8> {
    let bearer = format!("Bearer {token}");
    let response = challenge_response(flow, &[("authorization", &bearer)], true);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    let challenge = unb64(
        body["challenge"]
            .as_str()
            .unwrap_or_else(|| panic!("no challenge in {body}")),
    );
    assert!(challenge.len() >= 16, "a guessable challenge: {body}");
    challenge
}

/// The head of a CBOR byte string of `len` bytes, then the bytes.
fn bstr(bytes: &[u8]) -> Vec<u8> {
    let len = bytes.len();
    let mut out = match len {
        0..24 => vec![0x40 | u8::try_from(len).unwrap()],
        24..256 => vec![0x58, u8::try_from(len).unwrap()],
        256..65_536 => {
            let mut head = vec![0x59];
            head.extend_from_slice(&u16::try_from(len).unwrap().to_be_bytes());
            head
        }
        _ => panic!("a challenge of {len} bytes"),
    };
    out.extend_from_slice(bytes);
    out
}

/// The `auth` frame of `challenge`, `device` and `signature`.
fn auth_frame(challenge: &[u8], device: DeviceId, signature: &[u8; 64]) -> Vec<u8> {
    let mut frame = vec![0x84, AUTH];
    frame.extend(bstr(challenge));
    frame.extend(bstr(device.as_bytes()));
    frame.extend(bstr(signature));
    frame
}

/// The signature of `auth` by `key` over `challenge` and `origin`.
fn sign(key: &SigningKey, challenge: &[u8], origin: &str) -> [u8; 64] {
    key.sign(AUTH_LABEL, &[challenge, origin.as_bytes()])
}

/// Opens a WebSocket at `/ws` on `server`, stating the protocol version, with a read timeout.
fn open(server: &Server) -> WebSocket {
    let offer = skald_proto::subprotocol(PROTOCOL_VERSION);
    let (socket, response) = server
        .open_websocket("/ws", Some(&offer))
        .unwrap_or_else(|response| panic!("/ws: {}", text(&response)));
    assert_eq!(response.status, 101);
    if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
        TcpStream::set_read_timeout(stream, Some(Duration::from_millis(500)))
            .expect("set a timeout");
    }
    socket
}

/// Opens a WebSocket on `server` and sends `auth` of `challenge` and `device`, signed by `key`
/// over `origin`.
fn connect(
    server: &Server,
    challenge: &[u8],
    device: DeviceId,
    key: &SigningKey,
    origin: &str,
) -> WebSocket {
    let mut socket = open(server);
    let frame = auth_frame(challenge, device, &sign(key, challenge, origin));
    socket
        .send(Message::Binary(frame.into()))
        .expect("send auth");
    socket
}

/// Opens a WebSocket on `flow`'s server and proves `linked`'s device on `challenge`, as its client
/// does.
fn connect_as(flow: &DeviceFlow, linked: &Linked, challenge: &[u8]) -> WebSocket {
    connect(
        &flow.stand.server,
        challenge,
        linked.device.id,
        &linked.device.signing.key,
        &flow.stand.public,
    )
}

/// How the server closed a connection.
#[derive(Debug, PartialEq, Eq)]
enum Closed {
    /// With a close frame of this code and reason.
    Frame(CloseCode, String),
    /// Without a close frame.
    Dropped,
}

/// Waits up to `within` for the server to close `socket`; `None` if it is still open by then.
fn closing(socket: &mut WebSocket, within: Duration) -> Option<Closed> {
    let deadline = Instant::now() + within;
    loop {
        match socket.read() {
            Ok(Message::Close(frame)) => {
                return Some(frame.map_or(Closed::Dropped, |frame| {
                    Closed::Frame(frame.code, frame.reason.as_str().to_owned())
                }));
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            Ok(message) => panic!("a frame before the connection is taken: {message:?}"),
            Err(tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if Instant::now() > deadline {
                    return None;
                }
            }
            Err(_) => return Some(Closed::Dropped),
        }
    }
}

/// Checks that the server closes `socket` with code 1008 and `reason`, taking nothing.
#[track_caller]
fn assert_closed_with(socket: &mut WebSocket, reason: &str) {
    let closed = closing(socket, CLOSE_WITHIN);
    assert_eq!(
        closed,
        Some(Closed::Frame(CloseCode::Policy, reason.to_owned())),
        "closed with {reason}"
    );
}

/// Checks that `socket` is open: the server answers a ping.
#[track_caller]
fn assert_open(socket: &mut WebSocket) {
    socket
        .send(Message::Ping(b"open".to_vec().into()))
        .expect("send a ping");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match socket.read() {
            Ok(Message::Pong(payload)) if payload.as_ref() == b"open".as_slice() => return,
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            Ok(message) => panic!("the connection is not open: {message:?}"),
            Err(tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) && Instant::now() < deadline => {}
            Err(err) => panic!("the connection is not open: {err}"),
        }
    }
}

/// Checks that `server` took `socket`: it is still open once a connection opened after it without
/// a proof has been closed for sending none.
#[track_caller]
fn assert_connection_taken(server: &Server, socket: &mut WebSocket) {
    let mut bare = open(server);
    assert_closed_with(&mut bare, AUTH_REQUIRED);
    assert_open(socket);
}

/// Checks that a proof of each of `devices`, each on a new challenge of `linked`'s session, is
/// refused for its device.
#[track_caller]
fn assert_devices_refused(flow: &DeviceFlow, linked: &Linked, devices: &[&Device]) {
    for device in devices {
        let challenge = challenge(flow, &linked.machine.token);
        let mut socket = connect(
            &flow.stand.server,
            &challenge,
            device.id,
            &device.signing.key,
            &flow.stand.public,
        );

        assert_closed_with(&mut socket, DEVICE_REFUSED);
    }
}

/// Writes to `flow`'s database a device of `kind` on `linked`'s account, with a certificate issued
/// by the account's master key: a device the server's API does not register yet.
fn device_of_kind(flow: &DeviceFlow, linked: &Linked, kind: DeviceKind, name: &str) -> Device {
    let device = Device::new(name);
    let body = CertBody {
        kind,
        ..device.body(&linked.machine.account)
    };
    let cert = cert_of(&body, &linked.master.key, &device.signing.key);
    let kind = match kind {
        DeviceKind::Client => "client",
        DeviceKind::Browser => "browser",
        DeviceKind::Service => "service",
    };
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    flow.stand.db.execute(&format!(
        "INSERT INTO devices (id, account_id, kind, signing, hpke, name, cert)
         VALUES ('{}', '{}', '{kind}', decode('{}', 'hex'), decode('{}', 'hex'),
                 decode('{}', 'hex'), decode('{}', 'hex'))",
        device.id,
        account_id(&linked.machine.account),
        hex(&device.signing.public()),
        hex(&device.hpke),
        hex(device.name.as_bytes()),
        hex(&cert),
    ));
    device
}

/// A second instance of `flow`'s server, on its database, with its public address and sign-in.
fn second_instance(flow: &DeviceFlow) -> Server {
    let settings = github_settings(&flow.stand.public, &flow.stand.github);
    Server::launch(
        &flow.stand.db,
        "127.0.0.1:0",
        &settings,
        &[(SECRET_ENV, CLIENT_SECRET)],
    )
}

/// A certified `client` device that proves its key on a challenge of its account's session gets a
/// connection, which the server keeps open.
#[test]
fn certified_client_device_proving_its_key_gets_a_connection() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_001, "ada-connects-standin");
    let challenge = challenge(&flow, &ada.machine.token);

    let mut socket = connect_as(&flow, &ada, &challenge);

    assert_connection_taken(&flow.stand.server, &mut socket);
}

/// Each challenge is new: two of one session differ.
#[test]
fn each_challenge_is_new() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_002, "ada-two-challenges-standin");

    let first = challenge(&flow, &ada.machine.token);
    let second = challenge(&flow, &ada.machine.token);

    assert_ne!(first, second);
}

/// A challenge of any device session of the account serves its certified device: one of another
/// machine signed in to it.
#[test]
fn challenge_of_another_session_of_the_account_is_taken() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_003, "ada-other-session-standin");
    let user = flow.stand.user(3_121_220_003, "ada-other-session-standin");
    let other = machine(&flow, &user);
    assert_eq!(other.account, ada.machine.account);

    let mut socket = connect_as(&flow, &ada, &challenge(&flow, &other.token));

    assert_connection_taken(&flow.stand.server, &mut socket);
}

/// A proof that names the account's certified device but is signed by another key is refused.
#[test]
fn proof_signed_by_another_key_is_refused() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_004, "ada-other-key-standin");
    let challenge = challenge(&flow, &ada.machine.token);
    let other = Key::generate();

    let mut socket = connect(
        &flow.stand.server,
        &challenge,
        ada.device.id,
        &other.key,
        &flow.stand.public,
    );

    assert_closed_with(&mut socket, SIGNATURE_REFUSED);
}

/// A proof of another account's certified device, on a challenge of this account's session, is
/// refused.
#[test]
fn proof_of_a_device_of_another_account_is_refused() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_005, "ada-own-account-standin");
    let bob = linked(&flow, 3_121_220_006, "bob-another-account-standin");
    let challenge = challenge(&flow, &ada.machine.token);

    let mut socket = connect_as(&flow, &bob, &challenge);

    assert_closed_with(&mut socket, DEVICE_REFUSED);
}

/// A proof of a device of the account that has no certificate on the server, such as one waiting
/// for linking, is refused; so is one of a device the server does not know.
#[test]
fn proof_of_a_device_without_a_certificate_is_refused() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_007, "ada-uncertified-standin");
    let waiting = Device::new("Desktop");
    ada.machine.register_device(&waiting);
    let unknown = Device::new("Unknown");

    assert_devices_refused(&flow, &ada, &[&waiting, &unknown]);
}

/// A proof of a `browser` or a `service` device of the account, with a certificate issued by its
/// master key, is refused: those kinds come with later stories.
#[test]
fn proof_of_a_browser_or_service_device_is_refused() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_008, "ada-other-kinds-standin");
    let browser = device_of_kind(&flow, &ada, DeviceKind::Browser, "Browser");
    let service = device_of_kind(&flow, &ada, DeviceKind::Service, "CI");

    assert_devices_refused(&flow, &ada, &[&browser, &service]);
}

/// A proof signed over an origin other than the server's public address is refused, so a malicious
/// server cannot relay a challenge to another one: over the address the test reaches the server
/// at, and over another server's.
#[test]
fn proof_over_another_origin_is_refused() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_009, "ada-other-origin-standin");
    assert_ne!(flow.stand.server.base(), flow.stand.public);

    for origin in [flow.stand.server.base(), "https://skald.example"] {
        let challenge = challenge(&flow, &ada.machine.token);
        let mut socket = connect(
            &flow.stand.server,
            &challenge,
            ada.device.id,
            &ada.device.signing.key,
            origin,
        );

        assert_closed_with(&mut socket, SIGNATURE_REFUSED);
    }
}

/// A challenge works once: a second connection with it is refused, and the first stays.
#[test]
fn challenge_used_twice_is_refused() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_010, "ada-twice-standin");
    let challenge = challenge(&flow, &ada.machine.token);
    let mut first = connect_as(&flow, &ada, &challenge);
    assert_connection_taken(&flow.stand.server, &mut first);

    let mut second = connect_as(&flow, &ada, &challenge);

    assert_closed_with(&mut second, CHALLENGE_REFUSED);
    assert_open(&mut first);
}

/// A challenge one instance of the server issued works on another instance on the same database,
/// and only once across both.
#[test]
fn challenge_works_once_on_any_instance() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_011, "ada-instances-standin");
    let other = second_instance(&flow);
    let challenge = challenge(&flow, &ada.machine.token);
    let key = &ada.device.signing.key;

    let mut taken = connect(&other, &challenge, ada.device.id, key, &flow.stand.public);
    assert_connection_taken(&other, &mut taken);

    let mut again = connect_as(&flow, &ada, &challenge);
    assert_closed_with(&mut again, CHALLENGE_REFUSED);
    let mut on_other = connect(&other, &challenge, ada.device.id, key, &flow.stand.public);
    assert_closed_with(&mut on_other, CHALLENGE_REFUSED);
}

/// A challenge the server never issued is refused.
#[test]
fn unknown_challenge_is_refused() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_012, "ada-unknown-challenge-standin");

    let mut socket = connect_as(&flow, &ada, &random());

    assert_closed_with(&mut socket, CHALLENGE_REFUSED);
}

/// A challenge expires within a minute: past it, the proof is refused.
#[test]
fn expired_challenge_is_refused() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_013, "ada-expired-standin");
    let challenge = challenge(&flow, &ada.machine.token);

    thread::sleep(PAST_LIFETIME);
    let mut socket = connect_as(&flow, &ada, &challenge);

    assert_closed_with(&mut socket, CHALLENGE_REFUSED);
}

/// A challenge stands for its session: once the session has ended, its proof is refused.
#[test]
fn challenge_of_an_ended_session_is_refused() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_014, "ada-ended-standin");
    let challenge = challenge(&flow, &ada.machine.token);
    let ended = ada.machine.sign_out();
    assert_eq!(ended.status, 204, "{}", text(&ended));

    let mut socket = connect_as(&flow, &ada, &challenge);

    assert_closed_with(&mut socket, CHALLENGE_REFUSED);
}

/// A connection whose first frame is not `auth` is closed, also when `auth` follows: a `sync`, and
/// a text frame.
#[test]
fn connection_whose_first_frame_is_another_is_closed() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_015, "ada-other-frame-standin");
    let sync = vec![0x82, 0x04, 0x00];

    for first in [Message::Binary(sync.into()), Message::Text("auth".into())] {
        let challenge = challenge(&flow, &ada.machine.token);
        let mut socket = open(&flow.stand.server);
        socket.send(first).expect("send the first frame");
        let signature = sign(&ada.device.signing.key, &challenge, &flow.stand.public);
        let auth = auth_frame(&challenge, ada.device.id, &signature);
        // The server may have closed the connection already.
        let _ = socket.send(Message::Binary(auth.into()));

        assert_closed_with(&mut socket, AUTH_REQUIRED);
    }
}

/// A connection that sends no proof within the wait, a few seconds, is closed.
#[test]
fn connection_without_a_proof_is_closed() {
    let flow = DeviceFlow::start();

    let mut socket = open(&flow.stand.server);

    assert_closed_with(&mut socket, AUTH_REQUIRED);
}

/// Signing out closes the connection of the session's challenge; ending another session of the
/// account leaves it open.
#[test]
fn signing_out_closes_the_connection() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_016, "ada-signs-out-standin");
    let user = flow.stand.user(3_121_220_016, "ada-signs-out-standin");
    let other = machine(&flow, &user);
    let mut socket = connect_as(&flow, &ada, &challenge(&flow, &ada.machine.token));
    assert_connection_taken(&flow.stand.server, &mut socket);

    assert_eq!(other.sign_out().status, 204);
    thread::sleep(Duration::from_secs(2));
    assert_open(&mut socket);

    let ended = ada.machine.sign_out();
    assert_eq!(ended.status, 204, "{}", text(&ended));
    assert!(
        closing(&mut socket, Duration::from_secs(10)).is_some(),
        "the connection is still open after its session ended"
    );
}

/// No challenge without a device session: none, an unknown token, a web session's cookie or token,
/// or an ended device session.
#[test]
fn no_challenge_without_a_device_session() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_017, "ada-no-challenge-standin");
    let web_token = ada
        .machine
        .cookie
        .strip_prefix("skald_session=")
        .unwrap_or_else(|| panic!("a session cookie: {}", ada.machine.cookie));
    let unknown = format!("Bearer {}", b64(&random()));
    let web_bearer = format!("Bearer {web_token}");
    let user = flow.stand.user(3_121_220_017, "ada-no-challenge-standin");
    let ended = machine(&flow, &user);
    assert_eq!(ended.sign_out().status, 204);
    let ended_bearer = format!("Bearer {}", ended.token);

    let cases: [Vec<(&str, &str)>; 5] = [
        vec![],
        vec![("authorization", &unknown)],
        vec![("cookie", &ada.machine.cookie)],
        vec![("authorization", &web_bearer)],
        vec![("authorization", &ended_bearer)],
    ];
    for headers in cases {
        let response = challenge_response(&flow, &headers, true);
        assert_refused(&response, 401);
        assert!(
            json(&response).get("challenge").is_none(),
            "{headers:?}: {}",
            text(&response)
        );
    }
    // The device session itself still gets one.
    challenge(&flow, &ada.machine.token);
}

/// A challenge request states the protocol version: one without it is refused.
#[test]
fn challenge_request_states_the_protocol_version() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_018, "ada-version-standin");
    let bearer = format!("Bearer {}", ada.machine.token);

    let response = challenge_response(&flow, &[("authorization", &bearer)], false);

    assert_eq!(response.status, 400, "{}", text(&response));
    assert_eq!(
        json(&response)["type"],
        "https://skald.so/problems/protocol-version-invalid"
    );
}

/// Signing out closes the session's connection with the reason `session-ended`.
#[test]
fn signing_out_closes_the_connection_as_its_session_ended() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_019, "ada-session-ended-standin");
    let mut socket = connect_as(&flow, &ada, &challenge(&flow, &ada.machine.token));
    assert_connection_taken(&flow.stand.server, &mut socket);

    assert_eq!(ada.machine.sign_out().status, 204);

    assert_closed_with(&mut socket, "session-ended");
}

/// A refused proof uses its challenge up: the device's own proof with it is refused after.
#[test]
fn refused_proof_uses_its_challenge_up() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_020, "ada-refused-once-standin");
    let challenge = challenge(&flow, &ada.machine.token);
    let other = Key::generate();
    let mut refused = connect(
        &flow.stand.server,
        &challenge,
        ada.device.id,
        &other.key,
        &flow.stand.public,
    );
    assert_closed_with(&mut refused, SIGNATURE_REFUSED);

    let mut socket = connect_as(&flow, &ada, &challenge);

    assert_closed_with(&mut socket, CHALLENGE_REFUSED);
}

/// The server answers the close of a taken connection with the closing handshake.
#[test]
fn taken_connection_closes_with_the_closing_handshake() {
    let flow = DeviceFlow::start();
    let ada = linked(&flow, 3_121_220_021, "ada-closes-standin");
    let mut socket = connect_as(&flow, &ada, &challenge(&flow, &ada.machine.token));
    assert_connection_taken(&flow.stand.server, &mut socket);

    socket.close(None).expect("send a close");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match socket.read() {
            Ok(Message::Close(_) | Message::Ping(_) | Message::Pong(_)) => {}
            Ok(message) => panic!("a frame after the close: {message:?}"),
            Err(tungstenite::Error::ConnectionClosed) => break,
            Err(tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) && Instant::now() < deadline => {}
            Err(err) => panic!("close: {err}"),
        }
    }
}
