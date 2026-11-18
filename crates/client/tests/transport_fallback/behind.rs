//! What the tests of `skald` behind a proxy that blocks WebSocket or streaming share: Ada's machine
//! behind the TLS-intercepting proxy and its filter (`main.rs`), Bob's device the tests play, a
//! channel of `acme` with both, the transports of a device's connection, and what the proxy saw of
//! a run.

use std::num::NonZeroU64;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::{
    Body, Cbor as _, ChannelId, Content, MessageId, RawEnvelope, Role, Roster, SpaceId,
};
use skald_server::Settings;

use crate::browser::{APPROVE, Browser};
use crate::cloud::{Cloud, WAIT, shown_grant};
use crate::filter::{EVENTS, Fate, Filter, POLL, SEND, Seen, Stream, WEBSOCKET};
use crate::fixtures::{TIMEOUT_SETTING, database, hex, send};
use crate::kit::{RECV, Space, create_space, with_channel};
use crate::machine::{FIRST_DEVICE, Finished, Machine, SERVER, SIGN_IN, YES};
use crate::played::{Played, Took};
use crate::proxy::Proxy;
use crate::talk::{ADA_HOST, BOB_HOST, Line, assert_shows_once, encrypted, stored};
use crate::times::{MAILBOX, WEEK, moved_back};
use crate::tls::Ca;
use crate::tls_front::Front;
use crate::world::{
    ACME, FORK, Person, World, create, members, next_by, serve_revisions, warnings,
};

/// The setting of the proxy, and that of the root certificates the client trusts instead of the
/// platform's.
pub const HTTPS_PROXY: &str = "HTTPS_PROXY";
pub const CERT_FILE: &str = "SSL_CERT_FILE";

/// The server's host that only the proxy reaches, and the names of the endpoint's certificate.
pub const HOST: &str = "skald.test";
pub const NAMES: &str = "DNS:skald.test";

/// The request timeout of Ada's runs once she has signed in, in seconds.
pub const TIMEOUT: &str = "4";

/// How far back the tests move the server's times: the hours a recipient stays offline.
pub const HOURS_LATER: &str = "10 hours";

/// The transports of a device's connection, in the order the client tries them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    WebSocket,
    Sse,
    LongPoll,
}

pub const TRANSPORTS: [Transport; 3] = [Transport::WebSocket, Transport::Sse, Transport::LongPoll];

impl Transport {
    /// What the proxy blocks so that this transport is the first one that works: it refuses each
    /// one before it.
    pub fn blocked(self) -> (bool, Stream) {
        match self {
            Transport::WebSocket => (false, Stream::Forward),
            Transport::Sse => (true, Stream::Forward),
            Transport::LongPoll => (true, Stream::Refuse),
        }
    }
}

/// Ada, whose machine runs the CLI behind the proxy and created `acme`, Bob, whose device the test
/// plays, and a channel of `acme` with both, whose group holds both devices.
pub struct Behind {
    pub world: World,
    pub filter: Filter,
    pub ada: Person,
    pub bob: Played,
    pub space: Space,
    pub channel: ChannelId,
    /// The counter of Bob's last message in the channel.
    pub counter: u64,
    pub proxy: Proxy,
    pub front: Front,
    /// The test CA and the machine whose directory holds its files.
    pub trust: (Ca, Machine),
}

impl Behind {
    /// Ada and Bob, GitHub users `ids` with `logins`, on a server with a mailbox retention of 7
    /// days; at first the proxy blocks nothing.
    pub fn start(ids: [u64; 2], logins: [&str; 2]) -> Self {
        Self::start_with(ids, logins, TIMEOUT, |_| {})
    }

