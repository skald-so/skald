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

use std::num::NonZeroU64;
use std::thread;
use std::time::{Duration, Instant};

use browser::{APPROVE, Browser};
use cloud::{Cloud, WAIT, shown_grant};
use filter::{EVENTS, Fate, Filter, POLL, SEND, Seen, Stream, WEBSOCKET};
use fixtures::{TIMEOUT_SETTING, database, hex, send, send_args};
use kit::{RECV, Space, create_space, with_channel};
use machine::{FIRST_DEVICE, Finished, Machine, SERVER, SIGN_IN, YES};
use played::{Played, Took};
use proxy::Proxy;
use serde_json::Value;
use skald_proto::{Body, Cbor as _, ChannelId, Content, MessageId, RawEnvelope, Role};
use skald_server::Settings;
use talk::{ADA_HOST, BOB_HOST, Line, assert_shows_once, delivered, encrypted, stored};
use times::{MAILBOX, WEEK, moved_back};
use tls::Ca;
use tls_front::Front;
use world::{ACME, Person, World};

/// The setting of the proxy, and that of the root certificates the client trusts instead of the
/// platform's.
const HTTPS_PROXY: &str = "HTTPS_PROXY";
const CERT_FILE: &str = "SSL_CERT_FILE";

/// The server's host that only the proxy reaches, and the names of the endpoint's certificate.
const HOST: &str = "skald.test";
const NAMES: &str = "DNS:skald.test";

/// The request timeout of Ada's runs once she has signed in, in seconds.
const TIMEOUT: &str = "4";

/// The request timeout of Ada's runs, in seconds, and the server's longest long-poll wait, in the
/// test of runs that exit at the end of catch-up: as long as by default, so that a run that waits
/// out either instead of exiting takes far longer than [`AT_ONCE`].
const LONG_TIMEOUT: &str = "30";
const LONG_POLL_WAIT: Duration = Duration::from_secs(30);

/// How soon a run whose catch-up ends at once exits, whatever the load on the machine.
const AT_ONCE: Duration = Duration::from_secs(5);

/// How far back the tests move the server's times: the hours a recipient stays offline.
const HOURS_LATER: &str = "10 hours";

/// The transports of a device's connection, in the order the client tries them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    WebSocket,
    Sse,
    LongPoll,
}

const TRANSPORTS: [Transport; 3] = [Transport::WebSocket, Transport::Sse, Transport::LongPoll];

impl Transport {
    /// What the proxy blocks so that this transport is the first one that works: it refuses each
    /// one before it.
    fn blocked(self) -> (bool, Stream) {
        match self {
            Transport::WebSocket => (false, Stream::Forward),
            Transport::Sse => (true, Stream::Forward),
            Transport::LongPoll => (true, Stream::Refuse),
        }
    }
}

/// Ada, whose machine runs the CLI behind the proxy and created `acme`, Bob, whose device the test
/// plays, and a channel of `acme` with both, whose group holds both devices.
struct Behind {
    world: World,
    filter: Filter,
    ada: Person,
    bob: Played,
    space: Space,
    channel: ChannelId,
    /// The counter of Bob's last message in the channel.
    counter: u64,
    proxy: Proxy,
    front: Front,
    /// The test CA and the machine whose directory holds its files.
    trust: (Ca, Machine),
}

impl Behind {
    /// Ada and Bob, GitHub users `ids` with `logins`, on a server with a mailbox retention of 7
    /// days; at first the proxy blocks nothing.
    fn start(ids: [u64; 2], logins: [&str; 2]) -> Self {
        Self::start_with(ids, logins, (TIMEOUT, None))
    }

    /// As [`Behind::start`], Ada's runs with the request timeout `timeout`, in seconds, and the
    /// server's longest long-poll wait `long_poll`, if given.
    fn start_with(
        ids: [u64; 2],
        logins: [&str; 2],
        (timeout, long_poll): (&str, Option<Duration>),
    ) -> Self {
        let keeper = Machine::new();
        let ca = Ca::new(&keeper.root().join("ca"));
        let front = Front::bind(vec![ca.issue(NAMES)]);
        let retained = |settings: &mut Settings| {
            settings.mailbox_retention = Some(Duration::from_secs(WEEK));
            settings.long_poll_max_wait = long_poll;
        };
        let cloud = Cloud::fronted_over_tls_with(HOST, front.port(), (120, 1), retained);
        let filter = Filter::start(cloud.relay.address());
        front.forward_to(&filter.address());
        let world = World {
            cloud,
            browser: Browser::launch(),
        };
        let proxy = Proxy::tunneling();
        let behind = |host: &str| {
            Machine::new()
                .with_env(CERT_FILE, &ca.cert)
                .with_env(HTTPS_PROXY, proxy.url())
                .with_host_name(host)
        };
        let mut ada = signed_up(&world, behind(ADA_HOST), (ids[0], logins[0]), ADA_HOST);
        let bob = signed_up(&world, behind(BOB_HOST), (ids[1], logins[1]), BOB_HOST);
        let space = create_space(&world.cloud, &ada, ACME);
        let mut bob = Played::new(&world.cloud, bob);
        let added = [(&bob.person, Role::Member)];
        let channel = with_channel(&world.cloud, space.id, &ada, &added);
        let joined = recv(&ada.machine);
        assert!(joined.success, "{joined:?}");
        bob.take_in(&world.cloud);
        ada.machine = (ada.machine).with_env(TIMEOUT_SETTING, timeout);
        Behind {
            world,
            filter,
            ada,
            bob,
            space,
            channel,
            counter: 0,
            proxy,
            front,
            trust: (ca, keeper),
        }
    }

    fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// From now on, the proxy blocks what `transport` needs blocked to be the first that works.
    fn leave(&self, transport: Transport) {
        let (websocket, stream) = transport.blocked();
        self.filter.block(websocket, stream);
    }

    /// Runs `skald recv` on Ada's machine; gives what it showed, and what the proxy saw of it.
    fn ada_recv(&self) -> (Finished, Vec<Seen>) {
        let from = self.filter.mark();
        let finished = recv(&self.ada.machine);
        (finished, self.filter.seen_since(from))
    }

    /// Runs `skald send` of `text` to the channel on Ada's machine; gives what it showed, and what
    /// the proxy saw of it.
    fn ada_sends(&self, text: &str) -> (Finished, Vec<Seen>) {
        let from = self.filter.mark();
        let finished = send(&self.ada.machine, self.channel, text);
        (finished, self.filter.seen_since(from))
    }

    /// Bob's device has the server store its next message of `text` in the channel; gives its
    /// envelope.
    fn bob_sends(&mut self, text: &str) -> RawEnvelope {
        self.counter += 1;
        let from = self.address(&self.bob.person);
        let at = (self.space.id, self.channel);
        let line = Line::new(&from, self.counter, text);
        let envelope = encrypted(&mut self.bob, at, self.channel, &line);
        stored(&self.world.cloud, &self.bob, &envelope);
        envelope
    }

    /// Bob's device takes in its mailbox; gives the `msg_id`, `counter` and text of each message of
    /// the channel it read, in order.
    fn bob_reads(&mut self) -> Vec<(MessageId, u64, String)> {
        let mut read = Vec::new();
        for took in self.bob.take_in(&self.world.cloud) {
            if let Took::Read(channel, msg_id, decrypted) = took
                && channel == self.channel
            {
                let content = Content::from_cbor(&decrypted.plaintext).expect("content");
                let counter = content.counter.map_or(0, NonZeroU64::get);
                read.push((msg_id, counter, text_in(&content)));
            }
        }
        read
    }

    /// `person`'s person address in `acme`, on a server named after its host.
    fn address(&self, person: &Person) -> String {
        format!("{}/{}/user@{HOST}", self.space.name, person.handle)
    }

    /// Fails unless `shown` shows Bob's message `text` once, with the channel, his address and his
    /// device's name.
    #[track_caller]
    fn assert_shows_bobs(&self, shown: &str, text: &str) {
        let from = self.address(&self.bob.person);
        assert_shows_once(shown, text, (self.channel, &from, BOB_HOST));
    }

    /// Waits for up to [`WAIT`] until the server no longer holds `envelope`: every recipient
    /// device has acked it.
    #[track_caller]
    fn assert_acked(&self, envelope: &RawEnvelope) {
        let bytes = hex(envelope.bytes());
        let deadline = Instant::now() + WAIT;
        loop {
            if !database(self.cloud()).contains(&bytes) {
                return;
            }
            assert!(Instant::now() < deadline, "the envelope was never acked");
            thread::sleep(Duration::from_millis(200));
        }
    }

    /// Moves every time the server keeps of envelopes, sends and mailbox entries back by
    /// [`HOURS_LATER`].
    fn hours_pass(&self) {
        self.cloud().execute(&moved_back(&MAILBOX, HOURS_LATER));
    }

    /// Runs `skald recv` on Ada's machine; fails unless it succeeds over `transport`. Gives what
    /// it showed.
    #[track_caller]
    fn ada_receives_over(&self, transport: Transport) -> Finished {
        let (finished, seen) = self.ada_recv();
        assert!(finished.success, "{transport:?}: {finished:?}");
        assert_over(&seen, transport, &finished);
        finished
    }

    /// Ada's `skald send` of `text` succeeds over `transport` and exits; `meanwhile` runs; then
    /// Bob's device reads the message once, with the `counter` after Ada's `previous` messages in
    /// the channel. Gives its `msg_id`.
    #[track_caller]
    fn ada_message_reaches_bob(
        &mut self,
        transport: Transport,
        (text, previous): (&str, u64),
        meanwhile: impl FnOnce(&Self),
    ) -> MessageId {
        let (said, seen) = self.ada_sends(text);
        assert!(said.success, "{transport:?}: {said:?}");
        assert_over(&seen, transport, &said);
        assert_sent_over(&seen, transport);
        meanwhile(self);
        match self.bob_reads().as_slice() {
            [(msg_id, counter, read)] => {
                assert_eq!((*counter, read.as_str()), (previous + 1, text));
                *msg_id
            }
            other => panic!("{transport:?}: Bob's device read {other:?}"),
        }
    }

