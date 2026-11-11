//! The protocol version check: each row of "Server answers" in `docs/protocol/README.md`, over HTTPS
//! and WebSocket opening requests, with the server's range set to versions 2–3.

mod common;

use std::net::TcpStream;
use std::time::Duration;

use common::spec::{spec, spec_violations};
use common::{Response, Server, TestDb, WebSocket, start_failure};
use serde_json::{Value, json};
use skald_proto::PROTOCOL_VERSION;
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;

/// The range the tests set: N = 3, N−1 = 2.
const RANGE: [&str; 2] = ["--protocol-versions", "2-3"];

/// Versions that are not valid, in `Skald-Protocol-Version` or after `skald.`.
const INVALID: [&str; 8] = ["0", "00", "02", "-1", "+2", "2.0", "two", ""];

fn conforms(path: &str, response: &Response) {
    assert_eq!(
        spec_violations(&spec(), path, response),
        Vec::<String>::new()
    );
}

/// `GET /protocol` stating `version`.
fn protocol(server: &Server, version: &str) -> Response {
    server.get_with("/protocol", &[("Skald-Protocol-Version", version)])
}

/// Checks that `response` to `GET <path>` refuses the request with `problem`, the server accepting
/// `supported` as `accepts` says; `stated` is what the detail says the request stated.
fn assert_refused(
    path: &str,
    response: &Response,
    problem: &str,
    title: &str,
    accepts: &str,
    stated: &str,
    supported: &[u64],
) {
    assert_eq!(response.status, 400);
    assert_eq!(response.content_type, "application/problem+json");
    let latest = supported.last().unwrap().to_string();
    assert_eq!(
        response.header("skald-protocol-latest"),
        Some(latest.as_str())
    );
    let body: Value = serde_json::from_slice(&response.body).expect("body is JSON");
    assert_eq!(
        body,
        json!({
            "type": format!("https://skald.so/problems/{problem}"),
            "title": title,
            "status": 400,
            "detail": format!("This server accepts {accepts}; the request stated {stated}."),
            "supported": supported,
        })
    );
    conforms(path, response);
}

fn assert_too_old(path: &str, response: &Response, stated: &str) {
    let accepts = "protocol versions 2 and 3";
    let title = "Please update skald";
    assert_refused(
        path,
        response,
        "protocol-too-old",
        title,
        accepts,
        stated,
        &[2, 3],
    );
}

fn assert_too_new(path: &str, response: &Response, stated: &str) {
    let accepts = "protocol versions 2 and 3";
    let title = "This server is older than the client";
    assert_refused(
        path,
        response,
        "protocol-too-new",
        title,
        accepts,
        stated,
        &[2, 3],
    );
}

fn assert_invalid(path: &str, response: &Response) {
    let accepts = "protocol versions 2 and 3";
    let title = "Missing or invalid protocol version";
    let stated = "no valid version";
    assert_refused(
        path,
        response,
        "protocol-version-invalid",
        title,
        accepts,
        stated,
        &[2, 3],
    );
}

/// Opens a WebSocket at `/ws` offering `offer`; the server must accept it with `selected`.
fn assert_websocket_opens(server: &Server, offer: &str, selected: &str) {
    let (mut socket, response) = server
        .open_websocket("/ws", Some(offer))
        .unwrap_or_else(|response| panic!("{offer:?}: status {}", response.status));
    assert_eq!(response.status, 101);
    assert_eq!(response.header("sec-websocket-protocol"), Some(selected));
    assert_eq!(response.header("skald-protocol-latest"), Some("3"));
    conforms("/ws", &response);
    assert_stays_open(&mut socket);
}

/// The server keeps `socket` open: it answers a ping, then a close.
fn assert_stays_open(socket: &mut WebSocket) {
    if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
        TcpStream::set_read_timeout(stream, Some(Duration::from_secs(10))).expect("set a timeout");
    }
    socket
        .send(Message::Ping(b"ping".to_vec().into()))
        .expect("send a ping");
    assert_eq!(
        socket.read().expect("read a pong"),
        Message::Pong(b"ping".to_vec().into())
    );
    socket.close(None).expect("send a close");
    loop {
        match socket.read() {
            Ok(Message::Close(_)) => {}
            Ok(message) => panic!("unexpected frame: {message:?}"),
            Err(tungstenite::Error::ConnectionClosed) => break,
            Err(err) => panic!("close: {err}"),
        }
    }
}

/// Sends a WebSocket opening request to `/ws` offering `offer`; the server must refuse it.
fn websocket_refusal(server: &Server, offer: Option<&str>) -> Response {
    match server.open_websocket("/ws", offer) {
        Ok(_) => panic!("{offer:?}: the WebSocket opened"),
        Err(response) => response,
    }
}

#[test]
fn https_serves_current_and_previous_versions() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    for version in ["3", "2"] {
        let response = protocol(&server, version);
        assert_eq!(response.status, 200, "version {version}");
        assert_eq!(response.header("skald-protocol-latest"), Some("3"));
        let body: Value = serde_json::from_slice(&response.body).expect("body is JSON");
        assert_eq!(body, json!({ "supported": [2, 3] }));
        conforms("/protocol", &response);
    }
}

#[test]
fn https_tells_older_client_to_update() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    assert_too_old("/protocol", &protocol(&server, "1"), "1");
}

