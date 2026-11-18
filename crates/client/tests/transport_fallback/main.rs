//! `skald` in one-shot mode behind a proxy that blocks WebSocket or streaming
//! (`docs/dev/architecture.md`, "Network"; `docs/protocol/delivery.md`, "Frames", "Mailbox" and
//! "Gap detection"). The client's connection tries WebSocket, then the mailbox's event stream over
//! SSE, then long-poll, each when the one before does not open or does not end its catch-up in
//! time; over SSE and long-poll it sends and acks over HTTPS. Over long-poll, a run that exits at
//! the end of catch-up asks with a wait of 0 until an answer is empty, and it sends a channel's
//! envelopes one at a time in `counter` order, each after the previous one is answered. Every
//! command that connects sends, receives, acks and shows the same over each transport.
//!
//! The machines reach the server at `https://skald.test:<port>`, which only their proxy, the one
//! `HTTPS_PROXY` names, reaches: the recording proxy of the tests of `HTTPS_PROXY`
//! (`https_proxy/proxy.rs`). It intercepts TLS: it tunnels to the TLS endpoint of the tests of
//! HTTPS (`https/front.rs`), with a certificate of a test CA the machines trust through
//! `SSL_CERT_FILE`, which forwards what it reads inside TLS to the filter of `filter.rs`, then to
//! the recording relay and `skald-server`. The filter refuses WebSocket upgrades and the event
//! stream, or holds the stream back until it ends, as such a proxy would, and tells which transport
//! each run used by the paths of the OpenAPI spec: `GET /ws`, `GET /mailbox/events`,
//! `POST /mailbox/poll` and `POST /mailbox/send`.
//!
//! As in the tests of `HTTPS_PROXY`, the server has GitHub sign-in through the GitHub stand-in, a
//! test plays the person in headless Chrome, and the CLI runs on Ada's machine, which creates the
//! space; Bob's device is played by the tests over WebSocket at the relay, with an MLS state of its
//! own, as in the tests of channel messages.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod behind;
mod filter;

// The channel of the tests of channel messages, with the kit of channel groups and the device they
// play.
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// What these need: the server's tests' readers of the KeyPackages API's answers, the front and
// fixtures of the tests of sending, and the peer of the tests of receiving.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;

// The person's browser, and time passing as the server's database shows it.
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../../../server/tests/common/times.rs"]
mod times;

// The server, the relay and the machines of the tests of signing in, and people in spaces.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The recording proxy of the tests of `HTTPS_PROXY`, and the test CA and TLS endpoint of the tests
// of HTTPS.
#[path = "../https_proxy/proxy.rs"]
mod proxy;
#[path = "../common/tls.rs"]
mod tls;
#[path = "../https/front.rs"]
mod tls_front;

// Finding keys in bytes, and the helpers of every client test.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

use std::thread;
use std::time::{Duration, Instant};

use behind::{
    Behind, Forked, TIMEOUT, TRANSPORTS, Transport, answered, assert_over, assert_sent_over, first,
    recv,
};
use cloud::WAIT;
use filter::{EVENTS, Fate, POLL, SEND, Seen, Stream, WEBSOCKET};
use fixtures::{await_running, send_args};
use kit::RECV;
use machine::Finished;
use skald_proto::{Cbor as _, Frame, Push};
use talk::delivered;
use world::{devices_request, store};

/// The request timeout of Ada's runs, in seconds, and the server's longest long-poll wait, in the
/// test of runs that exit at the end of catch-up: as long as by default, so that a run that waits
/// out either instead of exiting takes far longer than [`AT_ONCE`].
const LONG_TIMEOUT: &str = "30";
const LONG_POLL_WAIT: Duration = Duration::from_secs(30);

/// How soon a run whose catch-up ends at once exits, whatever the load on the machine.
const AT_ONCE: Duration = Duration::from_secs(5);

