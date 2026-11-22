//! Refused and not stored: a hello naming a device with a certificate, of another account or that
//! the server does not have; malformed hellos and later messages; a later message naming a run the
//! server does not hold. Without a session, every link request is refused with `401`.

use serde_json::json;

use crate::api::Session;
use crate::common::cbor::{array_of, byte_item, uint};
use crate::common::device_flow::text;
use crate::common::devices::{Device, assert_taken, b64, random};
use crate::kit::{
    HELLO, LINKS, Link, MESSAGES, WAITING, Waiting, abort, assert_refused_link, fresh_session,
    hello, reply,
};

/// Checks that alice's account holds no run: no hello and no waiting device.
#[track_caller]
fn assert_no_run(l: &Link, what: &str) {
    assert_eq!(l.alice.session.hello(), None, "{what}: no hello");
    assert_eq!(
        l.alice.session.waiting(),
        Vec::new(),
        "{what}: nothing waits"
    );
}

#[test]
fn a_hello_naming_a_device_with_a_certificate_or_of_another_account_is_refused() {
    let l = Link::start(3_170_300_211);
    let unknown = Device::new("Unknown");
    for (what, device) in [
        ("alice's certified laptop", l.alice.laptop.id),
        ("bob's device", l.bob.uncertified.id),
        ("bob's certified laptop", l.bob.laptop.id),
        ("a device the server does not have", unknown.id),
    ] {
        let response = l.new_side.send_hello(device, &hello(&fresh_session()));
        assert_refused_link(&response, what);
        assert_no_run(&l, what);
    }
    assert_eq!(l.bob.session.hello(), None, "bob's account holds no run");
    assert_eq!(l.bob.session.waiting(), Vec::new());

    // A hello naming alice's desktop opens a run.
    let (_, sent) = l.open();
    assert_eq!(l.alice.session.hello(), Some(sent));
}

#[test]
fn malformed_hellos_are_refused() {
    let l = Link::start(3_170_300_221);
    let session = fresh_session();
    let device = l.desktop().to_string();
    let share = byte_item(&random());
    let id = byte_item(&session);
    let encoded = |items: &[Vec<u8>]| b64(&array_of(items));
    let cases = [
        (
            "a reply",
            json!({ "device": device, "hello": b64(&reply(&session)) }),
        ),
        (
            "an abort",
            json!({ "device": device, "hello": b64(&abort(&session)) }),
        ),
        (
            "bytes that are not CBOR",
            json!({ "device": device, "hello": b64(&[0xff, 0x00]) }),
        ),
        (
            "a share of 31 bytes",
            json!({ "device": device, "hello": encoded(&[uint(1), id.clone(), byte_item(&[7; 31])]) }),
        ),
        (
            "a session of 15 bytes",
            json!({ "device": device, "hello": encoded(&[uint(1), byte_item(&[7; 15]), share.clone()]) }),
        ),
        (
            "an extra item",
            json!({ "device": device, "hello": encoded(&[uint(1), id.clone(), share.clone(), uint(0)]) }),
        ),
        (
            "a hello that is not base64url",
            json!({ "device": device, "hello": "a hello!" }),
        ),
        ("no hello", json!({ "device": device })),
        (
            "no device",
            json!({ "hello": encoded(&[uint(1), id.clone(), share.clone()]) }),
        ),
        (
            "a device that is not an ID",
            json!({ "device": "desktop", "hello": encoded(&[uint(1), id.clone(), share.clone()]) }),
        ),
    ];
    for (what, body) in cases {
        assert_refused_link(&l.new_side.send_hello_body(&body), what);
        assert_no_run(&l, what);
    }

    // The same hello, well formed, is taken.
    let body = json!({ "device": device, "hello": encoded(&[uint(1), id, share]) });
    assert_taken(&l.new_side.send_hello_body(&body));
    assert_eq!(
        l.alice.session.waiting(),
        vec![Waiting::of(&l.alice.uncertified)]
    );
}

#[test]
fn malformed_later_messages_and_messages_of_runs_the_server_does_not_hold_are_refused() {
    let l = Link::start(3_170_300_231);
    let (session, _) = l.open();
    let id = byte_item(&session);
    let encoded = |items: &[Vec<u8>]| b64(&array_of(items));
    let cases = [
        (
            "bytes that are not CBOR",
            json!({ "message": b64(&[0xff, 0x00]) }),
        ),
        (
            "an unknown type",
            json!({ "message": encoded(&[uint(6), id.clone(), byte_item(&random())]) }),
        ),
        (
            "a reply with a share of 31 bytes",
            json!({ "message": encoded(&[uint(2), id.clone(), byte_item(&[7; 31])]) }),
        ),
        (
            "a grant whose sealed is a number",
            json!({ "message": encoded(&[uint(4), id.clone(), uint(7)]) }),
        ),
        (
            "an abort with an extra item",
            json!({ "message": encoded(&[uint(5), id.clone(), uint(0)]) }),
        ),
        (
            "a message that is not base64url",
            json!({ "message": "a reply!" }),
        ),
        ("no message", json!({})),
        (
            "a reply of another run",
            json!({ "message": b64(&reply(&fresh_session())) }),
        ),
        (
            "an abort of another run",
            json!({ "message": b64(&abort(&fresh_session())) }),
        ),
    ];
    for (what, body) in cases {
        let refused = l.alice.session.send_message_body(&body);
        assert_refused_link(&refused, what);
    }
    assert_eq!(
        l.new_side.messages(&session),
        Vec::<Vec<u8>>::new(),
        "the run holds no refused message"
    );
    assert_eq!(
        l.alice.session.waiting(),
        vec![Waiting::of(&l.alice.uncertified)],
        "the run stays open"
    );

    // A well-formed reply is taken.
    let reply = reply(&session);
    assert_taken(&l.alice.session.send_message(&reply));
    assert_eq!(l.new_side.messages_with(&session, &reply), vec![reply]);
}

#[test]
fn without_a_session_each_link_request_is_refused() {
    let l = Link::start(3_170_300_241);
    let (session, _) = l.open();
    let none = Session::none(&l.flow);
    let hello = json!({ "device": l.desktop().to_string(), "hello": b64(&hello(&session)) });
    let message = json!({ "message": b64(&reply(&session)) });
    for (method, path, body) in [
        ("post", LINKS.to_owned(), Some(&hello)),
        ("get", HELLO.to_owned(), None),
        ("post", MESSAGES.to_owned(), Some(&message)),
        ("get", format!("{MESSAGES}/{}", b64(&session)), None),
        ("get", WAITING.to_owned(), None),
    ] {
        let response = none.call(method, &path, body);
        assert_eq!(response.status, 401, "{method} {path}: {}", text(&response));
    }
    assert_eq!(l.new_side.messages(&session), Vec::<Vec<u8>>::new());
}
