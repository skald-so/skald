//! What the server refuses of runs without a session: a request that names no device or names a
//! pairing, a first message that is not a hello, a run of a device that holds a certificate, a
//! pairing it does not hold or whose run has no hello yet, a second hello; and a run's token is no
//! session.

use serde_json::json;

use crate::api::assert_refused;
use crate::common::cbor::{array_of, byte_item, uint};
use crate::common::device_flow::text;
use crate::common::devices::{Device, b64};
use crate::kit::{Fresh, OPEN, Run, pair, send_pairing};
use crate::relay::{abort, assert_refused_link, fresh_session, hello};

#[test]
fn a_request_that_names_no_device_or_a_pairing_opens_no_run() {
    let f = Fresh::start(3_175_300_311);
    let tablet = Device::new("Tablet");
    let device = tablet.id.to_string();
    for (body, what) in [
        (json!({}), "no device"),
        (json!({ "device": "not-an-id" }), "a device that is no ID"),
        (
            json!({ "device": device, "pairing": "PQRSTU" }),
            "a pairing the device picked",
        ),
        (
            json!({ "device": device, "hello": b64(&hello(&fresh_session())) }),
            "a hello in the request",
        ),
    ] {
        let response = f.nobody().call("post", OPEN, Some(&body));
        assert_refused(&response, what);
        assert_ne!(response.status, 401, "{what}: {}", text(&response));
    }
    // None of them left a run behind: the device's first run opens as any.
    let run = f.open(&tablet);
    assert_eq!(pair(&f.alice.session, &run.pairing), run.hello);
}

/// Until its hello comes, a run takes no other message and no account pairs it; a first message
/// that is not a hello leaves it waiting for one.
#[test]
fn a_run_takes_its_hello_first() {
    let f = Fresh::start(3_175_300_316);
    let (pairing, side) = f.open_without_hello(&Device::new("Tablet"));
    let session = fresh_session();
    let run = Run {
        session,
        hello: hello(&session),
        pairing,
        side,
    };
    assert_refused_link(
        &send_pairing(&f.alice.session, &run.pairing),
        "a run without its hello",
    );
    for (bytes, what) in [
        (vec![0xff, 0x00], "bytes that do not decode"),
        (
            array_of(&[uint(2), byte_item(&session), byte_item(&[7; 32])]),
            "a link-reply",
        ),
        (
            array_of(&[uint(1), byte_item(&session[..8]), byte_item(&[7; 32])]),
            "a hello with a session of 8 bytes",
        ),
        (abort(&session), "a link-abort"),
    ] {
        let response = run.send(&bytes);
        assert_refused(&response, what);
        assert_ne!(response.status, 401, "{what}: {}", text(&response));
    }
    assert_eq!(run.messages(), Vec::<Vec<u8>>::new(), "nothing relayed");
    let sent = run.send(&run.hello);
    assert_eq!(sent.status, 201, "the hello: {}", text(&sent));
    assert_eq!(pair(&f.bob.session, &run.pairing), run.hello);
}

/// Each run gets a pairing of its own, which the server picks.
#[test]
fn each_run_has_a_pairing_of_its_own() {
    let f = Fresh::start(3_175_300_318);
    let pairings: Vec<String> = (0..8)
        .map(|n| f.open(&Device::new(&format!("Tablet {n}"))).pairing)
        .collect();
    for pairing in &pairings {
        assert!(
            pairing.len() == 6
                && pairing
                    .bytes()
                    .all(|c| matches!(c, b'A'..=b'Z' | b'2'..=b'7')),
            "not a pairing: {pairing:?}"
        );
    }
    let mut distinct = pairings.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), pairings.len(), "{pairings:?}");
}

#[test]
fn a_hello_of_a_device_that_holds_a_certificate_opens_no_run() {
    let f = Fresh::start(3_175_300_321);
    let response = f.send_open(&f.alice.laptop);
    assert_refused(&response, "alice's laptop, certified");
    assert_ne!(response.status, 401, "{}", text(&response));
}

#[test]
fn a_pairing_the_server_does_not_hold_gives_no_hello() {
    let f = Fresh::start(3_175_300_331);
    let run = f.open(&Device::new("Tablet"));
    for pairing in ["", "AAAA", "not a pairing", &format!("{}X", run.pairing)] {
        assert_refused_link(
            &send_pairing(&f.alice.session, pairing),
            &format!("the pairing {pairing:?}"),
        );
    }
    let response = send_pairing(&f.nobody(), &run.pairing);
    assert_eq!(
        response.status,
        401,
        "without a session: {}",
        text(&response)
    );
    assert_eq!(
        pair(&f.alice.session, &run.pairing),
        run.hello,
        "the run's own pairing still pairs"
    );
}

#[test]
fn a_hello_is_no_later_message_of_a_run() {
    let f = Fresh::start(3_175_300_341);
    let run = f.open(&Device::new("Tablet"));
    pair(&f.alice.session, &run.pairing);
    let response = run.send(&hello(&run.session));
    assert_refused(&response, "a hello on the run");
    assert_ne!(response.status, 401, "{}", text(&response));
    assert_eq!(run.messages(), Vec::<Vec<u8>>::new(), "nothing relayed");
}

#[test]
fn a_runs_token_is_no_session() {
    let f = Fresh::start(3_175_300_351);
    let run = f.open(&Device::new("Tablet"));
    pair(&f.alice.session, &run.pairing);
    for path in [
        "/session",
        "/devices",
        "/account/master",
        "/account/links/hello",
        "/account/links/waiting",
    ] {
        let response = run.side.call("get", path, None);
        assert_eq!(response.status, 401, "GET {path}: {}", text(&response));
    }
    let response = run.side.call("post", "/session/challenge", None);
    assert_eq!(response.status, 401, "a challenge: {}", text(&response));
    let registered = run.side.call(
        "post",
        crate::common::devices::DEVICES,
        Some(&Device::new("Phone").registration()),
    );
    assert_eq!(
        registered.status,
        401,
        "a registration: {}",
        text(&registered)
    );
}
