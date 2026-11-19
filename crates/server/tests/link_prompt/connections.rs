//! Each connection of the account learns that one of its devices waits for linking; a connection of
//! another account does not.

use std::time::{Duration, Instant};

use skald_proto::{Cbor as _, Frame};
use tungstenite::Message;

use crate::api::Person;
use crate::common::device_flow::DeviceFlow;
use crate::common::hex;
use crate::kit::{Link, WAIT};
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
