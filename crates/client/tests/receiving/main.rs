//! Receiving messages in the CLI: `skald recv` connects as its device, first sends what is left in
//! its outbox, states its cursor, writes each mailbox entry it gets to its local state with the new
//! cursor, acks, and exits once catch-up has sent every entry the mailbox held
//! (`docs/protocol/delivery.md`, "Mailbox"). It opens each `message` and shows it only if it passes
//! the checks of `docs/protocol/README.md`, "Envelope" and "Scheme `hpke`", the sending device's
//! certificate checked as `docs/protocol/membership.md`, "Device checks", says and the device
//! admitted to the channel by the last accepted revision ("Admitted devices"); before it drops a
//! message for its sender, it accepts the space's latest revisions. It drops content without
//! `counter`, and content whose `msg_id` it already has from the same sending device. It shows each
//! message once, and each `delivered` and `expired` of the device's own messages; as an agent, only
//! those of its folder's space.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay and, before it, the front of the tests of sending, at the server's public
//! address. The front can also cut a connection at a machine's first ack, swallow the acks, hold
//! back the server's `message` pushes and the `batch` that ends catch-up, and put envelopes the
//! tests seal into a device's catch-up, as a compromised server would. A test plays the person on
//! the verification page in headless Chrome. The tests sign fixture revisions with the keys of the
//! machines' devices, read from their secrets files, and store them through the API, as an admin's
//! device would; they move storing times back in the server's database.
//!
//! A device follows only the spaces it created until invites and linking give it others, so the
//! CLI under test runs on the machine that created the space. The other members' devices are
//! played by the tests over the delivery protocol with their machines' keys: they send the
//! envelopes the tests seal and read and ack their mailboxes as their own clients would.
//!
//! The CLI, as these tests use it, beside `skald send` and the commands of the tests of roster
//! revisions, of creating channels and of agent isolation:
//! - `skald recv`: gets the device's mailbox, succeeds once catch-up has ended, and acks the entries
//!   it wrote in `ack` without waiting for that end;
//! - for each message it shows, its output holds the text once, its control characters escaped as
//!   in other text from the server, such as `\u{1b}` for ESC, and the channel's ID, `from` and the
//!   sending device's certificate `name`;
//! - for each `delivered` or `expired` of a message the device sent, a line holding the status, the
//!   message's ID and the recipient device's ID;
//! - for each message it drops for failing a check, a warning: a line saying `warning` that holds
//!   the sending device's ID when the payload names it, and never the message's text;
//! - `skald --agent <name> recv`, in a folder bound to a space: only that space's messages,
//!   statuses and warnings, its output naming nothing of other spaces; a later run of the person
//!   shows the rest.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/machine.rs"]
mod machine;
mod peer;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

use std::cell::Cell;
use std::num::NonZeroU64;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use cloud::{Cloud, WAIT};
use fixtures::{
    Channel, Issuer, Keyed, acme, ada_and_bob, add_device, cert_of, channel_in, database, fronted,
    fronted_with, hex, keyed, kill_once, killed_once, msg_id, person_address, recipient, send,
    send_args, strs,
};
use front::{Acks, Answers, Front, Sent};
use machine::{Finished, Machine, Run};
use peer::{Peer, last_seq, messages};
use serde_json::Value;
use skald_crypto::{SigningKey, open_message, seal_message};
use skald_proto::{
    Address, Body, Cbor as _, Cert, ChannelId, ChannelMember, Content, DeviceId, Envelope, Header,
    MessageId, NonEmpty, PROTOCOL_VERSION, RawEnvelope, Scheme, SpaceId, Unknown,
};
use skald_server::Settings;
use world::{
    ACME, AGENT, CLIENT_A, CLIENT_B, Desk, Person, WARNING, World, members, next_with, run, store,
};

/// The command that receives messages.
const RECV: &str = "recv";

/// The server statuses as the CLI shows them.
const DELIVERED: &str = "delivered";
const EXPIRED: &str = "expired";

/// The server's mailbox retention in these tests: 7 days.
const RETENTION_DAYS: u64 = 7;

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "the release train leaves at noon sharp";
const SECOND: &str = "carol joined, the runbook is in the wiki";
const CONTROL: &str = "\u{1b}]0;pwned\u{7}\u{1b}[2Kfake line";
/// [`CONTROL`] as the CLI shows it, its control characters escaped as in other text from the
/// server.
const CONTROL_SHOWN: &str = r"\u{1b}]0;pwned\u{7}\u{1b}[2Kfake line";

/// The agent the tests receive as.
const CLAUDE: &str = "claude-r7m2xd";