#[test]
fn https_refuses_newer_version() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    for version in ["4", "10", "99999999999999999999999"] {
        assert_too_new("/protocol", &protocol(&server, version), version);
    }
}

#[test]
fn https_refuses_missing_or_invalid_version() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    assert_invalid("/protocol", &server.get("/protocol"));
    for version in INVALID.iter().chain(&["2,3", "2, 3"]) {
        assert_invalid("/protocol", &protocol(&server, version));
    }
    let twice = [
        ("Skald-Protocol-Version", "3"),
        ("Skald-Protocol-Version", "3"),
    ];
    assert_invalid("/protocol", &server.get_with("/protocol", &twice));
}

#[test]
fn websocket_opens_with_current_and_previous_versions() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    assert_websocket_opens(&server, "skald.3", "skald.3");
    assert_websocket_opens(&server, "skald.2", "skald.2");
    assert_websocket_opens(&server, "chat, skald.3", "skald.3");
}

#[test]
fn websocket_tells_older_client_to_update() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    assert_too_old("/ws", &websocket_refusal(&server, Some("skald.1")), "1");
}

#[test]
fn websocket_refuses_newer_version() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    assert_too_new("/ws", &websocket_refusal(&server, Some("skald.4")), "4");
}

#[test]
fn websocket_refuses_missing_or_invalid_version() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    assert_invalid("/ws", &websocket_refusal(&server, None));
    assert_invalid("/ws", &websocket_refusal(&server, Some("chat")));
    assert_invalid("/ws", &websocket_refusal(&server, Some("skald.2, skald.3")));
    for version in INVALID {
        let offer = format!("skald.{version}");
        assert_invalid("/ws", &websocket_refusal(&server, Some(&offer)));
    }
}

#[test]
fn websocket_path_refuses_plain_request() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    let response = server.get_with("/ws", &[("Sec-WebSocket-Protocol", "skald.3")]);
    assert_eq!(response.status, 400);
    assert!(
        response.content_type.starts_with("text/plain"),
        "content type: {:?}",
        response.content_type
    );
    assert_eq!(response.header("skald-protocol-latest"), Some("3"));
    conforms("/ws", &response);

    // The version is checked first.
    assert_too_old(
        "/ws",
        &server.get_with("/ws", &[("Sec-WebSocket-Protocol", "skald.1")]),
        "1",
    );
}

#[test]
fn default_range_is_current_and_previous_version() {
    let current = PROTOCOL_VERSION.get();
    let supported: Vec<u64> = (current.saturating_sub(1).max(1)..=current).collect();
    let db = TestDb::create();
    let server = Server::start(&db);

    for version in &supported {
        let response = protocol(&server, &version.to_string());
        assert_eq!(response.status, 200, "version {version}");
        assert_eq!(
            response.header("skald-protocol-latest"),
            Some(current.to_string().as_str())
        );
        let body: Value = serde_json::from_slice(&response.body).expect("body is JSON");
        assert_eq!(body, json!({ "supported": supported }));
    }
    let offer = format!("skald.{current}");
    let (mut socket, response) = server
        .open_websocket("/ws", Some(&offer))
        .unwrap_or_else(|response| panic!("status {}", response.status));
    assert_eq!(
        response.header("sec-websocket-protocol"),
        Some(offer.as_str())
    );
    assert_stays_open(&mut socket);

    let newer = (current + 1).to_string();
    let accepts = match supported.as_slice() {
        [only] => format!("protocol version {only}"),
        [previous, current] => format!("protocol versions {previous} and {current}"),
        _ => unreachable!(),
    };
    let title = "This server is older than the client";
    let response = protocol(&server, &newer);
    assert_refused(
        "/protocol",
        &response,
        "protocol-too-new",
        title,
        &accepts,
        &newer,
        &supported,
    );
}

#[test]
fn single_version_range_refuses_previous_version() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &["--protocol-versions", "3"]);

    let response = protocol(&server, "3");
    assert_eq!(response.status, 200);
    let body: Value = serde_json::from_slice(&response.body).expect("body is JSON");
    assert_eq!(body, json!({ "supported": [3] }));

    let title = "Please update skald";
    let accepts = "protocol version 3";
    let response = protocol(&server, "2");
    assert_refused(
        "/protocol",
        &response,
        "protocol-too-old",
        title,
        accepts,
        "2",
        &[3],
    );
}

#[test]
fn requests_without_protocol_are_not_checked() {
    let db = TestDb::create();
    let server = Server::start_with(&db, &RANGE);

    for path in ["/health", "/openapi.json"] {
        for headers in [
            &[][..],
            &[("Skald-Protocol-Version", "1")][..],
            &[("Skald-Protocol-Version", "two")][..],
        ] {
            let response = server.get_with(path, headers);
            assert_eq!(response.status, 200, "GET {path} with {headers:?}");
            conforms(path, &response);
        }
    }
}

#[test]
fn exits_with_error_on_invalid_range() {
    let db = TestDb::create();
    for range in ["3-2", "1-3", "5-7", "0", "0-1", "2-", "-2", "two", ""] {
        let stderr = start_failure(&[
            "--listen",
            "127.0.0.1:0",
            "--database-url",
            &db.url(),
            &format!("--protocol-versions={range}"),
        ]);
        assert!(
            stderr.contains("--protocol-versions"),
            "{range:?}: stderr: {stderr:?}"
        );
    }
}
