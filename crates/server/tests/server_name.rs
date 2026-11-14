//! The server's name in agent addresses (`docs/protocol/README.md`, "Agent address"): a DNS name
//! the server's operator sets, by default the host of the server's public address. A server whose
//! name would not be a valid server name, such as one without a public address or with an IP
//! address as its host, has none. Every response is checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The setting `--server-name <name>`, or `SKALD_SERVER_NAME`, gives the name in place of the
//! public address's host, also without a public address. One that is not a valid server name stops
//! the server at start, with an error naming the setting.
//!
//! The API, as these tests use it, each request stating the protocol version and without a
//! session:
//! - `GET /server`: `{"name": <server name> | null}`, `null` or no `name` when the server has none.

mod common;

use common::sign_in::SignIn;
use common::spec::{spec, spec_violations};
use common::{Response, Server, TestDb, start_failure};
use serde_json::Value;

/// The request that gives the server's name.
const SERVER_NAME: &str = "/server";

/// The setting, as an argument and as an environment variable.
const SETTING: &str = "--server-name";
const SETTING_ENV: &str = "SKALD_SERVER_NAME";

/// The server's public address setting.
const PUBLIC: &str = "--public-url";

/// `GET /server` at `server`, stating the protocol version; checked against the spec. Gives the
/// name, or `None` when the server has none.
fn name_of(server: &Server) -> Option<String> {
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let response = server.get_with(SERVER_NAME, &[(skald_proto::VERSION_HEADER, &version)]);
    assert_eq!(
        spec_violations(&spec(), SERVER_NAME, &response),
        Vec::<String>::new()
    );
    assert_eq!(response.status, 200, "{}", text(&response));
    let body: Value = serde_json::from_slice(&response.body).expect("a JSON body");
    match &body["name"] {
        Value::Null => None,
        Value::String(name) => Some(name.clone()),
        other => panic!("GET {SERVER_NAME}: name {other}"),
    }
}

fn text(response: &Response) -> String {
    String::from_utf8_lossy(&response.body).into_owned()
}

/// The name of a server started with the arguments `args` and the environment variables `envs`.
fn name_with(args: &[&str], envs: &[(&str, &str)]) -> Option<String> {
    let db = TestDb::create();
    let server = Server::launch(&db, "127.0.0.1:0", args, envs);
    name_of(&server)
}

/// A PostgreSQL URL where nothing listens: a server whose settings are taken exits at once too,
/// but with another error.
const CLOSED_DATABASE: &str = "postgres://skald@127.0.0.1:1/skald";

/// Starts a server with `args` and `envs` on a database it cannot reach; gives what it printed.
fn start_output(args: &[&str], envs: &[(&str, &str)]) -> String {
    let mut all = vec!["--listen", "127.0.0.1:0", "--database-url", CLOSED_DATABASE];
    all.extend_from_slice(args);
    let (started, output) = common::run(&all, envs);
    assert!(!started, "{args:?}: {output}");
    output
}

/// Without the setting, the name is the public address's host, in A-label form for an
/// internationalized one, with no port; a server with GitHub sign-in at `http://skald.test:<port>`
/// is `skald.test`.
#[test]
fn name_is_the_public_address_host_by_default() {
    for (public, name) in [
        ("http://localhost:8080", "localhost"),
        ("https://skald.example", "skald.example"),
        ("https://api.skald.example:8443", "api.skald.example"),
        ("https://bücher.example", "xn--bcher-kva.example"),
    ] {
        assert_eq!(
            name_with(&[PUBLIC, public], &[]).as_deref(),
            Some(name),
            "{public}"
        );
    }

    let stand = SignIn::start();
    assert_eq!(name_of(&stand.server).as_deref(), Some("skald.test"));
}

/// The setting replaces the public address's host, as an argument or from the environment, and
/// names the server also without a public address.
#[test]
fn setting_replaces_the_public_address_host() {
    let public = "https://cloud.skald-hosting.example";
    assert_eq!(
        name_with(&[PUBLIC, public, SETTING, "skald.example"], &[]).as_deref(),
        Some("skald.example")
    );
    assert_eq!(
        name_with(&[PUBLIC, public], &[(SETTING_ENV, "skald.example")]).as_deref(),
        Some("skald.example")
    );
    assert_eq!(
        name_with(&[SETTING, "localhost"], &[]).as_deref(),
        Some("localhost")
    );
    assert_eq!(
        name_with(
            &[PUBLIC, "http://127.0.0.1:8080", SETTING, "skald.example"],
            &[]
        )
        .as_deref(),
        Some("skald.example")
    );

    let stand = SignIn::start_with(&[SETTING, "skald.example"], common::sign_in::CLIENT_SECRET);
    assert_eq!(name_of(&stand.server).as_deref(), Some("skald.example"));
}

/// Without a public address or the setting, or with an IP address as the public address's host,
/// the server has no name; it runs and its health check answers ok.
#[test]
fn without_a_valid_name_the_server_has_none() {
    for args in [
        vec![],
        vec![PUBLIC, "http://127.0.0.1:8080"],
        vec![PUBLIC, "http://[::1]:8080"],
    ] {
        let db = TestDb::create();
        let server = Server::start_with(&db, &args);
        assert_eq!(name_of(&server), None, "{args:?}");
        server.assert_healthy();
    }
}

/// A setting that is not a valid server name stops the server at start, as an argument or from
/// the environment, with an error naming the setting; a valid one does not.
#[test]
fn invalid_setting_stops_the_server_at_start() {
    let long_label = format!("{}.example", "a".repeat(64));
    let too_long = vec!["a".repeat(63); 3].join(".") + "." + &"a".repeat(62);
    assert_eq!(too_long.len(), 254);
    let invalid = [
        "",
        "127.0.0.1",
        "10.0.0.1",
        "::1",
        "[::1]",
        "Skald.Example",
        "skald.example.",
        "skald.example:443",
        "https://skald.example",
        "skald..example",
        ".skald.example",
        "-skald.example",
        "skald-.example",
        "skald_test.example",
        "skald example",
        "bücher.example",
        "skald.123",
        long_label.as_str(),
        too_long.as_str(),
    ];

    let valid = start_output(&[SETTING, "skald.example"], &[]);
    assert!(!valid.contains(SETTING), "a valid name is refused: {valid}");
    let valid = start_output(&[], &[(SETTING_ENV, "skald.example")]);
    assert!(
        !valid.contains(SETTING) && !valid.contains(SETTING_ENV),
        "a valid name is refused: {valid}"
    );

    for name in invalid {
        let stderr = start_failure(&[
            "--listen",
            "127.0.0.1:0",
            "--database-url",
            CLOSED_DATABASE,
            &format!("{SETTING}={name}"),
        ]);
        assert!(stderr.contains(SETTING), "{name:?}: {stderr}");
        assert!(
            !stderr.contains("unexpected argument"),
            "{name:?}: {stderr}"
        );
    }
    for name in invalid.iter().filter(|name| !name.is_empty()) {
        let output = start_output(&[], &[(SETTING_ENV, name)]);
        assert!(
            output.contains(SETTING) || output.contains(SETTING_ENV),
            "{name:?}: {output}"
        );
    }
}