/// Gives the server a mailbox retention of [`RETENTION_DAYS`]. The server has no setting of it
/// yet (`docs/protocol/delivery.md`, "Mailbox", expiry), so a test that needs one fails here.
fn retained(_settings: &mut Settings) {
    panic!("the server has no setting of its mailbox retention yet");
}

/// Moves the storing and adding times of every envelope, send and mailbox entry the server holds
/// back by `interval`, a PostgreSQL interval such as `10 hours`.
fn move_back(cloud: &Cloud, interval: &str) {
    cloud.execute(&format!(
        "DO $$
         DECLARE c record;
         BEGIN
             FOR c IN
                 SELECT table_name, column_name FROM information_schema.columns
                 WHERE table_schema = current_schema()
                     AND table_name IN ('envelopes', 'sends', 'entries')
                     AND data_type = 'timestamp with time zone'
             LOOP
                 EXECUTE format('UPDATE %I SET %I = %I - interval %L',
                     c.table_name, c.column_name, c.column_name, '{interval}');
             END LOOP;
         END $$"
    ));
}

/// Runs `skald recv` on `machine`.
fn recv(machine: &Machine) -> Finished {
    run(machine, &[RECV])
}

/// Runs `skald recv` on `machine`; fails unless it succeeds.
#[track_caller]
fn received(machine: &Machine) -> Finished {
    let finished = recv(machine);
    assert!(finished.success, "{finished:?}");
    finished
}

/// Runs `skald recv` on `machine` until what the runs showed, together, satisfies `done`, for up
/// to [`WAIT`]; gives what they showed. Fails if a run fails.
fn received_until(machine: &Machine, what: &str, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + WAIT;
    let mut shown = String::new();
    loop {
        shown.push_str(&received(machine).shown());
        if done(&shown) {
            return shown;
        }
        assert!(Instant::now() < deadline, "no {what}: {shown}");
        thread::sleep(Duration::from_millis(500));
    }
}

/// How many times `shown` holds `text`.
fn times(shown: &str, text: &str) -> usize {
    shown.matches(text).count()
}

/// A message as a recipient's CLI shows it.
struct Shown<'a> {
    /// Its text as shown.
    text: &'a str,
    channel: ChannelId,
    from: String,
    /// The sending device's certificate `name`.
    device: &'a str,
}

/// Fails unless `shown` shows `message` once, with its channel, `from` and sending device.
#[track_caller]
fn assert_shows_once(shown: &str, message: &Shown) {
    assert_eq!(
        times(shown, message.text),
        1,
        "{:?} is not shown once: {shown}",
        message.text
    );
    for (what, part) in [
        ("channel", message.channel.to_string()),
        ("from", message.from.clone()),
        ("sending device", message.device.to_owned()),
    ] {
        assert!(shown.contains(&part), "no {what} {part}: {shown}");
    }
}

/// The lines of `shown` that report `status` for the message `msg_id` to the device `device`.
fn status_lines(shown: &str, status: &str, msg_id: MessageId, device: DeviceId) -> usize {
    let (msg_id, device) = (msg_id.to_string(), device.to_string());
    shown
        .lines()
        .filter(|line| {
            line.to_lowercase().contains(status) && line.contains(&msg_id) && line.contains(&device)
        })
        .count()
}

/// The lines of `shown` that mention the message `msg_id`.
fn lines_of(shown: &str, msg_id: MessageId) -> usize {
    let msg_id = msg_id.to_string();
    shown.lines().filter(|line| line.contains(&msg_id)).count()
}

/// The warning lines of `shown`.
fn warnings(shown: &str) -> Vec<String> {
    shown
        .lines()
        .filter(|line| line.to_lowercase().contains(WARNING))
        .map(str::to_owned)
        .collect()
}

/// The warning lines of `shown` that name the device `device`.
fn warnings_naming(shown: &str, device: DeviceId) -> usize {
    let device = device.to_string();
    warnings(shown)
        .iter()
        .filter(|line| line.contains(&device))
        .count()
}

/// The last envelope to `device` that the front saw sent.
fn envelope_to(front: &Front, device: DeviceId) -> Sent {
    front
        .sent()
        .into_iter()
        .rfind(|sent| recipient(sent) == device)
        .unwrap_or_else(|| panic!("no envelope to {device}"))
}

