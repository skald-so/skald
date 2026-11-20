//! Runs `skald-server` and checks its HTTP API against the OpenAPI spec in `api/openapi.json`; and
//! the tests' own HTTP client.

mod common;

use std::net::TcpListener;
use std::panic::{self, AssertUnwindSafe};
use std::process::Command;
use std::time::{Duration, Instant};

use common::spec::{spec, spec_bytes, spec_violations};
use common::{Response, SERVER, Server, TestDb, on_free_port, run, start_failure};
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

/// The tests' HTTP client keeps its connection for the next request. A request on a kept
/// connection that its server closes as the request comes, a GET, goes again on a new one.
#[test]
fn the_tests_http_client_keeps_its_connection_and_sends_again_when_the_server_closes_it() {
    use std::io::{BufRead, BufReader, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread;

    // Answers each request on a connection with `ok`, until the connection ends; with `closing`
    // set, closes the connection of the next request instead.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let url = format!("http://{}/", listener.local_addr().expect("local address"));
    let closing = Arc::new(AtomicBool::new(false));
    let close_next = Arc::clone(&closing);
    let (accepted, connections) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = accepted.send(());
            let closing = Arc::clone(&close_next);
            thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().expect("clone a stream"));
                let mut writer = stream;
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if line != "\r\n" {
                        continue;
                    }
                    if closing.swap(false, Ordering::SeqCst)
                        || writer
                            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                            .is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    let get = || {
        common::agent::get(|| common::agent::agent().get(&url).call())
            .expect("GET")
            .body_mut()
            .read_to_string()
            .expect("the body")
    };

    assert_eq!((get(), get()), ("ok".to_owned(), "ok".to_owned()));
    assert_eq!(connections.try_iter().count(), 1, "connections");

    closing.store(true, Ordering::SeqCst);
    assert_eq!(get(), "ok");
    assert_eq!(connections.try_iter().count(), 1, "new connections");
}

/// The server refuses a `POST /key-packages` that states no protocol version before reading its
/// body, whose rest has not come: the answer says `Connection: close` and the server closes the
/// connection after it. The answer to a `GET /health`, a request without a body, keeps it.
#[test]
fn answer_to_a_request_whose_body_is_left_unread_closes_the_connection() {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let db = TestDb::create();
    let server = Server::start(&db);
    let address = server.base().trim_start_matches("http://").to_owned();
    // The head of the answer to `request` on a new connection, in lowercase, and the connection.
    let answer = |request: &str| -> (String, TcpStream) {
        let mut stream = TcpStream::connect(&address).expect("reach the server");
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("a read timeout");
        stream.write_all(request.as_bytes()).expect("send");
        let mut head = Vec::new();
        let mut byte = [0; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).expect("an answer");
            head.push(byte[0]);
        }
        (String::from_utf8_lossy(&head).to_lowercase(), stream)
    };

    let (head, mut stream) = answer(&format!(
        "POST /key-packages HTTP/1.1\r\nhost: {address}\r\ncontent-type: application/json\r\n\
         content-length: 64\r\n\r\n{{"
    ));
    assert!(head.starts_with("http/1.1 400"), "{head}");
    assert!(head.contains("\r\nconnection: close\r\n"), "{head}");
    let mut rest = Vec::new();
    stream
        .read_to_end(&mut rest)
        .expect("the connection closed");

    let (head, _) = answer(&format!("GET /health HTTP/1.1\r\nhost: {address}\r\n\r\n"));
    assert!(head.starts_with("http/1.1 200"), "{head}");
    assert!(!head.contains("connection: close"), "{head}");
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

#[test]
fn exits_with_error_on_a_google_issuer_neither_https_nor_http_on_loopback() {
    // A closed port: a server that took the issuer would fail at the database instead.
    let url = "postgres://postgres@127.0.0.1:1/postgres";
    let secret = [("SKALD_GOOGLE_CLIENT_SECRET", "stand-in-google-secret")];
    for issuer in [
        "http://accounts.google.example",
        "ftp://127.0.0.1",
        "not a URL",
    ] {
        let (started, output) = run(
            &[
                "--listen",
                "127.0.0.1:0",
                "--database-url",
                url,
                "--public-url",
                "http://127.0.0.1:8080",
                "--google-client-id",
                "stand-in-google-client",
                "--google-issuer",
                issuer,
            ],
            &secret,
        );
        assert!(!started, "{issuer}: {output}");
        assert!(
            output.starts_with(&format!("skald-server: the issuer {issuer:?}: ")),
            "{issuer}: {output}"
        );
    }
}

/// A case of a provider's settings: the arguments, the environment variables and the refusal.
type ProviderHalf = (
    &'static [&'static str],
    &'static [(&'static str, &'static str)],
    &'static str,
);