    /// Bob's device sends `text` while Ada is offline; `meanwhile` runs; then Ada's `skald recv`
    /// shows the message once over `transport` and acks it.
    #[track_caller]
    fn bob_message_reaches_ada(
        &mut self,
        transport: Transport,
        text: &str,
        meanwhile: impl FnOnce(&Self),
    ) {
        let envelope = self.bob_sends(text);
        meanwhile(self);
        let got = self.ada_receives_over(transport);
        self.assert_shows_bobs(&got.shown(), text);
        self.assert_acked(&envelope);
    }
}

/// A new account of GitHub user `id` with `login`, whose first machine, `machine` on the host
/// `host`, signs in at the server's public address through its proxy, the person approving its
/// code in Chrome.
fn signed_up(
    world: &World,
    machine: Machine,
    (id, login): (u64, &str),
    host: &'static str,
) -> Person {
    let (cloud, page) = (&world.cloud, world.browser.page());
    let grants = cloud.grants().len();
    let user = cloud.user(id, login);
    let args = [SIGN_IN, SERVER, cloud.public.as_str(), FIRST_DEVICE, YES];
    let mut signing_in = machine.start(&args);
    let grant = shown_grant(cloud, &mut signing_in, grants);
    cloud.decide(&page, &user, &grant, APPROVE);
    let finished = signing_in.finish();
    assert!(finished.success, "{login}: {finished:?}");
    world.person(user, machine, host)
}

/// Runs `skald recv` on `machine`.
fn recv(machine: &Machine) -> Finished {
    machine.start(&[RECV]).finish()
}

/// The text of the A2A message in `content`.
fn text_in(content: &Content) -> String {
    let Some(Body::A2a(json)) = &content.body else {
        panic!("no A2A message: {content:?}");
    };
    let message: Value = serde_json::from_str(json).expect("an A2A message");
    let text = &message["parts"][0]["text"];
    text.as_str().expect("a text part").to_owned()
}

/// Whether the proxy saw `method` `path` answered with `status`.
fn answered(seen: &[Seen], method: &str, path: &str, status: u16) -> bool {
    (seen.iter()).any(|one| one.is(method, path) && one.fate == Fate::Answered(status))
}

/// Where the proxy first saw `method` `path`, if it did.
fn first(seen: &[Seen], method: &str, path: &str) -> Option<usize> {
    seen.iter().position(|one| one.is(method, path))
}

/// Fails unless `seen`, what the proxy saw of `run`, a run that exits at the end of catch-up, shows
/// that its device's connection went over `transport`, and over no transport before it in the order
/// the client tries them; over long-poll, with a wait of 0 in each request.
#[track_caller]
fn assert_over(seen: &[Seen], transport: Transport, run: &Finished) {
    let websocket = answered(seen, "GET", WEBSOCKET, 101);
    let stream = answered(seen, "GET", EVENTS, 200);
    let poll = answered(seen, "POST", POLL, 200);
    let used = (websocket, stream, poll);
    let expected = match transport {
        Transport::WebSocket => (true, false, false),
        Transport::Sse => (false, true, false),
        Transport::LongPoll => (false, false, true),
    };
    assert_eq!(
        used, expected,
        "(WebSocket, SSE, long-poll) used, not {transport:?} alone: {seen:#?}\n{run:?}"
    );
    if transport == Transport::WebSocket {
        let mailbox: Vec<&Seen> = (seen.iter())
            .filter(|one| one.path.starts_with("/mailbox/"))
            .collect();
        assert!(
            mailbox.is_empty(),
            "HTTPS delivery over WebSocket: {mailbox:#?}"
        );
    }
    if transport == Transport::Sse {
        assert_eq!(
            first(seen, "POST", POLL),
            None,
            "a long-poll over SSE: {seen:#?}"
        );
    }
    let waiting: Vec<&Seen> = (seen.iter())
        .filter(|one| one.is("POST", POLL) && one.parameter("wait") != Some("0"))
        .collect();
    assert!(waiting.is_empty(), "a long-poll with a wait: {waiting:#?}");
}

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

/// Fails unless `seen`, what the proxy saw of a run that sent a message, shows that it was sent as
/// `transport` sends: over the WebSocket, or in `POST /mailbox/send` answered with its `stored`.
#[track_caller]
fn assert_sent_over(seen: &[Seen], transport: Transport) {
    let over_https = answered(seen, "POST", SEND, 200);
    assert_eq!(
        over_https,
        transport != Transport::WebSocket,
        "sent over HTTPS or not as {transport:?} sends: {seen:#?}"
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
        (LONG_TIMEOUT, Some(LONG_POLL_WAIT)),
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