/// Waits for up to [`WAIT`] until the server's database no longer holds `envelope`.
fn assert_deleted(cloud: &Cloud, envelope: &RawEnvelope) {
    let deadline = Instant::now() + WAIT;
    let bytes = hex(envelope.bytes());
    while database(cloud).contains(&bytes) {
        assert!(
            Instant::now() < deadline,
            "the server still holds the envelope: it was never acked"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// Content a test seals as a sending device would, or as a compromised server or member could.
struct Draft<'a> {
    space: SpaceId,
    channel: ChannelId,
    msg_id: MessageId,
    /// `from`, in text form.
    from: String,
    counter: Option<u64>,
    /// The A2A `messageId`; `None`, `msg_id`.
    message_id: Option<MessageId>,
    text: &'a str,
}

impl<'a> Draft<'a> {
    /// A message of `channel` from `from` with `counter` 1 and a new `msg_id`.
    fn new(channel: &Channel, from: String, text: &'a str) -> Self {
        Draft {
            space: channel.space,
            channel: channel.id,
            msg_id: MessageId::generate(),
            from,
            counter: Some(1),
            message_id: None,
            text,
        }
    }

    /// The envelope of the draft, signed with `sender`, to the device `to`, sealed to the HPKE key
    /// `key`.
    fn seal(&self, sender: &SigningKey, to: DeviceId, key: &[u8; 32]) -> RawEnvelope {
        let header = Header {
            v: PROTOCOL_VERSION,
            space: self.space,
            channel: self.channel,
            msg_id: self.msg_id,
            scheme: Scheme::Hpke,
            recipients: Some(NonEmpty::new(to)),
            ttl: None,
            unknown: Unknown::default(),
        };
        let a2a = serde_json::json!({
            "messageId": self.message_id.unwrap_or(self.msg_id).to_string(),
            "role": "ROLE_USER",
            "parts": [{ "text": self.text }],
        })
        .to_string();
        let content = Content {
            from: self.from.parse::<Address>().expect("an address"),
            to: None,
            counter: self
                .counter
                .map(|counter| NonZeroU64::new(counter).expect("above 0")),
            hops: 0,
            reply_to: None,
            body: Some(Body::A2a(a2a)),
            delegation: None,
            grant: None,
            attachments: None,
            unknown: Unknown::default(),
        };
        let payload = seal_message(&header, &content, sender, key).expect("a sealed payload");
        RawEnvelope::from_bytes(Envelope { header, payload }.to_cbor()).expect("an envelope")
    }
}

/// Ada, whose machine runs the CLI and created `acme`, Bob, whose device the test plays, and the
/// channel of `acme` they are both in; Ada's machine retries for the tests' timeout.
struct Pair {
    world: World,
    front: Front,
    ada: Person,
    bob: Person,
    bob_keyed: Keyed,
    /// The HPKE key of the certificate of Ada's machine's device.
    ada_key: [u8; 32],
    channel: Channel,
    /// The counter of Bob's last message to Ada's device.
    counter: Cell<u64>,
}

impl Pair {
    /// The pair on a server with the settings `change` gives.
    fn start(ids: [u64; 2], logins: [&str; 2], change: impl FnOnce(&mut Settings)) -> Self {
        let (world, front) = fronted_with(change);
        let (ada, bob, channel) = ada_and_bob(&world, ids, logins);
        let cloud = &world.cloud;
        Pair {
            bob_keyed: keyed(cloud, &bob),
            ada_key: cert_of(cloud, &ada).body().hpke,
            front,
            ada,
            bob,
            channel,
            counter: Cell::new(0),
            world,
        }
    }

    fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// Bob's device's next message of `text` to Ada's device in the channel, sealed.
    fn bobs_next(&self, text: &str) -> RawEnvelope {
        self.counter.set(self.counter.get() + 1);
        let mut draft = Draft::new(&self.channel, person_address(ACME, &self.bob), text);
        draft.counter = Some(self.counter.get());
        draft.seal(&self.bob_keyed.signing, self.ada.device, &self.ada_key)
    }

    /// Bob's device connects, has the server store its message of `text` to Ada's device, and
    /// goes offline; gives the envelope.
    fn bob_sends(&self, text: &str) -> RawEnvelope {
        let envelope = self.bobs_next(text);
        Peer::connect(self.cloud(), &self.bob, &self.bob_keyed.signing).stored(&envelope);
        envelope
    }

    /// Runs `skald send` of `text` on Ada's machine; fails unless it succeeds. Gives the envelope
    /// to Bob's device.
    fn ada_sends(&self, text: &str) -> Sent {
        let finished = send(&self.ada.machine, self.channel.id, text);
        assert!(finished.success, "{finished:?}");
        envelope_to(&self.front, self.bob.device)
    }

    /// Starts `skald send` of `text` on Ada's machine.
    fn start_send(&self, text: &str) -> Run {
        self.ada
            .machine
            .start(&strs(&send_args(self.channel.id, text)))
    }

    /// Bob's device connects, gets its mailbox and acks it; gives the envelopes to it, once the
    /// server has deleted them.
    fn bob_catches_up(&self) -> Vec<RawEnvelope> {
        let mut peer = Peer::connect(self.cloud(), &self.bob, &self.bob_keyed.signing);
        let entries = peer.catch_up(0);
        if let Some(seq) = last_seq(&entries) {
            peer.ack(seq);
        }
        let envelopes = messages(&entries, self.bob.device);
        for envelope in &envelopes {
            assert_deleted(self.cloud(), envelope);
        }
        envelopes
    }

    /// Bob's message `text` as Ada's CLI shows it.
    fn message_of_bob<'a>(&self, text: &'a str) -> Shown<'a> {
        Shown {
            text,
            channel: self.channel.id,
            from: person_address(ACME, &self.bob),
            device: self.bob.host,
        }
    }
}