/// A sign-in provider's client ID and its secret go together: with only one of them, the server
/// exits naming the other.
#[test]
fn exits_with_error_on_a_provider_s_client_id_or_secret_alone() {
    // A closed port: a server that took the settings would fail at the database instead.
    let url = "postgres://postgres@127.0.0.1:1/postgres";
    let cases: [ProviderHalf; 4] = [
        (
            &["--github-client-id", "stand-in-github-client"],
            &[],
            "skald-server: --github-client-id needs the client secret in SKALD_GITHUB_CLIENT_SECRET",
        ),
        (
            &[],
            &[("SKALD_GITHUB_CLIENT_SECRET", "stand-in-github-secret")],
            "skald-server: SKALD_GITHUB_CLIENT_SECRET needs --github-client-id",
        ),
        (
            &["--google-client-id", "stand-in-google-client"],
            &[],
            "skald-server: --google-client-id needs the client secret in SKALD_GOOGLE_CLIENT_SECRET",
        ),
        (
            &[],
            &[("SKALD_GOOGLE_CLIENT_SECRET", "stand-in-google-secret")],
            "skald-server: SKALD_GOOGLE_CLIENT_SECRET needs --google-client-id",
        ),
    ];
    for (args, envs, refusal) in cases {
        let output = Command::new(SERVER)
            .args([
                "--listen",
                "127.0.0.1:0",
                "--database-url",
                url,
                "--public-url",
                "http://127.0.0.1:8080",
            ])
            .args(args)
            .env_remove("SKALD_GITHUB_CLIENT_ID")
            .env_remove("SKALD_GITHUB_CLIENT_SECRET")
            .env_remove("SKALD_GOOGLE_CLIENT_ID")
            .env_remove("SKALD_GOOGLE_CLIENT_SECRET")
            .envs(envs.iter().copied())
            .output()
            .expect("run skald-server");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{refusal}: {stderr}");
        assert!(stderr.starts_with(refusal), "{refusal}: {stderr}");
    }
}

/// The tests' servers whose address must be known before they start: the port they are given can
/// be taken before they listen on it.
#[test]
fn a_test_server_whose_port_is_taken_before_it_listens_starts_on_another() {
    let db = TestDb::create();
    let holder = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let taken = holder.local_addr().expect("local address").port();
    let mut launches = Vec::new();

    // The first launch is on the taken port, as if another socket took the one it was given.
    let server = on_free_port(Duration::from_secs(30), |port| {
        let port = if launches.is_empty() { taken } else { port };
        let launched = Server::try_launch(&db, &format!("127.0.0.1:{port}"), &[], &[]);
        launches.push((port, launched.is_ok()));
        launched
    });

    assert_eq!(launches.first(), Some(&(taken, false)), "{launches:?}");
    let &(port, listens) = launches.last().expect("a launch");
    assert!(listens && port != taken, "{launches:?}");
    assert_eq!(server.base(), format!("http://127.0.0.1:{port}"));
    assert_eq!(server.get("/health").status, 200);
}

#[test]
fn a_test_server_that_fails_to_start_for_another_reason_is_launched_once() {
    let db = TestDb::create();
    let mut launches = 0;

    let failure = panic_of(|| {
        on_free_port(Duration::from_secs(5), |port| {
            launches += 1;
            let no_secret = ["--github-client-id", "Ov23li0standin0client"];
            Server::try_launch(&db, &format!("127.0.0.1:{port}"), &no_secret, &[])
        })
    });

    assert_eq!(launches, 1, "{failure}");
    assert!(
        failure.starts_with(
            "unexpected skald-server output: \"skald-server: --github-client-id needs the client \
             secret in SKALD_GITHUB_CLIENT_SECRET"
        ),
        "{failure}"
    );
}

#[test]
fn a_test_server_whose_port_stays_taken_is_given_up_at_the_deadline() {
    let db = TestDb::create();
    let holder = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let taken = holder.local_addr().expect("local address").port();
    let started = Instant::now();

    let failure = panic_of(|| {
        on_free_port(Duration::from_secs(1), |_| {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "launches went on 10 s past a deadline of 1 s"
            );
            Server::try_launch(&db, &format!("127.0.0.1:{taken}"), &[], &[])
        })
    });

    assert!(
        failure.starts_with(&format!(
            "no free port for skald-server within 1s: \"skald-server: cannot listen on \
             127.0.0.1:{taken}: Address already in use"
        )),
        "{failure}"
    );
}

/// What `run` panics with; fails if it returns.
fn panic_of<T>(run: impl FnOnce() -> T) -> String {
    let Err(payload) = panic::catch_unwind(AssertUnwindSafe(run)) else {
        panic!("no panic");
    };
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => payload
            .downcast_ref::<&str>()
            .map_or_else(String::new, |message| (*message).to_owned()),
    }
}
