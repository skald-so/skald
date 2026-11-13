//! `skald status`: each row of "Server answers" in `docs/protocol/README.md` against a `skald-server`
//! whose range is set to versions 2–3, and the answers the protocol does not define, from a stub.

#[allow(dead_code)]
mod common;

use std::net::TcpListener;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::{Output, Server, Stub, cleartext, elsewhere, response, skald, skald_with_timeout};
use skald_proto::PROTOCOL_VERSION;

/// The range the server accepts: N = 3, N−1 = 2.
const RANGE: &str = "2-3";

/// A `protocol-too-old` problem details object.
const TOO_OLD: &str = r#"{"type":"https://skald.so/problems/protocol-too-old","title":"Please update skald","status":400,"detail":"This server accepts protocol versions 2 and 3; the request stated 1.","supported":[2,3]}"#;

fn status(url: &str, versions: Option<&str>) -> Output {
    skald(&["status", "--server", url], versions)
}

fn assert_output(output: &Output, success: bool, stdout: &str, stderr: &str) {
    assert_eq!(
        (
            output.success,
            output.stdout.as_str(),
            output.stderr.as_str()
        ),
        (success, stdout, stderr)
    );
}

#[test]
fn current_version_is_compatible() {
    let server = Server::start(Some(RANGE));
    let url = &server.url;

    let output = status(url, Some("3"));

    let stdout = format!("Server {url}: reachable\nProtocol: version 3, compatible\n");
    assert_output(&output, true, &stdout, "");
}

#[test]
fn previous_version_tells_an_update_is_available() {
    let server = Server::start(Some(RANGE));
    let url = &server.url;

    let output = status(url, Some("2"));

    let stdout = format!(
        "Server {url}: reachable\nProtocol: version 2, compatible\n\
         Update available: the server speaks protocol version 3; update skald\n"
    );
    assert_output(&output, true, &stdout, "");
}

#[test]
fn older_version_says_please_update() {
    let server = Server::start(Some(RANGE));
    let url = &server.url;

    for versions in ["1", "1-1"] {
        let output = status(url, Some(versions));

        let stdout = format!("Server {url}: reachable\nProtocol: incompatible\n");
        let stderr = "skald: please update skald: the server accepts protocol versions 2 and 3; \
                      this skald speaks protocol version 1\n";
        assert_output(&output, false, &stdout, stderr);
    }
}

#[test]
fn newer_version_retries_with_a_supported_one() {
    let server = Server::start(Some(RANGE));
    let url = &server.url;

    for versions in ["3-4", "1-9"] {
        let output = status(url, Some(versions));

        let stdout = format!("Server {url}: reachable\nProtocol: version 3, compatible\n");
        assert_output(&output, true, &stdout, "");
    }
}

#[test]
fn newer_version_retries_down_to_the_previous_one() {
    let server = Server::start(Some("2"));
    let url = &server.url;

    let output = status(url, Some("1-3"));

    let stdout = format!("Server {url}: reachable\nProtocol: version 2, compatible\n");
    assert_output(&output, true, &stdout, "");
}

#[test]
fn newer_version_without_a_supported_one_says_the_server_is_older() {
    let server = Server::start(Some(RANGE));
    let url = &server.url;

    let output = status(url, Some("4-5"));

    let stdout = format!("Server {url}: reachable\nProtocol: incompatible\n");
    let stderr = "skald: the server is older than this skald: it accepts protocol versions 2 and 3; \
                  this skald speaks protocol versions 4 and 5\n";
    assert_output(&output, false, &stdout, stderr);
}

#[test]
fn default_versions_are_compatible_with_a_default_server() {
    let server = Server::start(None);
    let url = &server.url;

    let output = status(url, None);

    let stdout =
        format!("Server {url}: reachable\nProtocol: version {PROTOCOL_VERSION}, compatible\n");
    assert_output(&output, true, &stdout, "");
}

#[test]
fn trailing_slash_in_the_url_is_ignored() {
    let server = Server::start(Some(RANGE));
    let url = format!("{}/", server.url);

    let output = status(&url, Some("3"));

    let stdout = format!("Server {url}: reachable\nProtocol: version 3, compatible\n");
    assert_output(&output, true, &stdout, "");
}

