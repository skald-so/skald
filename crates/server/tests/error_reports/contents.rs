//! Reports carry no request body, token, secret or envelope content.

use std::thread;
use std::time::Instant;

use skald_proto::{Cbor, Envelope, HpkePayload};
use tungstenite::Message;
use tungstenite::protocol::frame::coding::CloseCode;

use crate::common::devices::random;
use crate::common::sign_in::{CLIENT_SECRET, SignIn};
use crate::common::{Closed, TestDb, get_url, hex, post_form};
use crate::fixture::Fixture;
use crate::sentry::Sentry;
use crate::wire::{self, Socket};
use crate::{
    AFTER, DSN_ENV, USERNAME, WITHIN, api_headers, assert_failed, assert_hold_none, fail_a_request,
    reported, secret,
};

const KEY: &str = "contentkey3b4c5d6e";

/// Requests with tokens in `Authorization` and in cookies, a body, a form with a device code and a
/// user code in the query, each failing while the database is down, are reported without any of
/// them, and without the server's client secret.
#[test]
fn a_report_holds_no_token_body_or_secret() {
    let sentry = Sentry::start();
    let dsn = sentry.dsn(KEY, 11);
    let stand = SignIn::on_with(TestDb::create(), &[], &[(DSN_ENV, &dsn)]);
    let base = stand.server.base().to_owned();
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let bearer = secret();
    let marker = format!("body{}", hex(&random()[..8]));
    let body = format!(r#"{{"username":"{marker}"}}"#);
    let session_cookie = secret();
    let device_code = secret();
    let page_cookie = secret();
    let user_code: String = random()[..8]
        .iter()
        .map(|byte| char::from(b'A' + byte % 26))
        .collect();
    stand.db.set_reachable(false);

    fail_a_request(&stand.server, &bearer, body.as_bytes());
    let cookie = format!("skald_session={session_cookie}");
    let session = get_url(
        &format!("{base}/session"),
        &api_headers(&version, &[("cookie", &cookie)]),
    );
    assert_failed(&session);
    let token = post_form(
        &format!("{base}/auth/token"),
        &api_headers(&version, &[]),
        &[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &device_code),
            ("client_id", "skald"),
        ],
    );
    assert_failed(&token);
    let cookie = format!("skald_session={page_cookie}");
    let page = get_url(
        &format!("{base}/device?user_code={user_code}"),
        &[("cookie", &cookie)],
    );
    assert_failed(&page);

    sentry.wait_until(WITHIN, |reports| {
        [USERNAME, "/session", "/auth/token", "/device"]
            .iter()
            .all(|path| reported(reports, KEY, 11, path))
    });
    thread::sleep(AFTER);

    assert_hold_none(
        &sentry.reports(),
        &[
            ("the bearer token", bearer.as_bytes()),
            ("the body", body.as_bytes()),
            ("the body's username", marker.as_bytes()),
            ("the session cookie", session_cookie.as_bytes()),
            ("the device code", device_code.as_bytes()),
            ("the page's session cookie", page_cookie.as_bytes()),
            ("the user code", user_code.as_bytes()),
            ("the client secret", CLIENT_SECRET.as_bytes()),
        ],
    );
}

/// A `send` the server fails to store closes its connection with `server-error`, which is reported
/// naming `/ws`; the reports that follow hold nothing of its envelope.
#[test]
fn a_report_holds_no_envelope_content() {
    let sentry = Sentry::start();
    let dsn = sentry.dsn(KEY, 12);
    let fixture = Fixture::new(3_121_480_001);
    let t = &fixture.team;
    let reporting = t.flow.stand.instance(&[], &[(DSN_ENV, &dsn)]);
    let mut alice = Socket::to(&reporting, &t.flow, &t.alice, &t.alice.laptop);
    // A first envelope stored: the connection is served, so the database's failure, not the wait
    // for its sign-in, closes it.
    let first = fixture.to(t.bob.laptop.id);
    alice.store(&first, &wire::envelope(&first));
    let mut ct = random().to_vec();
    ct.extend_from_slice(&random());
    let envelope = Envelope {
        header: fixture.to(t.bob.laptop.id),
        payload: HpkePayload {
            enc: random(),
            ct: ct.clone(),
        }
        .to_cbor(),
    }
    .to_cbor();
    fixture.db().set_reachable(false);

    alice.send(&envelope);

    let deadline = Instant::now() + WITHIN;
    let closed = loop {
        match alice.next(deadline) {
            Ok(Some(Message::Close(frame))) => break Closed::of(frame),
            Ok(Some(_)) => {}
            Ok(None) => panic!("still open after {WITHIN:?}"),
            Err(err) => panic!("closed without a close frame: {err}"),
        }
    };
    assert_eq!(
        closed,
        Closed::Frame(CloseCode::Error, "server-error".to_owned())
    );
    sentry.wait_until(WITHIN, |reports| reported(reports, KEY, 12, "/ws"));
    thread::sleep(AFTER);

    assert_hold_none(
        &sentry.reports(),
        &[
            ("the envelope", &envelope),
            ("the envelope's ciphertext", &ct),
            ("the start of the envelope's ciphertext", &ct[..24]),
            ("the client secret", CLIENT_SECRET.as_bytes()),
        ],
    );
}