/// The text of the A2A message in `envelope`, which opens for `to` and verifies with `sender`.
fn text_of(envelope: &RawEnvelope, to: &Keyed, sender: &Cert) -> String {
    let opened = envelope.envelope();
    let content = open_message(&opened.header, &opened.payload, to.id, &to.hpke)
        .expect("opens for the recipient")
        .verify(sender)
        .expect("signed by the sending device");
    let Some(Body::A2a(json)) = content.body else {
        panic!("not an a2a body: {content:?}");
    };
    let message: Value = serde_json::from_str(&json).expect("an A2A message in JSON");
    message["parts"][0]["text"]
        .as_str()
        .expect("a text part")
        .to_owned()
}

/// Bob's device sends to Ada's and goes offline; with a retention of 7 days and the envelope's
/// storing time moved 10 hours back, Ada's `skald recv` shows the message once, with its channel,
/// `from` and Bob's device, and acks it: the server deletes it. Ada's next run does not show it
/// again.
#[test]
fn message_sent_while_the_recipient_was_offline_is_received_hours_later_once() {
    let pair = Pair::start(
        [4_122_071_001, 4_122_071_002],
        ["ada-late-standin", "bob-early-standin"],
        retained,
    );
    let sent = pair.bob_sends(FIRST);
    move_back(pair.cloud(), "10 hours");

    let first = received(&pair.ada.machine);

    assert_shows_once(&first.shown(), &pair.message_of_bob(FIRST));
    assert_deleted(pair.cloud(), &sent);
    let again = received(&pair.ada.machine);
    assert_eq!(times(&again.shown(), FIRST), 0, "shown again: {again:?}");
}

/// Ada's `skald send` exits; with a retention of 7 days and the envelope's storing time moved 10
/// hours back, Bob's device gets it and acks it, and Ada's next `skald recv` shows `delivered` for
/// Bob's device, once.
#[test]
fn sender_sees_delivered_once_the_recipient_acks_hours_later() {
    let pair = Pair::start(
        [4_122_071_051, 4_122_071_052],
        ["ada-reports-standin", "bob-acks-standin"],
        retained,
    );
    let sent = pair.ada_sends(FIRST);
    move_back(pair.cloud(), "10 hours");
    let got = pair.bob_catches_up();
    assert_eq!(got.len(), 1, "Bob's device got {got:?}");

    let shown = received(&pair.ada.machine).shown();

    let (id, bob) = (msg_id(&sent), pair.bob.device);
    assert_eq!(
        status_lines(&shown, DELIVERED, id, bob),
        1,
        "delivered is not shown once: {shown}"
    );
    let again = received(&pair.ada.machine).shown();
    assert_eq!(lines_of(&again, id), 0, "shown again: {again}");
}

/// Ada's `skald send` whose `stored` the front lost fails after retrying; Ada's next `skald recv`
/// sends the envelope again, and Bob's device gets the message once.
#[test]
fn send_retried_after_a_lost_stored_reaches_the_recipient_once() {
    let pair = Pair::start(
        [4_122_071_101, 4_122_071_102],
        ["ada-retries-standin", "bob-once-standin"],
        |_| {},
    );
    pair.front.answer(Answers::Lose);
    let lost = send(&pair.ada.machine, pair.channel.id, FIRST);
    assert!(!lost.success, "{lost:?}");
    assert!(pair.front.lost() > 0, "no stored was lost: {lost:?}");
    let left = envelope_to(&pair.front, pair.bob.device);
    pair.front.answer(Answers::Forward);
    let before = pair.front.sent().len();

    received(&pair.ada.machine);

    let resent: Vec<Sent> = pair.front.sent().split_off(before);
    assert!(
        resent
            .iter()
            .any(|sent| sent.envelope.bytes() == left.envelope.bytes()),
        "the next recv did not send the left envelope again"
    );
    let got = pair.bob_catches_up();
    assert_eq!(got.len(), 1, "Bob's device got {got:?}");
    let ada_cert = cert_of(pair.cloud(), &pair.ada);
    assert_eq!(text_of(&got[0], &pair.bob_keyed, &ada_cert), FIRST);
}

