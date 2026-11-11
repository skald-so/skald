//! Runs `skald-server` and checks its HTTP API against the OpenAPI spec in `api/openapi.json`.

mod common;

use std::net::TcpListener;
use std::process::Command;

use common::spec::{spec, spec_bytes, spec_violations};
use common::{Response, SERVER, Server, TestDb, start_failure};
use serde_json::{Value, json};

impl Response {
    fn json(status: u16, body: Value) -> Self {
        Response {
            status,
            content_type: "application/json".to_owned(),
            headers: Vec::new(),
            body: body.to_string().into_bytes(),
        }
    }

    fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

#[test]
fn health_conforms_to_spec() {
    let db = TestDb::create();
    let server = Server::start(&db);
    let response = server.get("/health");

    assert_eq!(response.status, 200);
    assert_eq!(
        spec_violations(&spec(), "/health", &response),
        Vec::<String>::new()
    );
    let body: Value = serde_json::from_slice(&response.body).expect("body is JSON");
    assert_eq!(body, json!({ "status": "ok" }));
}

#[test]
fn health_reports_unreachable_database() {
    let db = TestDb::create();
    let server = Server::start(&db);
    assert_eq!(server.get("/health").status, 200);

    db.set_reachable(false);
    let response = server.get("/health");
    assert_eq!(response.status, 503);
    assert_eq!(
        spec_violations(&spec(), "/health", &response),
        Vec::<String>::new()
    );
    let body: Value = serde_json::from_slice(&response.body).expect("body is JSON");
    assert_eq!(body, json!({ "status": "unavailable" }));

    db.set_reachable(true);
    assert_eq!(server.get("/health").status, 200);
}

#[test]
fn published_spec_matches_repository() {
    let db = TestDb::create();
    let server = Server::start(&db);
    let response = server.get("/openapi.json");

    assert_eq!(response.status, 200);
    assert!(
        response.body == spec_bytes(),
        "GET /openapi.json differs from api/openapi.json"
    );
    assert_eq!(
        spec_violations(&spec(), "/openapi.json", &response),
        Vec::<String>::new()
    );
}

#[test]
fn spec_violations_reports_responses_outside_spec() {
    let spec = spec();
    let ok = json!({ "status": "ok" });
    let unavailable = json!({ "status": "unavailable" });

    let mut with_charset = Response::json(200, ok.clone());
    with_charset.content_type = "application/json; charset=utf-8".to_owned();
    assert_eq!(
        spec_violations(&spec, "/health", &with_charset),
        Vec::<String>::new()
    );
    assert_eq!(
        spec_violations(&spec, "/health", &Response::json(503, unavailable.clone())),
        Vec::<String>::new()
    );

    let protocol = Response::json(200, json!({ "supported": [2, 3] }));
    let switching = Response {
        content_type: String::new(),
        body: Vec::new(),
        ..Response::json(101, Value::Null)
    }
    .with_header("sec-websocket-protocol", "skald.3");
    let not_websocket = Response {
        content_type: "text/plain; charset=utf-8".to_owned(),
        body: b"Connection header did not include 'upgrade'".to_vec(),
        ..Response::json(400, Value::Null)
    };
    for (path, response) in [
        ("/protocol", &protocol),
        ("/ws", &switching),
        ("/ws", &not_websocket),
    ] {
        assert_eq!(
            spec_violations(
                &spec,
                path,
                &response.clone().with_header("skald-protocol-latest", "3")
            ),
            Vec::<String>::new()
        );
    }

    let mut text = Response::json(200, ok.clone());
    text.content_type = "text/plain".to_owned();
    let not_json = Response {
        body: b"ok".to_vec(),
        ..Response::json(200, Value::Null)
    };
    let switching_with_body = Response {
        body: b"ok".to_vec(),
        ..switching.clone()
    };
    let outside = [
        // A required header is missing or breaks its schema.
        ("/protocol", protocol.clone()),
        (
            "/protocol",
            protocol.clone().with_header("skald-protocol-latest", "0"),
        ),
        (
            "/protocol",
            protocol.with_header("skald-protocol-latest", "three"),
        ),
        ("/ws", switching.clone()),
        (
            "/ws",
            Response {
                headers: Vec::new(),
                ..switching.clone()
            }
            .with_header("skald-protocol-latest", "3"),
        ),
        (
            "/ws",
            Response {
                headers: Vec::new(),
                ..switching.clone()
            }
            .with_header("sec-websocket-protocol", "skald.03")
            .with_header("skald-protocol-latest", "3"),
        ),
        // A body where the spec lists no content.
        (
            "/ws",
            switching_with_body.with_header("skald-protocol-latest", "3"),
        ),
        ("/missing", Response::json(200, ok.clone())),
        ("/health", Response::json(503, ok)),
        ("/health", Response::json(200, unavailable.clone())),
        ("/health", Response::json(500, unavailable)),
        ("/health", text),
        ("/health", not_json),
        ("/health", Response::json(200, json!({}))),
        ("/health", Response::json(200, json!({ "status": "down" }))),
        (
            "/health",
            Response::json(200, json!({ "status": "ok", "extra": 1 })),
        ),
    ];
    for (path, response) in outside {
        assert!(
            !spec_violations(&spec, path, &response).is_empty(),
            "GET {path} {} {} {:?} passed",
            response.status,
            response.content_type,
            String::from_utf8_lossy(&response.body)
        );
    }
}

#[test]
fn version_prints_package_version() {
    let output = Command::new(SERVER)
        .arg("--version")
        .output()
        .expect("run skald-server");

    assert!(output.status.success(), "exit status: {}", output.status);
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        format!("skald-server {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn exits_with_error_when_address_is_taken() {
    let taken = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let addr = taken.local_addr().expect("local address").to_string();
    let db = TestDb::create();

    let stderr = start_failure(&["--listen", &addr, "--database-url", &db.url()]);

    assert!(
        stderr.starts_with(&format!("skald-server: cannot listen on {addr}: ")),
        "stderr: {stderr:?}"
    );
}