    /// As [`Behind::start`], Ada's runs with the request timeout `timeout`, in seconds, and the
    /// server's settings changed by `change`.
    pub fn start_with(
        ids: [u64; 2],
        logins: [&str; 2],
        timeout: &str,
        change: impl FnOnce(&mut Settings),
    ) -> Self {
        let keeper = Machine::new();
        let ca = Ca::new(&keeper.root().join("ca"));
        let front = Front::bind(vec![ca.issue(NAMES)]);
        let retained = |settings: &mut Settings| {
            settings.mailbox_retention = Some(Duration::from_secs(WEEK));
            change(settings);
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

    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// From now on, the proxy blocks what `transport` needs blocked to be the first that works.
    pub fn leave(&self, transport: Transport) {
        let (websocket, stream) = transport.blocked();
        self.filter.block(websocket, stream);
    }

    /// Runs `skald recv` on Ada's machine; gives what it showed, and what the proxy saw of it.
    pub fn ada_recv(&self) -> (Finished, Vec<Seen>) {
        let from = self.filter.mark();
        let finished = recv(&self.ada.machine);
        (finished, self.filter.seen_since(from))
    }

    /// Runs `skald send` of `text` to the channel on Ada's machine; gives what it showed, and what
    /// the proxy saw of it.
    pub fn ada_sends(&self, text: &str) -> (Finished, Vec<Seen>) {
        let from = self.filter.mark();
        let finished = send(&self.ada.machine, self.channel, text);
        (finished, self.filter.seen_since(from))
    }

    /// Bob's device has the server store its next message of `text` in the channel; gives its
    /// envelope.
    pub fn bob_sends(&mut self, text: &str) -> RawEnvelope {
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
    pub fn bob_reads(&mut self) -> Vec<(MessageId, u64, String)> {
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
    pub fn address(&self, person: &Person) -> String {
        format!("{}/{}/user@{HOST}", self.space.name, person.handle)
    }

    /// Fails unless `shown` shows Bob's message `text` once, with the channel, his address and his
    /// device's name.
    #[track_caller]
    pub fn assert_shows_bobs(&self, shown: &str, text: &str) {
        let from = self.address(&self.bob.person);
        assert_shows_once(shown, text, (self.channel, &from, BOB_HOST));
    }

    /// Waits for up to [`WAIT`] until the server no longer holds `envelope`: every recipient
    /// device has acked it.
    #[track_caller]
    pub fn assert_acked(&self, envelope: &RawEnvelope) {
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
    pub fn hours_pass(&self) {
        self.cloud().execute(&moved_back(&MAILBOX, HOURS_LATER));
    }

    /// Runs `skald recv` on Ada's machine; fails unless it succeeds over `transport`. Gives what
    /// it showed.
    #[track_caller]
    pub fn ada_receives_over(&self, transport: Transport) -> Finished {
        let (finished, seen) = self.ada_recv();
        assert!(finished.success, "{transport:?}: {finished:?}");
        assert_over(&seen, transport, &finished);
        finished
    }

    /// Ada's `skald send` of `text` succeeds over `transport` and exits; `meanwhile` runs; then
    /// Bob's device reads the message once, with the `counter` after Ada's `previous` messages in
    /// the channel. Gives its `msg_id`.
    #[track_caller]
    pub fn ada_message_reaches_bob(
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
    pub fn bob_message_reaches_ada(
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

/// A space Ada created, its revisions so far, and two revisions with the next number, both hers:
/// `late` adds Bob as a member, `fork` as an admin.
pub struct Forked {
    pub name: &'static str,
    pub space: SpaceId,
    pub kept: Vec<Roster>,
    pub late: Roster,
    pub fork: Roster,
}

impl Forked {
    /// Ada's account creates the space `name`, which no revision of hers yet follows on her
    /// machine.
    pub fn create(behind: &Behind, name: &'static str) -> Self {
        let (cloud, ada, bob) = (behind.cloud(), &behind.ada, &behind.bob.person);
        let last = create(cloud, ada, name);
        let space = last.body().space;
        let kept = (cloud.revisions(&ada.token, space).into_iter())
            .map(|(_, roster)| roster)
            .collect();
        Forked {
            name,
            space,
            kept,
            late: next_by(&last, ada, |list| list.push(bob.member(Role::Member))),
            fork: next_by(&last, ada, |list| list.push(bob.member(Role::Admin))),
        }
    }

    /// From now on, the relay gives the space's revisions as the kept ones and the fork. Fails
    /// unless `skald members` on Ada's machine then warns of a fork: her machine had accepted
    /// `late`.
    #[track_caller]
    pub fn assert_late_accepted(&self, behind: &Behind, what: &str) {
        let mut given: Vec<&Roster> = self.kept.iter().collect();
        given.push(&self.fork);
        serve_revisions(behind.cloud(), self.space, &given);
        let shown = members(&behind.ada.machine, self.name);
        assert!(
            warnings(&shown).iter().any(|line| line.contains(FORK)),
            "{what}: the revision was not accepted by its notice: {shown:?}"
        );
    }
}

/// A new account of GitHub user `id` with `login`, whose first machine, `machine` on the host
/// `host`, signs in at the server's public address through its proxy, the person approving its
/// code in Chrome.
pub fn signed_up(
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
pub fn recv(machine: &Machine) -> Finished {
    machine.start(&[RECV]).finish()
}

/// The text of the A2A message in `content`.
pub fn text_in(content: &Content) -> String {
    let Some(Body::A2a(json)) = &content.body else {
        panic!("no A2A message: {content:?}");
    };
    let message: Value = serde_json::from_str(json).expect("an A2A message");
    let text = &message["parts"][0]["text"];
    text.as_str().expect("a text part").to_owned()
}

/// Whether the proxy saw `method` `path` answered with `status`.
pub fn answered(seen: &[Seen], method: &str, path: &str, status: u16) -> bool {
    (seen.iter()).any(|one| one.is(method, path) && one.fate == Fate::Answered(status))
}

/// Where the proxy first saw `method` `path`, if it did.
pub fn first(seen: &[Seen], method: &str, path: &str) -> Option<usize> {
    seen.iter().position(|one| one.is(method, path))
}

/// Fails unless `seen`, what the proxy saw of `run`, a run that exits at the end of catch-up, shows
/// that its device's connection went over `transport`, and over no transport before it in the order
/// the client tries them; over long-poll, with a wait of 0 in each request.
#[track_caller]
pub fn assert_over(seen: &[Seen], transport: Transport, run: &Finished) {
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

/// Fails unless `seen`, what the proxy saw of a run that sent a message, shows that it was sent as
/// `transport` sends: over the WebSocket, or in `POST /mailbox/send` answered with its `stored`.
#[track_caller]
pub fn assert_sent_over(seen: &[Seen], transport: Transport) {
    let over_https = answered(seen, "POST", SEND, 200);
    assert_eq!(
        over_https,
        transport != Transport::WebSocket,
        "sent over HTTPS or not as {transport:?} sends: {seen:#?}"
    );
}