/// The client states its version on every request, so a server never answers it with
/// `protocol-version-invalid`; if a server does, the client reports the server's detail. The
/// WebSocket is refused after the HTTPS request is served.
#[test]
fn refusal_of_the_websocket_is_reported() {
    let stub = Stub::start(|path| match path {
        "/protocol" => response("200 OK", "application/json", r#"{"supported":[2,3]}"#),
        _ => response(
            "400 Bad Request",
            "application/problem+json",
            r#"{"type":"https://skald.so/problems/protocol-version-invalid","title":"Missing or invalid protocol version","status":400,"detail":"no version here","supported":[2,3]}"#,
        ),
    });
    let url = &stub.url;

    let output = status(url, Some("3"));

    let stdout = format!("Server {url}: reachable\n");
    let stderr = "skald: the server refused the request with 400: no version here\n";
    assert_output(&output, false, &stdout, stderr);
    let requests = stub.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(
        requests[0].starts_with("get /protocol http/1.1\r\n")
            && requests[0].contains("\r\nskald-protocol-version: 3\r\n"),
        "{:?}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("get /ws http/1.1\r\n")
            && requests[1].contains("\r\nsec-websocket-protocol: skald.3\r\n"),
        "{:?}",
        requests[1]
    );
}

/// Control characters in the server's text reach the terminal escaped.
#[test]
fn control_characters_in_the_detail_are_escaped() {
    let stub = Stub::start(|_| {
        response(
            "400 Bad Request",
            "application/problem+json",
            r#"{"type":"https://skald.so/problems/protocol-version-invalid","detail":"\u001b]0;pwned\u0007\u001b[2Kfake line\nskald: forged"}"#,
        )
    });
    let url = &stub.url;

    let output = status(url, Some("3"));

    let stdout = format!("Server {url}: reachable\n");
    let stderr = r"skald: the server refused the request with 400: \u{1b}]0;pwned\u{7}\u{1b}[2Kfake line\nskald: forged"
        .to_owned()
        + "\n";
    assert_output(&output, false, &stdout, &stderr);
}

/// `skald status` against a stub that accepts the connection and never answers its `requests`th
/// request gives up after its timeout.
fn assert_times_out(stub: &Stub, requests: usize) {
    let url = &stub.url;

    let output = skald_with_timeout(
        &["status", "--server", url, "--protocol-versions", "3"],
        3,
        Duration::from_secs(30),
    );

    let stdout = format!("Server {url}: unreachable\n");
    let stderr = "skald: cannot reach the server: no answer within 3 s\n";
    assert_output(&output, false, &stdout, stderr);
    assert_eq!(stub.requests().len(), requests);
}

#[test]
fn silent_server_times_out_on_the_https_request() {
    assert_times_out(&Stub::start(|_| String::new()), 1);
}

#[test]
fn silent_server_times_out_on_the_websocket() {
    let stub = Stub::start(|path| match path {
        "/protocol" => response("200 OK", "application/json", r#"{"supported":[2,3]}"#),
        _ => String::new(),
    });
    assert_times_out(&stub, 2);
}

#[test]
fn answer_outside_the_protocol_is_reported() {
    let answers: [fn(&str) -> String; 3] = [
        |_| response("404 Not Found", "text/plain", "not found"),
        |_| response("400 Bad Request", "application/problem+json", "not JSON"),
        // A problem's fields count only in a problem details object.
        |_| response("400 Bad Request", "application/json", TOO_OLD),
    ];
    for answer in answers {
        let stub = Stub::start(answer);
        let url = &stub.url;

        let output = status(url, Some("3"));

        let stdout = format!("Server {url}: reachable\n");
        let stderr = "skald: unexpected answer from the server: HTTP ";
        assert!(!output.success);
        assert_eq!(output.stdout, stdout);
        assert!(output.stderr.starts_with(stderr), "{:?}", output.stderr);
    }
}

/// A server that answers `protocol-too-new` while it lists the stated version: the client does not
/// state it again. A second request would get `404`, so a client that repeats it fails at once.
#[test]
fn too_new_listing_the_stated_version_says_the_server_is_older() {
    static ANSWERED: AtomicBool = AtomicBool::new(false);
    let stub = Stub::start(|_| {
        if ANSWERED.swap(true, Ordering::SeqCst) {
            return response("404 Not Found", "text/plain", "not found");
        }
        response(
            "400 Bad Request",
            "application/problem+json",
            r#"{"type":"https://skald.so/problems/protocol-too-new","supported":[2,3]}"#,
        )
    });
    let url = &stub.url;

    let output = status(url, Some("3"));

    let stdout = format!("Server {url}: reachable\nProtocol: incompatible\n");
    let stderr = "skald: the server is older than this skald: it accepts protocol versions 2 and 3; \
                  this skald speaks protocol version 3\n";
    assert_output(&output, false, &stdout, stderr);
    assert_eq!(stub.requests().len(), 1);
}

#[test]
fn closed_port_is_unreachable() {
    let closed = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);

    let output = status(&url, Some("3"));

    assert!(!output.success);
    assert_eq!(output.stdout, format!("Server {url}: unreachable\n"));
    assert!(
        output
            .stderr
            .starts_with("skald: cannot reach the server: "),
        "{:?}",
        output.stderr
    );
}

#[test]
fn url_other_than_https_or_http_is_refused() {
    for url in [
        "ftp://skald.example",
        "ws://localhost:8080",
        "localhost:8080",
    ] {
        let output = status(url, Some("3"));

        let stderr =
            format!("skald: {url:?} is not a server URL the client connects to: use https://\n");
        assert_output(&output, false, "", &stderr);
    }
}

#[test]
fn http_to_another_machine_is_refused_before_any_request() {
    let stub = Stub::start(|_| response("200 OK", "application/json", "{}"));
    let url = elsewhere(&stub);

    let output = status(&url, Some("3"));

    assert_output(&output, false, "", &cleartext(&url));
    assert_eq!(stub.requests(), Vec::<String>::new());
}

#[test]
fn http_to_localhost_reaches_the_server() {
    let server = Server::start(Some(RANGE));
    let port = server.url.rsplit(':').next().expect("the server's port");
    let url = format!("http://localhost:{port}");

    let output = status(&url, Some("3"));

    let stdout = format!("Server {url}: reachable\nProtocol: version 3, compatible\n");
    assert_output(&output, true, &stdout, "");
}

/// A redirect could take a request to another host, in plain HTTP: the client follows none.
#[test]
fn redirect_is_not_followed() {
    static ELSEWHERE: OnceLock<String> = OnceLock::new();
    let elsewhere = Stub::start(|_| response("200 OK", "application/json", "{}"));
    ELSEWHERE
        .set(elsewhere.url.clone())
        .expect("one test sets it");
    let stub = Stub::start(|path| {
        let location = format!(
            "{}{path}",
            ELSEWHERE.get().expect("set before the stub runs")
        );
        format!(
            "HTTP/1.1 307 Temporary Redirect\r\nlocation: {location}\r\ncontent-length: 0\r\n\
             connection: close\r\n\r\n"
        )
    });
    let url = &stub.url;

    let output = status(url, Some("3"));

    let stdout = format!("Server {url}: reachable\n");
    let stderr = "skald: unexpected answer from the server: HTTP 307\n";
    assert_output(&output, false, &stdout, stderr);
    assert_eq!(stub.requests().len(), 1);
    assert_eq!(elsewhere.requests(), Vec::<String>::new());
}

#[test]
fn invalid_protocol_versions_are_refused() {
    for versions in ["0", "3-2", "2-", "-2", "two", "02"] {
        let output = status("http://localhost:8080", Some(versions));

        assert!(!output.success, "{versions:?}");
        assert!(
            output.stdout.is_empty(),
            "{versions:?}: {:?}",
            output.stdout
        );
        assert!(
            output.stderr.contains("SKALD_PROTOCOL_VERSIONS")
                || output.stderr.contains("--protocol-versions"),
            "{versions:?}: {:?}",
            output.stderr
        );
    }
}
