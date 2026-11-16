//! Error reporting (`docs/dev/infrastructure.md`, "Monitoring"): the server reports its errors to
//! Sentry only when a setting gives a DSN, and its reports carry no request body, token, secret or
//! envelope content. Against a `skald-server` on PostgreSQL and a stand-in for Sentry's ingestion
//! endpoint on a local port.
//!
//! The interfaces, as these tests use them:
//! - The setting `--sentry-dsn <DSN>`, also `SKALD_SENTRY_DSN`. Unset, the server sends nothing,
//!   also with `SENTRY_DSN`, the Sentry SDKs' own variable, set.
//! - A DSN `http://<key>@<host>:<port>/<project>`: the server sends its reports there, as Sentry's
//!   SDKs do, to `/api/<project>/envelope/` or `/api/<project>/store/`, naming `<key>`.
//! - A server error: a request the server answers `500` while its database is down, or a
//!   connection it closes with code 1011 and the reason `server-error`.
//! - A report of a request's error: an event, of the level `error` or `fatal` if it states one,
//!   that names the request's path, within 30 seconds of the answer.

#[path = "../common/mod.rs"]
mod common;

// The fixture and frames of the tests of storing sent envelopes; this binary uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

mod sentry;

// The spaces tests' accounts and revisions, which the fixture builds on.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

mod background;
mod contents;
mod setting;

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

use common::devices::random;
use common::{Response, Server, hex, send};
use sentry::Report;

/// The setting's environment variable and flag.
pub const DSN_ENV: &str = "SKALD_SENTRY_DSN";
pub const DSN_FLAG: &str = "--sentry-dsn";

/// How long a report may take to arrive.
pub const WITHIN: Duration = Duration::from_secs(30);
/// How long the tests wait for further reports once the awaited ones came.
pub const AFTER: Duration = Duration::from_secs(3);

/// The path of the request [`fail_a_request`] sends.
pub const USERNAME: &str = "/account/username";

/// A new secret: 32 random bytes in unpadded Base64url, as the server's tokens are.
pub fn secret() -> String {
    URL_SAFE_NO_PAD.encode(random())
}

/// The headers of a JSON API request stating this protocol version, with `extra`.
pub fn api_headers<'a>(version: &'a str, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut headers = vec![(skald_proto::VERSION_HEADER, version)];
    headers.extend_from_slice(extra);
    headers
}

/// `PUT /account/username` to `server` with the bearer token `token` and the JSON body `body`,
/// which the server answers `500` while its database is down.
pub fn fail_a_request(server: &Server, token: &str, body: &[u8]) -> Response {
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let bearer = format!("Bearer {token}");
    let headers = api_headers(
        &version,
        &[
            ("authorization", &bearer),
            ("content-type", "application/json"),
        ],
    );
    let response = send(
        "put",
        &format!("{}{USERNAME}", server.base()),
        &headers,
        body,
    );
    assert_failed(&response);
    response
}

/// Checks that the server answered `response` with `500`.
#[track_caller]
pub fn assert_failed(response: &Response) {
    assert_eq!(
        response.status,
        500,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
}

/// Whether `reports` hold a report of an error of a request to `path` for the project `project`
/// of the DSN whose public key is `key`.
pub fn reported(reports: &[Report], key: &str, project: u32, path: &str) -> bool {
    reports
        .iter()
        .filter(|report| report.is_for(key, project))
        .flat_map(Report::events)
        .any(|event| {
            let level = event["level"].as_str();
            matches!(level, None | Some("error" | "fatal")) && event.to_string().contains(path)
        })
}

/// The forms in which a report could hold `bytes`: as they are, in hex and in Base64.
pub fn forms(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut forms = vec![
        bytes.to_vec(),
        hex(bytes).into_bytes(),
        hex(bytes).to_uppercase().into_bytes(),
    ];
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        forms.push(engine.encode(bytes).into_bytes());
    }
    // A text in lowercase too, and the bytes a Base64 text encodes.
    if let Ok(text) = std::str::from_utf8(bytes) {
        forms.push(text.to_lowercase().into_bytes());
        for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
            if let Ok(decoded) = engine.decode(text)
                && decoded.len() >= 8
            {
                forms.push(decoded.clone());
                forms.push(hex(&decoded).into_bytes());
            }
        }
    }
    forms
}

/// Checks that the stand-in could read each of `reports` and that none holds any of `secrets`, by
/// name, in a form of [`forms`].
#[track_caller]
pub fn assert_hold_none(reports: &[Report], secrets: &[(&str, &[u8])]) {
    for report in reports {
        assert_eq!(
            report.unread, None,
            "a report in an encoding the stand-in cannot read: {} {}",
            report.method, report.target
        );
        let bytes = report.bytes();
        for (name, secret) in secrets {
            for form in forms(secret) {
                assert!(
                    !bytes.windows(form.len()).any(|window| window == form),
                    "a report holds {name}: {} {}\n{}",
                    report.method,
                    report.target,
                    String::from_utf8_lossy(&bytes)
                );
            }
        }
    }
}