/// A `skald send` killed after it sent its envelope, which the front swallowed, leaves it in the
/// outbox; Ada's next `skald recv` sends it with the same bytes, and Bob's device gets it once.
#[test]
fn message_a_killed_send_left_in_the_outbox_is_sent_by_the_next_recv() {
    let pair = Pair::start(
        [4_122_071_201, 4_122_071_202],
        ["ada-cut-off-standin", "bob-gets-it-standin"],
        |_| {},
    );
    pair.front.swallow(true);
    kill_once(pair.start_send(FIRST), "swallowed send", || {
        pair.front.swallowed() > 0
    });
    let left = envelope_to(&pair.front, pair.bob.device);
    assert!(
        !database(pair.cloud()).contains(&hex(left.envelope.bytes())),
        "the server got the swallowed envelope"
    );
    pair.front.swallow(false);

    let flushed = received(&pair.ada.machine);

    let got = pair.bob_catches_up();
    assert_eq!(
        got.iter().map(RawEnvelope::bytes).collect::<Vec<_>>(),
        [left.envelope.bytes()],
        "Bob's device does not get the left envelope once: {flushed:?}"
    );
}

/// A message the front puts into Ada's next catch-up under the `seq` of Ada's cursor, as a
/// redelivery after a lost ack, is dropped: never shown, though it is valid and new.
#[test]
fn entry_with_a_seq_up_to_the_cursor_is_dropped() {
    let pair = Pair::start(
        [4_122_071_351, 4_122_071_352],
        ["ada-cursor-standin", "bob-replays-standin"],
        |_| {},
    );
    pair.bob_sends(FIRST);
    let first = received(&pair.ada.machine);
    assert_shows_once(&first.shown(), &pair.message_of_bob(FIRST));
    pair.front
        .redeliver(pair.ada.device, pair.bobs_next(SECOND));

    let next = received(&pair.ada.machine);

    assert_eq!(pair.front.injected(), 1, "not put into catch-up: {next:?}");
    assert_eq!(times(&next.shown(), SECOND), 0, "shown: {next:?}");
}

/// With Ada's connection cut at its first ack, which the server never gets, Ada's next run shows
/// nothing twice: the two runs show the message once, and the server deletes the envelope once Ada
/// has acked it.
#[test]
fn connection_cut_before_the_ack_repeats_nothing() {
    let pair = Pair::start(
        [4_122_071_301, 4_122_071_302],
        ["ada-cut-standin", "bob-steady-standin"],
        |_| {},
    );
    let sent = pair.bob_sends(FIRST);
    pair.front.acks(Acks::Cut);

    let cut = recv(&pair.ada.machine);

    assert!(pair.front.acked() > 0, "no ack: {cut:?}");
    pair.front.acks(Acks::Forward);
    let next = received(&pair.ada.machine);
    let shown = format!("{}\n{}", cut.shown(), next.shown());
    assert_shows_once(&shown, &pair.message_of_bob(FIRST));
    assert_deleted(pair.cloud(), &sent);
}

/// Ada's CLI killed after writing the message, once it has sent its ack, which the front
/// swallowed, and before catch-up ended: the next run shows nothing twice, and the server deletes
/// the envelope once Ada has acked it.
#[test]
fn cli_killed_after_writing_repeats_nothing() {
    let pair = Pair::start(
        [4_122_071_401, 4_122_071_402],
        ["ada-crashes-standin", "bob-calm-standin"],
        |_| {},
    );
    let sent = pair.bob_sends(FIRST);
    pair.front.acks(Acks::Swallow);
    pair.front.hold_end(true);

    let killed = killed_once(pair.ada.machine.start(&[RECV]), "ack", || {
        pair.front.acked() > 0
    });

    pair.front.acks(Acks::Forward);
    pair.front.hold_end(false);
    let next = received(&pair.ada.machine);
    let shown = format!("{}\n{}", killed.shown(), next.shown());
    assert_shows_once(&shown, &pair.message_of_bob(FIRST));
    assert_deleted(pair.cloud(), &sent);
}

/// Ada's CLI killed before writing the message, which the front held back: the next run shows it,
/// once.
#[test]
fn cli_killed_before_writing_shows_the_message_on_the_next_run() {
    let pair = Pair::start(
        [4_122_071_501, 4_122_071_502],
        ["ada-killed-standin", "bob-patient-standin"],
        |_| {},
    );
    pair.bob_sends(FIRST);
    pair.front.hold_messages(true);

    let killed = killed_once(pair.ada.machine.start(&[RECV]), "message", || {
        pair.front.held_messages() > 0
    });

    assert_eq!(times(&killed.shown(), FIRST), 0, "{killed:?}");
    pair.front.hold_messages(false);
    let next = received(&pair.ada.machine);
    assert_shows_once(&next.shown(), &pair.message_of_bob(FIRST));
}