/// Fails unless the client tried the transports up to `transport` in order, each before the next,
/// in `seen`, what the proxy saw of a run.
#[track_caller]
fn assert_tried_in_order(seen: &[Seen], transport: Transport) {
    let tries = [
        first(seen, "GET", WEBSOCKET),
        first(seen, "GET", EVENTS),
        first(seen, "POST", POLL),
    ];
    let upto = TRANSPORTS.iter().position(|one| *one == transport).unwrap() + 1;
    let tried: Vec<usize> = (tries[..upto].iter())
        .map(|at| {
            at.unwrap_or_else(|| panic!("not every transport up to {transport:?} tried: {seen:#?}"))
        })
        .collect();
    assert!(
        tried.windows(2).all(|two| two[0] < two[1]),
        "not tried in order up to {transport:?}: {seen:#?}"
    );
}

/// Ada, behind the proxy that leaves `transport` working and `stream` done to the event stream,
/// receives with an empty mailbox, sends a message, which Bob's device reads once, and receives
/// one of Bob's, once, acking it; each run goes over `transport` after trying the ones before it.
fn works_over(behind: &mut Behind, transport: Transport, stream: Stream) {
    behind.filter.block(true, stream);
    let (empty, seen) = behind.ada_recv();
    assert!(empty.success, "{empty:?}");
    assert_tried_in_order(&seen, transport);
    assert_over(&seen, transport, &empty);
    let says = format!("ada over {transport:?}: the build is green");
    behind.ada_message_reaches_bob(transport, (&says, 0), |_| {});
    let answer = format!("bob over {transport:?}: merging now");
    behind.bob_message_reaches_ada(transport, &answer, |_| {});
}

/// With WebSocket upgrades refused by the proxy, `skald send` and `skald recv` work over SSE,
/// after trying WebSocket: with an empty mailbox, sending a message Bob's device reads once, and
/// receiving and acking one of Bob's.
#[test]
fn send_and_recv_work_over_sse_when_the_proxy_refuses_websocket() {
    let mut behind = Behind::start(
        [4_144_041_001, 4_144_041_002],
        ["ada-sse-fallback", "bob-sse-fallback"],
    );
    works_over(&mut behind, Transport::Sse, Stream::Forward);
}

/// With WebSocket upgrades refused and the event stream held back by the proxy until its response
/// ends, `skald send` and `skald recv` work over long-poll, after trying WebSocket and SSE: with an
/// empty mailbox, sending a message Bob's device reads once, and receiving and acking one of
/// Bob's.
#[test]
fn send_and_recv_work_over_long_poll_when_the_proxy_also_holds_sse_back() {
    let mut behind = Behind::start(
        [4_144_041_101, 4_144_041_102],
        ["ada-poll-fallback", "bob-poll-fallback"],
    );
    works_over(&mut behind, Transport::LongPoll, Stream::Hold);
}

/// Starts `args` on Ada's machine; fails unless the run succeeds over `transport` within
/// [`AT_ONCE`]. Gives what it showed.
#[track_caller]
fn at_once(behind: &Behind, transport: Transport, args: &[&str], what: &str) -> Finished {
    let from = behind.filter.mark();
    let started = Instant::now();
    let finished = behind.ada.machine.start(args).finish();
    let took = started.elapsed();
    assert!(finished.success, "{what} over {transport:?}: {finished:?}");
    assert_over(&behind.filter.seen_since(from), transport, &finished);
    assert!(
        took < AT_ONCE,
        "{what} over {transport:?} took {took:?}: {finished:?}"
    );
    finished
}

/// With the proxy refusing WebSocket, and then the event stream too, `skald recv` and `skald send`
/// exit at the end of catch-up over SSE and over long-poll, in under [`AT_ONCE`] while the request
/// timeout and the long-poll wait are 30 s: with an empty mailbox, and with one of Bob's messages
/// waiting, which `skald recv` shows.
#[test]
fn send_and_recv_exit_at_the_end_of_catch_up_over_sse_and_long_poll() {
    let mut behind = Behind::start_with(
        [4_144_041_201, 4_144_041_202],
        ["ada-quick-exit", "bob-quick-exit"],
        LONG_TIMEOUT,
        |settings| settings.long_poll_max_wait = Some(LONG_POLL_WAIT),
    );
    for transport in [Transport::Sse, Transport::LongPoll] {
        behind.leave(transport);
        let settled = recv(&behind.ada.machine);
        assert!(settled.success, "{settled:?}");
        at_once(&behind, transport, &[RECV], "recv with an empty mailbox");
        let mine = format!("ada, quick over {transport:?}");
        let channel = behind.channel.to_string();
        at_once(
            &behind,
            transport,
            &["send", &channel, &mine],
            "send with an empty mailbox",
        );
        behind.bob_sends(&format!("bob, waiting for a send over {transport:?}"));
        let again = format!("ada, quick again over {transport:?}");
        at_once(
            &behind,
            transport,
            &["send", &channel, &again],
            "send with an entry waiting",
        );
        let waiting = format!("bob, waiting for a recv over {transport:?}");
        behind.bob_sends(&waiting);

        let got = at_once(&behind, transport, &[RECV], "recv with an entry waiting");

        behind.assert_shows_bobs(&got.shown(), &waiting);
    }
}

