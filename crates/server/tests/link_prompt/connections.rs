//! Each connection of the account learns that one of its devices waits for linking, a device
//! without a session once it registers on the account; a connection of another account does not.

use std::time::{Duration, Instant};

use serde_json::json;
use skald_proto::{Cbor as _, Frame, Push};
use tungstenite::Message;

use crate::api::{Person, Session};
use crate::common::device_flow::{DeviceFlow, text};
use crate::common::devices::{DEVICES, Device, assert_taken, b64};
use crate::common::hex;
use crate::kit::{Link, WAIT, fresh_session, hello};
use crate::wire::Socket;

/// How long a test watches a connection to see that nothing comes.
const QUIET: Duration = Duration::from_secs(2);

/// The next binary frame `socket` gets by `deadline`, other messages skipped; `None` if none comes
/// by then. Fails if the connection closes.
fn next_binary(socket: &mut Socket, deadline: Instant) -> Option<Vec<u8>> {
    loop {
        match socket.next(deadline) {
            Ok(Some(Message::Binary(bytes))) => return Some(bytes.to_vec()),
            Ok(Some(Message::Close(frame))) => panic!("the server closed it: {frame:?}"),
            Ok(Some(_)) => {}
            Ok(None) => return None,
            Err(err) => panic!("the connection failed: {err}"),
        }
    }
}

/// A connection of `person`'s certified laptop to `flow`'s server, once its first `sync` has got
/// catch-up up to its end.
#[track_caller]
fn caught_up(flow: &DeviceFlow, person: &Person) -> Socket {
    let mut socket = Socket::of(flow, person);
    socket.sync(0);
    let deadline = Instant::now() + WAIT;
    loop {
        let bytes = next_binary(&mut socket, deadline).expect("no end of catch-up");
        let frame = Frame::from_cbor(&bytes);
        if matches!(frame, Ok(Frame::Batch(pushes)) if pushes.is_empty()) {
            return socket;
        }
    }
}

/// The frame `socket` gets within [`QUIET`], if any.
fn quiet(socket: &mut Socket) -> Option<Vec<u8>> {
    next_binary(socket, Instant::now() + QUIET)
}

/// alice's laptop holds two connections and bob's one, each past catch-up and told nothing. The
/// machine of alice's desktop opens a run of its link code: each of alice's connections gets a
/// frame, and bob's gets none.
#[test]
fn each_connection_of_the_account_learns_of_a_device_waiting_for_linking() {
    let l = Link::start(3_178_100_001);
    let mut alice = [caught_up(&l.flow, &l.alice), caught_up(&l.flow, &l.alice)];
    let mut bob = caught_up(&l.flow, &l.bob);
    for socket in &mut alice {
        assert_eq!(quiet(socket), None, "before the hello");
    }

    l.open();

    for socket in &mut alice {
        let told = next_binary(socket, Instant::now() + WAIT)
            .expect("alice's connection learned nothing of the waiting desktop");
        assert!(
            Frame::from_cbor(&told).is_ok(),
            "not a frame: {}",
            hex(&told)
        );
    }
    assert_eq!(quiet(&mut bob), None, "bob's connection");
}

/// A new device without a session opens a run: alice's connection is told nothing while the device
/// is registered nowhere. Once the device's machine signs in to alice's account and registers it,
/// alice's connection gets a `link-notice`, and bob's gets nothing.
#[test]
fn a_connection_learns_of_a_device_without_a_session_once_it_registers_on_the_account() {
    let l = Link::start(3_178_100_011);
    let mut alice = caught_up(&l.flow, &l.alice);
    let mut bob = caught_up(&l.flow, &l.bob);
    let tablet = Device::new("Tablet");
    let body = json!({
        "device": tablet.id.to_string(),
        "hello": b64(&hello(&fresh_session())),
        "pairing": skald_crypto::LinkPairing::generate().to_string(),
    });
    let opened = Session::none(&l.flow).call("post", "/links", Some(&body));
    assert_eq!(opened.status, 201, "{}", text(&opened));
    assert_eq!(quiet(&mut alice), None, "before the device registers");

    assert_taken(
        &l.new_side
            .call("post", DEVICES, Some(&tablet.registration())),
    );

    let told = next_binary(&mut alice, Instant::now() + WAIT)
        .expect("alice's connection learned nothing of the registered tablet");
    assert!(
        Frame::from_cbor(&told) == Ok(Frame::Push(Push::LinkNotice)),
        "not a link-notice: {}",
        hex(&told)
    );
    assert_eq!(quiet(&mut bob), None, "bob's connection");
}