/// With a retention of 7 days, Bob's device offline and the envelope's storing time moved back
/// past it, Ada's `skald recv` shows `expired` for Bob's device, once, and Bob's device never gets
/// the message.
#[test]
fn message_expired_before_the_recipient_connects_shows_expired_to_its_sender() {
    let pair = Pair::start(
        [4_122_071_601, 4_122_071_602],
        ["ada-waits-standin", "bob-away-standin"],
        retained,
    );
    let sent = pair.ada_sends(FIRST);
    move_back(pair.cloud(), &format!("{} days", RETENTION_DAYS + 1));
    let (id, bob) = (msg_id(&sent), pair.bob.device);

    let shown = received_until(&pair.ada.machine, EXPIRED, |shown| {
        status_lines(shown, EXPIRED, id, bob) > 0
    });

    assert_eq!(
        status_lines(&shown, EXPIRED, id, bob),
        1,
        "expired is not shown once: {shown}"
    );
    let got = pair.bob_catches_up();
    assert!(got.is_empty(), "Bob's device got {got:?}");
}

/// Envelopes the front puts into Ada's catch-up, as a compromised server would, are dropped with a
/// warning: from a device without a valid certificate, one only the server certified and one with
/// none; from a device its member's `excluded` lists; with `from` another account's handle or a
/// space other than `space`; from Carol's account, outside the channel; with a `messageId` other
/// than `msg_id`; without `counter`; sealed to another device's key. A valid message among them is
/// shown once; the same content again under another `seq`, as the same bytes or sealed again, is
/// dropped. No dropped text is shown.
#[test]
fn envelopes_failing_a_check_are_dropped_with_a_warning() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_122_071_701, "ada-checks-standin", "ada-workstation");
    let bob = world.sign_up(4_122_071_702, "bob-honest-standin", "bob-workstation");
    let carol = world.sign_up(4_122_071_703, "carol-aside-standin", "carol-laptop");
    let forged = add_device(cloud, bob.account, "bob-forged", &Issuer::Server);
    let unlinked = add_device(cloud, bob.account, "bob-unlinked", &Issuer::Nobody);
    let barred = add_device(cloud, bob.account, "bob-barred", &Issuer::Device(&bob));
    let channel = acme(
        cloud,
        &ada,
        &[&bob, &carol],
        &[&bob],
        &[(bob.account, barred.id)],
    );
    let (bob_keyed, carol_keyed) = (keyed(cloud, &bob), keyed(cloud, &carol));
    let ada_key = cert_of(cloud, &ada).body().hpke;
    let (from_bob, from_carol) = (person_address(ACME, &bob), person_address(ACME, &carol));
    let to_ada = |draft: &Draft, sender: &Keyed| draft.seal(&sender.signing, ada.device, &ada_key);

    let valid = Draft::new(&channel, from_bob.clone(), "the valid one among the forged");
    let valid_bytes = to_ada(&valid, &bob_keyed);
    let mut envelopes = vec![valid_bytes.clone()];
    // Each dropped envelope's text, and the device its warning names, if the payload names one.
    let mut dropped: Vec<(&str, Option<DeviceId>)> = Vec::new();
    for (sender, text) in [
        (&forged, "from a device only the server certified"),
        (&unlinked, "from a device without a certificate"),
        (&barred, "from a device its member excludes"),
    ] {
        envelopes.push(to_ada(
            &Draft::new(&channel, from_bob.clone(), text),
            sender,
        ));
        dropped.push((text, Some(sender.id)));
    }
    let text = "from carol, a member outside the channel";
    let outside = Draft::new(&channel, from_carol.clone(), text);
    envelopes.push(to_ada(&outside, &carol_keyed));
    dropped.push((text, Some(carol.device)));
    let text = "from carol's handle, by bob's device";
    envelopes.push(to_ada(&Draft::new(&channel, from_carol, text), &bob_keyed));
    dropped.push((text, Some(bob.device)));
    let text = "from bob in a space other than acme";
    let elsewhere = person_address(bob.handle.as_str(), &bob);
    envelopes.push(to_ada(&Draft::new(&channel, elsewhere, text), &bob_keyed));
    dropped.push((text, Some(bob.device)));
    let text = "with a messageId other than msg_id";
    let mut other_id = Draft::new(&channel, from_bob.clone(), text);
    other_id.message_id = Some(MessageId::generate());
    envelopes.push(to_ada(&other_id, &bob_keyed));
    dropped.push((text, Some(bob.device)));
    let text = "without a counter";
    let mut uncounted = Draft::new(&channel, from_bob.clone(), text);
    uncounted.counter = None;
    envelopes.push(to_ada(&uncounted, &bob_keyed));
    dropped.push((text, Some(bob.device)));
    let text = "sealed to carol's key";
    let carol_key = cert_of(cloud, &carol).body().hpke;
    let misdirected = Draft::new(&channel, from_bob.clone(), text);
    envelopes.push(misdirected.seal(&bob_keyed.signing, ada.device, &carol_key));
    dropped.push((text, None));
    envelopes.push(valid_bytes);
    envelopes.push(to_ada(&valid, &bob_keyed));
    let count = envelopes.len();
    front.inject(ada.device, envelopes);

    let finished = received(&ada.machine);

    assert_eq!(
        front.injected(),
        count,
        "not put into catch-up: {finished:?}"
    );
    let shown = finished.shown();
    assert_shows_once(
        &shown,
        &Shown {
            text: valid.text,
            channel: channel.id,
            from: from_bob,
            device: bob.host,
        },
    );
    for (text, _) in &dropped {
        assert_eq!(times(&shown, text), 0, "{text:?} is shown: {shown}");
    }
    let warned = warnings(&shown);
    assert!(
        warned.len() >= dropped.len(),
        "{} warnings for {} dropped envelopes: {shown}",
        warned.len(),
        dropped.len()
    );
    for device in [forged.id, unlinked.id, barred.id, carol.device, bob.device] {
        let expected = dropped
            .iter()
            .filter(|(_, named)| *named == Some(device))
            .count();
        assert!(
            warnings_naming(&shown, device) >= expected,
            "fewer than {expected} warnings name {device}: {shown}"
        );
    }
}