/// Over each transport: Ada's `skald send` sends and exits; with a retention of 7 days and the
/// storing time moved 10 hours back, Bob's device reads the message once, and Ada's next
/// `skald recv` shows `delivered` for it once. Bob's device sends while Ada is offline; 10 hours
/// later Ada's `skald recv` shows the message once and acks it, and her next run does not show it
/// again.
#[test]
fn message_sent_and_received_hours_later_over_each_transport() {
    let mut behind = Behind::start(
        [4_144_041_301, 4_144_041_302],
        ["ada-hours-later", "bob-hours-later"],
    );
    for (previous, transport) in (0..).zip(TRANSPORTS) {
        behind.leave(transport);
        let says = format!("ada over {transport:?}, before the night");
        let msg_id =
            behind.ada_message_reaches_bob(transport, (&says, previous), Behind::hours_pass);

        let statuses = behind.ada_receives_over(transport);

        let bob = behind.bob.device();
        assert_eq!(
            delivered(&statuses.shown(), msg_id, bob),
            1,
            "delivered is not shown once over {transport:?}: {statuses:?}"
        );
        let answer = format!("bob over {transport:?}, after the night");
        behind.bob_message_reaches_ada(transport, &answer, Behind::hours_pass);
        let (again, _) = behind.ada_recv();
        assert!(again.success, "{again:?}");
        assert!(!again.shown().contains(&answer), "shown again: {again:?}");
    }
}

/// Over each transport: with Ada's machine cut off once the server has sent her device Bob's
/// message, so that nothing it sends after, its ack included, reaches the server, Ada's next
/// `skald recv` shows nothing twice: the two runs show the message once, and the server deletes it
/// once Ada has acked it.
#[test]
fn connection_cut_before_the_ack_repeats_nothing_over_each_transport() {
    let mut behind = Behind::start(
        [4_144_041_401, 4_144_041_402],
        ["ada-cut-mid-air", "bob-cut-mid-air"],
    );
    for transport in TRANSPORTS {
        behind.leave(transport);
        let says = format!("bob over {transport:?}, sent once");
        let envelope = behind.bob_sends(&says);
        behind.filter.cut_after(envelope.bytes());

        let (cut, seen) = behind.ada_recv();

        assert!(
            behind.filter.was_cut(),
            "{transport:?}: not cut: {cut:?}\n{seen:#?}"
        );
        behind.filter.stop_cutting();
        let next = behind.ada_receives_over(transport);
        let shown = format!("{}\n{}", cut.shown(), next.shown());
        behind.assert_shows_bobs(&shown, &says);
        behind.assert_acked(&envelope);
    }
}

