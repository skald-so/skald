//! What the server refuses of runs without a session: a hello that does not decode, one of a device
//! that holds a certificate, a pairing it does not hold, a hello as a later message; and a run's
//! token is no session.

use serde_json::json;

use crate::api::assert_refused;
use crate::common::cbor::{array_of, byte_item, uint};
use crate::common::device_flow::text;
use crate::common::devices::{Device, b64};
use crate::kit::{Fresh, OPEN, pair, send_pairing};
use crate::relay::{assert_refused_link, fresh_session, hello};

#[test]
fn a_hello_that_is_not_a_link_hello_opens_no_run() {
    let f = Fresh::start(3_175_300_311);
    let tablet = Device::new("Tablet");
    let session = fresh_session();
    for (bytes, what) in [
        (vec![0xff, 0x00], "bytes that do not decode"),
        (
            array_of(&[uint(2), byte_item(&session), byte_item(&[7; 32])]),
            "a link-reply",
        ),
        (
            array_of(&[uint(1), byte_item(&session[..8]), byte_item(&[7; 32])]),
            "a session of 8 bytes",
        ),
    ] {
        let response = f.send_open(&tablet, &bytes);
        assert_refused(&response, what);
        assert_ne!(response.status, 401, "{what}: {}", text(&response));
    }
    for (body, what) in [
        (json!({ "hello": b64(&hello(&session)) }), "no device"),
        (
            json!({ "device": "not-an-id", "hello": b64(&hello(&session)) }),
            "a device that is no ID",
        ),
        (json!({ "device": tablet.id.to_string() }), "no hello"),
    ] {
        let response = f.nobody().call("post", OPEN, Some(&body));
        assert_refused(&response, what);
        assert_ne!(response.status, 401, "{what}: {}", text(&response));
    }
    // None of them left a run behind: the device's first run opens as any.
    let run = f.open(&tablet);
    assert_eq!(pair(&f.alice.session, &run.pairing), run.hello);
}

#[test]
fn a_hello_of_a_device_that_holds_a_certificate_opens_no_run() {
    let f = Fresh::start(3_175_300_321);
    let response = f.send_open(&f.alice.laptop, &hello(&fresh_session()));
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