/// Carol's device sends to Ada's in the channel, which a revision Ada's machine has not accepted
/// yet adds Carol's account to; Ada's `skald recv` accepts the revision first and shows her
/// message.
#[test]
fn message_from_an_account_a_revision_not_yet_accepted_adds_is_shown() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_122_071_801, "ada-admits-standin", "ada-workstation");
    let bob = world.sign_up(4_122_071_802, "bob-member-standin", "bob-workstation");
    let carol = world.sign_up(4_122_071_803, "carol-joins-standin", "carol-laptop");
    let channel = acme(cloud, &ada, &[&bob, &carol], &[&bob], &[]);
    members(&ada.machine, ACME);
    let (_, last) = cloud
        .revisions(&ada.token, channel.space)
        .pop()
        .expect("a revision");
    let joined = next_with(&last, (ada.device, &ada.key), |body| {
        body.channels
            .iter_mut()
            .find(|listed| listed.id == channel.id)
            .expect("the channel")
            .members
            .push(ChannelMember {
                account: carol.account,
                history: false,
            });
    });
    store(cloud, &ada.token, &joined);
    let carol_keyed = keyed(cloud, &carol);
    let draft = Draft::new(&channel, person_address(ACME, &carol), SECOND);
    let ada_key = cert_of(cloud, &ada).body().hpke;
    let envelope = draft.seal(&carol_keyed.signing, ada.device, &ada_key);
    Peer::connect(cloud, &carol, &carol_keyed.signing).stored(&envelope);

    let finished = received(&ada.machine);

    assert_shows_once(
        &finished.shown(),
        &Shown {
            text: SECOND,
            channel: channel.id,
            from: person_address(ACME, &carol),
            device: carol.host,
        },
    );
}

/// The texts of the agent test: what Bob's device sends to each space's channel, and what the
/// front forges in each.
const BOB_IN_A: &str = "clienta's invoice is approved";
const BOB_IN_B: &str = "the other client's contract is signed";
const FORGED_IN_A: &str = "forged in clienta's channel";
const FORGED_IN_B: &str = "forged in the other client's channel";