/// Over long-poll: three runs of `skald send` with the server down for delivery fail, leaving
/// their messages in the outbox. Once it is back, Ada's `skald recv` sends them one at a time, each
/// after the previous one is answered, though the proxy holds the first back for a while; the
/// server stores them in `counter` order, and Bob's device reads them in that order, none missing.
#[test]
fn outbox_left_with_the_server_down_is_stored_in_counter_order_over_long_poll() {
    let mut behind = Behind::start(
        [4_144_041_501, 4_144_041_502],
        ["ada-outbox-order", "bob-outbox-order"],
    );
    behind.leave(Transport::LongPoll);
    behind.filter.down(true);
    let texts = [
        "first while the server is down",
        "second while the server is down",
        "third while the server is down",
    ];
    for text in texts {
        let (left, _) = behind.ada_sends(text);
        assert!(!left.success, "sent with the server down: {left:?}");
    }
    behind.filter.down(false);
    behind.filter.delay_next_send(Duration::from_secs(1));

    let (flushed, seen) = behind.ada_recv();

    assert!(flushed.success, "{flushed:?}");
    assert_over(&seen, Transport::LongPoll, &flushed);
    let sends: Vec<&Seen> = (seen.iter()).filter(|one| one.is("POST", SEND)).collect();
    assert!(
        sends.len() >= texts.len(),
        "not every message sent: {sends:#?}"
    );
    for two in sends.windows(2) {
        let answering = two[0].answering.expect("the send was answered");
        assert!(
            two[1].at >= answering,
            "a send before the previous one was answered: {sends:#?}"
        );
    }
    let read = behind.bob_reads();
    let got: Vec<(u64, &str)> = (read.iter())
        .map(|(_, counter, text)| (*counter, text.as_str()))
        .collect();
    assert_eq!(got, [(1, texts[0]), (2, texts[1]), (3, texts[2])]);
}

/// Over SSE, with the proxy dropping Ada's first send unanswered and letting WebSocket upgrades
/// through from then on, `skald send` reconnects in the transports' order: over a WebSocket, which
/// stores the message, and Bob's device reads it once.
#[test]
fn dropped_connection_reconnects_from_websocket() {
    let mut behind = Behind::start(
        [4_144_041_601, 4_144_041_602],
        ["ada-reconnects", "bob-reconnects"],
    );
    behind.leave(Transport::Sse);
    behind.filter.drop_next_send();
    let from = behind.filter.mark();
    let text = "ada, again after a drop";
    let args = send_args(behind.channel, text);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let sending = behind.ada.machine.start(&args);
    let deadline = Instant::now() + WAIT;
    while first(&behind.filter.seen_since(from), "POST", SEND).is_none() {
        assert!(Instant::now() < deadline, "nothing sent over HTTPS");
        thread::sleep(Duration::from_millis(10));
    }
    behind.leave(Transport::WebSocket);
    let said = sending.finish();

    assert!(said.success, "{said:?}");
    let seen = behind.filter.seen_since(from);
    let dropped = first(&seen, "POST", SEND).expect("the dropped send");
    assert_eq!(seen[dropped].fate, Fate::Closed, "{seen:#?}");
    assert!(
        answered(&seen[dropped..], "GET", WEBSOCKET, 101),
        "no WebSocket after the drop: {seen:#?}"
    );
    assert!(
        !answered(&seen, "POST", SEND, 200),
        "sent over HTTPS: {seen:#?}"
    );
    match behind.bob_reads().as_slice() {
        [(_, 1, read)] => assert_eq!(read, text),
        other => panic!("Bob's device read {other:?}"),
    }
}

/// Over long-poll, with one of Bob's messages waiting, Ada's `skald send` asks again after the
/// answer that gives the message, until an answer is empty, so the server deletes it once the run
/// ends; her next `skald recv` shows it once.
#[test]
fn long_poll_catch_up_asks_until_an_answer_is_empty() {
    let mut behind = Behind::start(
        [4_144_041_701, 4_144_041_702],
        ["ada-polls-to-the-end", "bob-polls-to-the-end"],
    );
    behind.leave(Transport::LongPoll);
    let waiting = "bob, waiting for a long-poll";
    let envelope = behind.bob_sends(waiting);

    let (said, seen) = behind.ada_sends("ada over long-poll, after Bob");

    assert!(said.success, "{said:?}");
    assert_over(&seen, Transport::LongPoll, &said);
    let polls = (seen.iter()).filter(|one| one.is("POST", POLL)).count();
    assert!(polls >= 2, "one long-poll: {seen:#?}");
    behind.assert_acked(&envelope);
    let got = behind.ada_receives_over(Transport::LongPoll);
    behind.assert_shows_bobs(&got.shown(), waiting);
}

