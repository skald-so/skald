//! `skald status`: each row of "Server answers" in `docs/protocol/README.md` against a `skald-server`
//! whose range is set to versions 2–3, and the answers the protocol does not define, from a stub.

mod common;

use std::net::TcpListener;

use common::{Output, Server, Stub, response, skald};
use skald_proto::PROTOCOL_VERSION;

/// The range the server accepts: N = 3, N−1 = 2.
const RANGE: &str = "2-3";

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

#[test]
fn answer_outside_the_protocol_is_reported() {
    let answers: [fn(&str) -> String; 2] = [
        |_| response("404 Not Found", "text/plain", "not found"),
        |_| response("400 Bad Request", "application/problem+json", "not JSON"),
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
fn url_other_than_http_is_refused() {
    for url in [
        "https://skald.example",
        "ws://localhost:8080",
        "localhost:8080",
    ] {
        let output = status(url, Some("3"));

        let stderr = format!(
            "skald: {url:?} is not a server URL the client connects to: it must start with http://\n"
        );
        assert_output(&output, false, "", &stderr);
    }
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
