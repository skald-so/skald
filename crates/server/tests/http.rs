//! Runs `skald-server` and checks its HTTP API against the OpenAPI spec in `api/openapi.json`.

use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use jsonschema::Registry;
use serde_json::{Value, json};

const SERVER: &str = env!("CARGO_BIN_EXE_skald-server");

/// The base URI the spec is registered under to resolve `$ref`s.
const SPEC_URI: &str = "urn:skald:openapi";

fn spec_bytes() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../api/openapi.json");
    std::fs::read(path).expect("read api/openapi.json")
}

fn spec() -> Value {
    serde_json::from_slice(&spec_bytes()).expect("api/openapi.json is JSON")
}

/// Starts `skald-server` with `args`, stdout discarded and stderr piped.
fn spawn(args: &[&str]) -> Child {
    Command::new(SERVER)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start skald-server")
}

/// A `skald-server` listening on a free local port; killed on drop.
struct Server {
    child: Child,
    /// Kept open so that the server never writes to a closed pipe.
    stderr: BufReader<ChildStderr>,
    base: String,
}

impl Server {
    fn start() -> Self {
        let mut child = spawn(&["--listen", "127.0.0.1:0"]);
        let stderr = BufReader::new(child.stderr.take().expect("stderr is piped"));
        let mut server = Server {
            child,
            stderr,
            base: String::new(),
        };
        let mut line = String::new();
        server
            .stderr
            .read_line(&mut line)
            .expect("read skald-server stderr");
        server.base = line
            .trim_end()
            .strip_prefix("skald-server: listening on ")
            .unwrap_or_else(|| panic!("unexpected skald-server output: {line:?}"))
            .to_owned();
        server
    }

    fn get(&self, path: &str) -> Response {
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .build()
            .new_agent();
        let mut response = agent
            .get(format!("{}{path}", self.base))
            .call()
            .unwrap_or_else(|err| panic!("GET {path}: {err}"));
        Response {
            status: response.status().as_u16(),
            content_type: response
                .headers()
                .get("content-type")
                .map(|value| value.to_str().expect("content type is ASCII").to_owned())
                .unwrap_or_default(),
            body: response.body_mut().read_to_vec().expect("read body"),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Response {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

impl Response {
    fn json(status: u16, body: Value) -> Self {
        Response {
            status,
            content_type: "application/json".to_owned(),
            body: body.to_string().into_bytes(),
        }
    }
}

/// Returns how `response` to `GET <path>` deviates from `spec`: a status or a content type
/// the spec does not list for the operation, or a body that does not match the response schema.
fn spec_violations(spec: &Value, path: &str, response: &Response) -> Vec<String> {
    let documented = format!(
        "/paths/{}/get/responses/{}",
        pointer_token(path),
        response.status
    );
    if spec.pointer(&documented).is_none() {
        return vec![format!(
            "GET {path}: status {} is not in the spec",
            response.status
        )];
    }
    let media_type = response.content_type.split(';').next().unwrap_or("").trim();
    let schema = format!("{documented}/content/{}/schema", pointer_token(media_type));
    if spec.pointer(&schema).is_none() {
        return vec![format!(
            "GET {path} {}: content type {media_type:?} is not in the spec",
            response.status
        )];
    }
    let body: Value = match serde_json::from_slice(&response.body) {
        Ok(body) => body,
        Err(err) => return vec![format!("GET {path}: body is not JSON: {err}")],
    };
    let registry = Registry::new()
        .add(SPEC_URI, spec)
        .expect("register the spec")
        .prepare()
        .expect("index the spec");
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&json!({ "$ref": format!("{SPEC_URI}#{schema}") }))
        .expect("compile the response schema");
    validator
        .iter_errors(&body)
        .map(|err| format!("GET {path}: {err} at {:?}", err.instance_path().as_str()))
        .collect()
}

/// Escapes `token` for a JSON pointer (RFC 6901).
fn pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Waits for `child` to exit, killing it after `timeout`.
fn wait(child: &mut Child, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("wait for skald-server") {
            return status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("skald-server did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn health_conforms_to_spec() {
    let server = Server::start();
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
fn published_spec_matches_repository() {
    let server = Server::start();
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

    let mut with_charset = Response::json(200, ok.clone());
    with_charset.content_type = "application/json; charset=utf-8".to_owned();
    assert_eq!(
        spec_violations(&spec, "/health", &with_charset),
        Vec::<String>::new()
    );

    let mut text = Response::json(200, ok.clone());
    text.content_type = "text/plain".to_owned();
    let not_json = Response {
        body: b"ok".to_vec(),
        ..Response::json(200, Value::Null)
    };
    let outside = [
        ("/missing", Response::json(200, ok.clone())),
        ("/health", Response::json(503, ok)),
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
    let mut child = spawn(&["--listen", &addr]);

    let status = wait(&mut child, Duration::from_secs(30));
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr is piped")
        .read_to_string(&mut stderr)
        .expect("read skald-server stderr");

    assert!(!status.success(), "exit status: {status}");
    assert!(
        stderr.starts_with(&format!("skald-server: cannot listen on {addr}: ")),
        "stderr: {stderr:?}"
    );
}