/// As an agent in `~/work/clientA`, `skald recv` shows only clienta's message from Bob, the
/// `delivered` of Ada's message there and the warning of a message dropped there; its output names
/// nothing of clientb. The person's next run shows clientb's message, status and warning, and
/// nothing of clienta's again.
#[test]
fn agent_receives_only_in_the_space_of_its_folder() {
    let (world, front) = fronted();
    let desk = Desk::sign_up(world, 4_122_071_901, "ada-desk-standin");
    let (cloud, ada) = (&desk.world.cloud, &desk.ada);
    let bob = desk
        .world
        .sign_up(4_122_071_902, "bob-client-standin", "bob-workstation");
    let mut channels = Vec::new();
    for first in [&desk.client_a, &desk.client_b] {
        channels.push(channel_in(cloud, ada, first, &[&bob], &[&bob], &[]));
    }
    let folder = desk.dir("work/clientA");
    desk.add_rule(&folder, CLIENT_A);
    let mut ada_ids = Vec::new();
    for (channel, text) in channels.iter().zip(["ada asks in a", "ada asks in b"]) {
        let finished = send(&ada.machine, channel.id, text);
        assert!(finished.success, "{finished:?}");
        ada_ids.push(msg_id(&envelope_to(&front, bob.device)));
    }
    let bob_keyed = keyed(cloud, &bob);
    let mut peer = Peer::connect(cloud, &bob, &bob_keyed.signing);
    let entries = peer.catch_up(0);
    let got = messages(&entries, bob.device);
    assert_eq!(got.len(), 2, "Bob's device got {got:?}");
    peer.ack(last_seq(&entries).expect("a seq"));
    for envelope in &got {
        assert_deleted(cloud, envelope);
    }
    let ada_key = cert_of(cloud, ada).body().hpke;
    let spaces = [CLIENT_A, CLIENT_B];
    for ((channel, space), text) in channels.iter().zip(spaces).zip([BOB_IN_A, BOB_IN_B]) {
        let draft = Draft::new(channel, person_address(space, &bob), text);
        peer.stored(&draft.seal(&bob_keyed.signing, ada.device, &ada_key));
    }
    drop(peer);
    let forged: Vec<RawEnvelope> = channels
        .iter()
        .zip(spaces.into_iter().zip([FORGED_IN_A, FORGED_IN_B]))
        .map(|(channel, (space, text))| {
            let mut draft = Draft::new(channel, person_address(space, &bob), text);
            draft.message_id = Some(MessageId::generate());
            draft.seal(&bob_keyed.signing, ada.device, &ada_key)
        })
        .collect();
    front.inject(ada.device, forged);
    let bob_in = |at: usize, text| Shown {
        text,
        channel: channels[at].id,
        from: person_address(spaces[at], &bob),
        device: bob.host,
    };

    let agent = as_agent(&ada.machine, &folder);

    assert!(agent.success, "{agent:?}");
    assert_eq!(front.injected(), 2, "not put into catch-up: {agent:?}");
    let shown = agent.shown();
    assert_shows_once(&shown, &bob_in(0, BOB_IN_A));
    assert_eq!(
        status_lines(&shown, DELIVERED, ada_ids[0], bob.device),
        1,
        "{shown}"
    );
    assert_eq!(warnings_naming(&shown, bob.device), 1, "{shown}");
    for named in [
        CLIENT_B.to_owned(),
        channels[1].space.to_string(),
        channels[1].id.to_string(),
        ada_ids[1].to_string(),
        BOB_IN_B.to_owned(),
        FORGED_IN_A.to_owned(),
        FORGED_IN_B.to_owned(),
    ] {
        assert!(
            !shown.to_lowercase().contains(&named.to_lowercase()),
            "the agent's output names {named}: {shown}"
        );
    }

    let person = received(&ada.machine);

    let shown = person.shown();
    assert_shows_once(&shown, &bob_in(1, BOB_IN_B));
    assert_eq!(
        status_lines(&shown, DELIVERED, ada_ids[1], bob.device),
        1,
        "{shown}"
    );
    assert_eq!(warnings_naming(&shown, bob.device), 1, "{shown}");
    assert_eq!(times(&shown, BOB_IN_A), 0, "shown again: {shown}");
    assert_eq!(lines_of(&shown, ada_ids[0]), 0, "shown again: {shown}");
    assert_eq!(times(&shown, FORGED_IN_B), 0, "{shown}");
}

/// Runs `skald --agent <CLAUDE> recv` on `machine` in `dir`.
fn as_agent(machine: &Machine, dir: &Path) -> Finished {
    machine.start_in(dir, &[AGENT, CLAUDE, RECV]).finish()
}

/// Text with control characters from Bob's device is shown escaped, and Ada's output holds none of
/// them.
#[test]
fn control_characters_in_a_message_are_shown_escaped() {
    let pair = Pair::start(
        [4_122_072_001, 4_122_072_002],
        ["ada-terminal-standin", "bob-escapes-standin"],
        |_| {},
    );
    pair.bob_sends(CONTROL);

    let finished = received(&pair.ada.machine);

    let shown = finished.shown();
    assert_shows_once(&shown, &pair.message_of_bob(CONTROL_SHOWN));
    assert!(
        !shown.contains(['\u{1b}', '\u{7}']),
        "a control character reaches the terminal: {shown:?}"
    );
}