/// Over SSE and over long-poll: once catch-up has ended, Ada's `skald recv` fetches the revisions
/// of a space of hers, then the devices of its accounts, which the relay holds back; meanwhile a
/// revision of the space is stored. The run fetched the revisions before that one, so it accepts it
/// only by its `space-notice`: once the relay gives another revision with its number,
/// `skald members` warns of a fork.
#[test]
fn revision_stored_after_the_fetch_is_accepted_by_its_notice_over_sse_and_long_poll() {
    let behind = Behind::start(
        [4_144_041_801, 4_144_041_802],
        ["ada-late-notice", "bob-late-notice"],
    );
    let (cloud, ada) = (behind.cloud(), &behind.ada);
    for (transport, name) in [
        (Transport::Sse, "notices-over-sse"),
        (Transport::LongPoll, "notices-over-poll"),
    ] {
        let forked = Forked::create(&behind, name);
        let space = forked.space;
        behind.leave(transport);
        let (relay, fetch) = (&cloud.relay, devices_request(space));
        relay.hold(&fetch, 2);

        let mut run = ada.machine.start(&[RECV]);
        let fetching = || relay.held() > 0;
        await_running(&mut run, "the devices fetch held back", fetching);
        store(cloud, &ada.token, &forked.late);
        if transport == Transport::Sse {
            let notice = Frame::Push(Push::SpaceNotice { space }).to_cbor();
            let passed = || behind.filter.streamed().contains(&notice);
            await_running(&mut run, "the notice passed to the machine", passed);
            // The notice reaches the machine's stream while its request waits.
            thread::sleep(Duration::from_millis(100));
        }
        relay.send_through(&fetch);
        let finished = run.finish();

        assert!(finished.success, "{transport:?}: {finished:?}");
        forked.assert_late_accepted(&behind, &format!("{transport:?}"));
    }
}

/// With a monthly limit of one message per space, over SSE Ada's first `skald send` is stored and
/// her second refused as `quota-exceeded`, and over long-poll her third too: each refused run shows
/// the refusal as over WebSocket, and her next run sends nothing again.
#[test]
fn refused_send_shows_its_refusal_over_sse_and_long_poll() {
    let behind = Behind::start_with(
        [4_144_041_901, 4_144_041_902],
        ["ada-over-quota", "bob-over-quota"],
        TIMEOUT,
        |settings| settings.monthly_messages_limit = Some(1),
    );
    behind.leave(Transport::Sse);
    let (under, seen) = behind.ada_sends("ada, under the limit");
    assert!(under.success, "{under:?}");
    assert_sent_over(&seen, Transport::Sse);
    for transport in [Transport::Sse, Transport::LongPoll] {
        behind.leave(transport);

        let (over, seen) = behind.ada_sends(&format!("ada over {transport:?}, over the limit"));

        assert!(!over.success, "{transport:?}: {over:?}");
        assert_over(&seen, transport, &over);
        assert_sent_over(&seen, transport);
        assert!(
            over.stdout.contains("refused: quota-exceeded"),
            "{transport:?}: no refusal shown: {over:?}"
        );
        let (next, seen) = behind.ada_recv();
        assert!(next.success, "{transport:?}: {next:?}");
        assert_eq!(first(&seen, "POST", SEND), None, "sent again: {seen:#?}");
    }
}

/// A problem details object the server answers in place of the WebSocket's opening, or of the
/// event stream, fails `skald recv` with its detail, without trying the next transport.
#[test]
fn server_refusal_fails_without_trying_the_next_transport() {
    const DETAIL: &str = "The server is shutting down.";
    let behind = Behind::start(
        [4_144_042_001, 4_144_042_002],
        ["ada-refused-by-server", "bob-refused-by-server"],
    );
    let problem = format!(
        r#"{{"type":"https://skald.so/problems/server-error","title":"Shutting down","status":503,"detail":"{DETAIL}"}}"#
    );
    let relay = &behind.cloud().relay;
    for (transport, refused, next) in [
        (Transport::WebSocket, "GET /ws", ("GET", EVENTS)),
        (Transport::Sse, "GET /mailbox/events", ("POST", POLL)),
    ] {
        behind.leave(transport);
        relay.refuse_with(refused, "503 Service Unavailable", &problem);

        let (failed, seen) = behind.ada_recv();

        relay.stop_refusing();
        assert!(!failed.success, "{transport:?}: {failed:?}");
        assert!(failed.stderr.contains(DETAIL), "{transport:?}: {failed:?}");
        assert_eq!(
            first(&seen, next.0, next.1),
            None,
            "{transport:?}: the next transport tried: {seen:#?}"
        );
    }
}
