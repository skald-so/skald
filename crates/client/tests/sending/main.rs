//! Sending a message in the CLI: `skald send` sends a text to a channel, by its ID, of a space the
//! device follows; as an agent, only in the space of its folder. It first accepts the space's
//! latest roster revisions, then seals one `hpke` envelope, with `report` true, to each admitted
//! `client` device of the channel but its own (`docs/protocol/membership.md`, "Admitted devices"),
//! with the HPKE key of the device's certificate. The content has `from` the person address, or
//! the agent's address, `hops` 0, a `counter` per recipient device from 1, and an `a2a` body: an
//! A2A message with the text whose `messageId` is the `msg_id` (`docs/protocol/README.md`,
//! "Content"). The envelopes go into the outbox before anything is sent; the CLI sends them over
//! its device's connection and retries with the same bytes until each gets `stored` or
//! `send-refused`, for as long as its request timeout, then names what is left and fails; a later
//! `skald send` sends that first (`docs/protocol/delivery.md`, "Send").
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay and, before it, a front at the server's public address,
//! `http://localhost:<port>`, which reads the WebSocket frames and can lose or change the server's
//! answers, swallow a `send` and take the server down for delivery; a test plays the person on the
//! verification page in headless Chrome. Each machine keeps its state in a directory of its own and
//! its secrets in a file there. The tests sign fixture revisions with the keys of the machines'
//! devices, read from their secrets files, and store them through the API, as an admin's device
//! would. Devices beside the machines' are written to the server's database, as linking will
//! register them.
//!
//! The CLI, as these tests use it, beside the commands of the tests of roster revisions, of
//! creating channels and of agent isolation:
//! - `skald send <channel> <text>`: sends `<text>` to the channel whose ID is `<channel>`; it says
//!   the message's ID, and for each recipient device a line holding the device's ID that says
//!   `stored` once the server has stored its envelope, or the problem the server refused it with;
//! - a run that is left with envelopes the server has not answered fails, its output holding the
//!   message's ID and each recipient device it is left with;
//! - with no device to send to, it fails and says `no device` can receive the message, sending
//!   nothing;
//! - `SKALD_TIMEOUT`: the request timeout, in seconds, which a run retries for.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
mod front;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use browser::Browser;
use cloud::{Cloud, WAIT};
use front::{Answers, Front, Sent};
use machine::{Finished, Machine, Run};
use serde_json::Value;
use skald_crypto::{HpkeKey, SigningKey, issue, open_message};
use skald_proto::{
    AccountId, Body, Cbor as _, Cert, CertBody, ChannelId, ChannelMember, Content, DeviceId,
    DeviceKind, MessageId, NonEmpty, Role, Roster, Scheme, SpaceId, Unknown,
};
use world::{
    ACME, AGENT, CLIENT_A, CLIENT_B, CREATE_CHANNEL, Desk, HOST, Person, World, create, next_by,
    next_with, now, run, space_id, store,
};

/// The command that sends a message.
const SEND: &str = "send";

/// The setting of the request timeout, and the one the tests give a run that retries.
const TIMEOUT_SETTING: &str = "SKALD_TIMEOUT";
const TIMEOUT: Duration = Duration::from_secs(4);

/// What the CLI says of an envelope the server stored, and when no device can receive a message.
const STORED: &str = "stored";
const NO_DEVICE: &str = "no device";

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "deploy the staging build at nine-forty";
const SECOND: &str = "the staging build is green, ship it";
const THIRD: &str = "rolling back the canary in five minutes";

/// The agent the tests send as.
const CLAUDE: &str = "claude-k3s8vq";

/// A world whose server's public address is that of a front before the relay, which forwards to
/// it, and the front.
fn fronted() -> (World, Front) {
    let front = Front::bind();
    let cloud = Cloud::fronted(HOST, front.port(), 120, 1);
    front.forward_to(cloud.relay.url.trim_start_matches("http://"));
    let world = World {
        cloud,
        browser: Browser::launch(),
    };
    (world, front)
}

/// `person`, its runs retrying for [`TIMEOUT`].
fn retrying_for_the_timeout(mut person: Person) -> Person {
    person.machine = person
        .machine
        .with_env(TIMEOUT_SETTING, TIMEOUT.as_secs().to_string());
    person
}

/// Runs `skald send <channel> <text>` on `machine`.
fn send(machine: &Machine, channel: ChannelId, text: &str) -> Finished {
    machine.start(&strs(&send_args(channel, text))).finish()
}

/// The arguments of `skald send <channel> <text>`.
fn send_args(channel: ChannelId, text: &str) -> Vec<String> {
    vec![SEND.to_owned(), channel.to_string(), text.to_owned()]
}

/// `args` as arguments.
fn strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

/// A device whose HPKE key the test holds, to open what is sealed to it.
struct Keyed {
    id: DeviceId,
    hpke: HpkeKey,
}

/// The certificate of `person`'s machine's device, as the server holds it.
fn cert_of(cloud: &Cloud, person: &Person) -> Cert {
    cloud
        .devices(&person.token)
        .into_iter()
        .find(|device| device.id == person.device)
        .and_then(|device| device.cert)
        .expect("the device's certificate")
}

/// `person`'s machine's device, its HPKE key read from its secrets file.
fn keyed(cloud: &Cloud, person: &Person) -> Keyed {
    let public = cert_of(cloud, person).body().hpke;
    let holders = person.machine.files_holding(&person.token);
    assert_eq!(holders.len(), 1, "the secrets file: {holders:?}");
    let secrets = fs::read(&holders[0]).expect("read the secrets file");
    let secret = keys::private_key(&secrets, &public, keys::x25519)
        .expect("the secrets file holds the HPKE key");
    Keyed {
        id: person.device,
        hpke: HpkeKey::from_bytes(&secret),
    }
}

/// Who issued the certificate of a device a test writes to the server's database.
enum Issuer<'a> {
    /// The device of this person's machine: a valid certificate.
    Device(&'a Person),
    /// A key of no device, in place of the account's master key, as only a server could sign it.
    Server,
    /// Nobody: the device has no certificate.
    Nobody,
}

/// Writes to the server's database a `client` device of `account` named `name`, its certificate
/// issued as `issuer` says.
fn add_device(cloud: &Cloud, account: AccountId, name: &str, issuer: &Issuer) -> Keyed {
    let signing = SigningKey::generate();
    let hpke = HpkeKey::generate();
    let id = DeviceId::from_signing_key(&signing.public_key());
    let body = |issuer| CertBody {
        account,
        device: id,
        kind: DeviceKind::Client,
        signing: signing.public_key(),
        hpke: hpke.public_key(),
        issuer,
        issued: now(),
        ssh: None,
        name: name.to_owned(),
        unknown: Unknown::default(),
    };
    let cert = match issuer {
        Issuer::Device(person) => Some(issue(&body(Some(person.device)), &person.key, &signing)),
        Issuer::Server => Some(issue(&body(None), &SigningKey::generate(), &signing)),
        Issuer::Nobody => None,
    }
    .map(|cert| cert.expect("a certificate").to_cbor());
    let bytes = |bytes: &[u8]| format!("'\\x{}'::bytea", hex(bytes));
    let columns = [
        format!("'{id}'::uuid"),
        format!("'{account}'::uuid"),
        "'client'".to_owned(),
        bytes(&signing.public_key()),
        bytes(&hpke.public_key()),
        bytes(name.as_bytes()),
        cert.as_deref().map_or_else(|| "NULL".to_owned(), bytes),
    ];
    cloud.execute(&format!(
        "INSERT INTO devices (id, account_id, kind, signing, hpke, name, cert) VALUES ({})",
        columns.join(", ")
    ));
    Keyed { id, hpke }
}

/// `bytes` in lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A space of Ada's and the one channel Ada's machine created in it.
struct Channel {
    space: SpaceId,
    id: ChannelId,
}

/// Ada's machine creates `acme`, whose revision 2 adds `members`, and a channel in it; revision 4,
/// which Ada's device signs and the server stores while the machine has not accepted it yet, adds
/// `joining` to the channel and has each member's account of `excluded` bar its device.
fn acme(
    cloud: &Cloud,
    ada: &Person,
    members: &[&Person],
    joining: &[&Person],
    excluded: &[(AccountId, DeviceId)],
) -> Channel {
    let first = create(cloud, ada, ACME);
    let space = first.body().space;
    let second = next_by(&first, ada, |list| {
        list.extend(members.iter().map(|person| person.member(Role::Member)));
    });
    store(cloud, &ada.token, &second);
    let created = run(&ada.machine, &[CREATE_CHANNEL, ACME]);
    assert!(created.success, "a channel: {created:?}");
    let (third, id) = last_channel(cloud, ada, space);
    if !joining.is_empty() || !excluded.is_empty() {
        let fourth = next_with(&third, (ada.device, &ada.key), |body| {
            let channel = body
                .channels
                .iter_mut()
                .find(|channel| channel.id == id)
                .expect("the channel");
            channel
                .members
                .extend(joining.iter().map(|person| ChannelMember {
                    account: person.account,
                    history: false,
                }));
            let mut members = body.members.clone().into_vec();
            for (account, device) in excluded {
                let member = members
                    .iter_mut()
                    .find(|member| member.account == *account)
                    .expect("a member");
                member.excluded = Some(NonEmpty::new(*device));
            }
            body.members = NonEmpty::from_vec(members).expect("a member");
        });
        store(cloud, &ada.token, &fourth);
    }
    Channel { space, id }
}

/// The last revision of `space` the server holds and the ID of its last channel.
fn last_channel(cloud: &Cloud, person: &Person, space: SpaceId) -> (Roster, ChannelId) {
    let (_, last) = cloud
        .revisions(&person.token, space)
        .pop()
        .expect("a revision");
    let id = last.body().channels.last().expect("a channel").id;
    (last, id)
}

/// The recipient device of `sent`, an envelope under `hpke` to one device.
fn recipient(sent: &Sent) -> DeviceId {
    let header = &sent.envelope.envelope().header;
    match header.recipients.as_deref() {
        Some([device]) => *device,
        other => panic!("not one recipient device: {other:?}"),
    }
}

fn msg_id(sent: &Sent) -> MessageId {
    sent.envelope.envelope().header.msg_id
}

/// The `send` frames the front saw after its first `from`.
fn sent_after(front: &Front, from: usize) -> Vec<Sent> {
    front.sent().split_off(from)
}

/// The distinct envelopes of `sent` to `device`, by their bytes, in the order they were first
/// sent.
fn envelopes_to(sent: &[Sent], device: DeviceId) -> Vec<Sent> {
    let mut envelopes: Vec<Sent> = Vec::new();
    for sent in sent.iter().filter(|sent| recipient(sent) == device) {
        if !envelopes
            .iter()
            .any(|seen| seen.envelope.bytes() == sent.envelope.bytes())
        {
            envelopes.push(sent.clone());
        }
    }
    envelopes
}

/// The one envelope of `sent` to `device`; fails unless every send to `device` carries the same
/// bytes.
#[track_caller]
fn only_envelope_to(sent: &[Sent], device: DeviceId) -> Sent {
    let mut envelopes = envelopes_to(sent, device);
    assert_eq!(envelopes.len(), 1, "envelopes to {device}: {envelopes:?}");
    envelopes.remove(0)
}

/// The two envelopes of `sent` to `device`, the one `left` sent first with the same bytes, then
/// another.
#[track_caller]
fn left_then_new(sent: &[Sent], device: DeviceId, left: &[u8]) -> Sent {
    let mut envelopes = envelopes_to(sent, device);
    assert_eq!(envelopes.len(), 2, "envelopes to {device}: {envelopes:?}");
    assert_eq!(
        envelopes[0].envelope.bytes(),
        left,
        "not the left envelope first, with the same bytes"
    );
    envelopes.remove(1)
}

/// The content of `sent`, opened with `to`'s HPKE key and verified with the sender's certificate
/// `sender`.
fn open(sent: &Sent, to: &Keyed, sender: &Cert) -> Content {
    let envelope = sent.envelope.envelope();
    open_message(&envelope.header, &envelope.payload, to.id, &to.hpke)
        .unwrap_or_else(|err| panic!("does not open for {}: {err:?}", to.id))
        .verify(sender)
        .unwrap_or_else(|err| panic!("not signed by the sending device: {err:?}"))
}

/// Fails unless `sent` is an `hpke` envelope of `channel` with `report` true, sealed to `to`'s key
/// alone of `others`' keys.
fn assert_sealed_for(sent: &Sent, channel: &Channel, to: &Keyed, others: &[&Keyed]) {
    let envelope = sent.envelope.envelope();
    let header = &envelope.header;
    assert_eq!(header.scheme, Scheme::Hpke, "{header:?}");
    assert_eq!(
        (header.space, header.channel),
        (channel.space, channel.id),
        "{header:?}"
    );
    assert_eq!(recipient(sent), to.id, "{header:?}");
    assert!(sent.report, "sent with report false: {header:?}");
    for other in others.iter().filter(|other| other.id != to.id) {
        assert!(
            open_message(header, &envelope.payload, to.id, &other.hpke).is_err(),
            "the envelope to {} opens with the key of {}",
            to.id,
            other.id
        );
    }
}

/// Fails unless `content` is what a person or an agent at `from` asked for: `hops` 0, no `to`,
/// `counter` `counter` and an `a2a` body, an A2A message whose `messageId` is `msg_id` and whose
/// parts hold `text`.
#[track_caller]
fn assert_content(content: &Content, from: &str, counter: u64, msg_id: MessageId, text: &str) {
    assert_eq!(content.from.to_string(), from, "{content:?}");
    assert_eq!(content.to, None, "{content:?}");
    assert_eq!(content.hops, 0, "{content:?}");
    assert_eq!(
        content.counter.map(|counter| counter.get()),
        Some(counter),
        "{content:?}"
    );
    let Some(Body::A2a(json)) = &content.body else {
        panic!("not an a2a body: {content:?}");
    };
    let message: Value = serde_json::from_str(json).expect("an A2A message in JSON");
    assert_eq!(
        message["messageId"].as_str(),
        Some(msg_id.to_string().as_str()),
        "{message}"
    );
    let parts = message["parts"].as_array().expect("the message's parts");
    assert!(
        parts.iter().any(|part| part["text"].as_str() == Some(text)),
        "no part holds the text: {message}"
    );
}

/// The person address of `person` in `space`.
fn person_address(space: &str, person: &Person) -> String {
    format!("{space}/{}/user@{HOST}", person.handle)
}

/// What the server's database holds, in lowercase, byte strings in hex.
fn database(cloud: &Cloud) -> String {
    cloud.rows().concat().to_lowercase()
}

/// Fails unless the server's database holds each of `envelopes` as its bytes.
#[track_caller]
fn assert_held(cloud: &Cloud, envelopes: &[&Sent]) {
    let held = database(cloud);
    for sent in envelopes {
        assert!(
            held.contains(&hex(sent.envelope.bytes())),
            "the server does not hold the envelope to {}",
            recipient(sent)
        );
    }
}

/// Fails unless `finished` succeeded, said `msg_id`, and said `stored` for each of `devices`.
#[track_caller]
fn assert_says_stored(finished: &Finished, msg_id: MessageId, devices: &[DeviceId]) {
    assert!(finished.success, "{finished:?}");
    let shown = finished.shown();
    assert!(
        shown.contains(&msg_id.to_string()),
        "no message ID: {finished:?}"
    );
    for device in devices {
        let id = device.to_string();
        assert!(
            shown
                .lines()
                .any(|line| line.contains(&id) && line.to_lowercase().contains(STORED)),
            "{id} is not said to be stored: {finished:?}"
        );
    }
}

/// Fails unless `finished` failed, naming `msg_id` and each of `devices`, the envelopes it was
/// left with.
#[track_caller]
fn assert_left_with(finished: &Finished, msg_id: MessageId, devices: &[DeviceId]) {
    assert!(!finished.success, "{finished:?}");
    let shown = finished.shown();
    assert!(
        shown.contains(&msg_id.to_string()),
        "no message ID: {finished:?}"
    );
    for device in devices {
        assert!(
            shown.contains(&device.to_string()),
            "{device} is not named: {finished:?}"
        );
    }
}

/// Waits for up to [`WAIT`] until `done` holds, then kills `run`; fails if `run` exits first.
fn kill_once(mut run: Run, what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        run.assert_running();
        assert!(Instant::now() < deadline, "no {what}: {}", run.shown());
        thread::sleep(Duration::from_millis(20));
    }
    drop(run);
}

/// Ada, Bob and the channel of `acme` they are both in, Ada's machine retrying for [`TIMEOUT`].
struct Pair {
    world: World,
    front: Front,
    ada: Person,
    bob: Person,
    bob_keyed: Keyed,
    ada_cert: Cert,
    channel: Channel,
}

impl Pair {
    fn start(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let (world, front) = fronted();
        let ada = retrying_for_the_timeout(world.sign_up(ids[0], logins[0], "ada-workstation"));
        let bob = world.sign_up(ids[1], logins[1], "bob-workstation");
        let cloud = &world.cloud;
        let channel = acme(cloud, &ada, &[&bob], &[&bob], &[]);
        Pair {
            bob_keyed: keyed(cloud, &bob),
            ada_cert: cert_of(cloud, &ada),
            front,
            ada,
            bob,
            channel,
            world,
        }
    }

    fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// Runs `skald send` of `text` on Ada's machine.
    fn send(&self, text: &str) -> Finished {
        send(&self.ada.machine, self.channel.id, text)
    }

    /// The content of `sent`, an envelope to Bob's device.
    fn open(&self, sent: &Sent) -> Content {
        open(sent, &self.bob_keyed, &self.ada_cert)
    }

    /// Fails unless `content`, of `msg_id`, is Ada's `text` with `counter`.
    #[track_caller]
    fn assert_from_ada(&self, content: &Content, counter: u64, msg_id: MessageId, text: &str) {
        let from = person_address(ACME, &self.ada);
        assert_content(content, &from, counter, msg_id, text);
    }

    /// Ada's next `skald send` of [`SECOND`] first sends `first`, the envelope to Bob's device the
    /// last run left, with the same bytes, then the new message: the server holds one envelope to
    /// Bob's device for each, with the counters 1 and 2, and the run says the new one is stored.
    fn assert_sends_what_is_left_first(&self, first: &Sent) {
        let bob = self.bob.device;
        let from = self.front.sent().len();

        let second = self.send(SECOND);

        assert!(second.success, "{second:?}");
        let next = left_then_new(&sent_after(&self.front, from), bob, first.envelope.bytes());
        let (first_id, next_id) = (msg_id(first), msg_id(&next));
        assert_ne!(first_id, next_id, "the new message has the left one's ID");
        self.assert_from_ada(&self.open(first), 1, first_id, FIRST);
        self.assert_from_ada(&self.open(&next), 2, next_id, SECOND);
        assert_says_stored(&second, next_id, &[bob]);
        assert_held(self.cloud(), &[first, &next]);
        let all = envelopes_to(&self.front.sent(), bob);
        assert_eq!(all.len(), 2, "envelopes to Bob's device: {all:?}");
    }
}

/// Ada's device sends to a channel whose members are Ada's account and Bob's, Bob added by a
/// revision Ada's machine has not accepted yet. The server holds one envelope for each admitted
/// device of the channel, Bob's and Ada's second one, each opening only with that device's key,
/// with the person address, `hops` 0, `counter` 1 and the text in an A2A message whose
/// `messageId` is the `msg_id`; nothing in the database holds the text. None goes to Ada's own
/// device, to Bob's devices without a valid certificate, one only the server certified and one
/// with none, to Bob's device his member's `excluded` lists, or to Carol's, a member of the space
/// outside the channel. The run says the message's ID and that each envelope is stored.
#[test]
fn message_is_sealed_once_for_each_admitted_device_of_the_channel() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_122_061_001, "ada-sends-standin", "ada-workstation");
    let bob = world.sign_up(4_122_061_002, "bob-receives-standin", "bob-workstation");
    let carol = world.sign_up(4_122_061_003, "carol-outside-standin", "carol-laptop");
    let laptop = add_device(cloud, ada.account, "ada-laptop", &Issuer::Device(&ada));
    let barred = add_device(cloud, bob.account, "bob-barred", &Issuer::Device(&bob));
    let forged = add_device(cloud, bob.account, "bob-forged", &Issuer::Server);
    let unlinked = add_device(cloud, bob.account, "bob-unlinked", &Issuer::Nobody);
    let channel = acme(
        cloud,
        &ada,
        &[&bob, &carol],
        &[&bob],
        &[(bob.account, barred.id)],
    );
    let (own, bob_keyed, carol_keyed) =
        (keyed(cloud, &ada), keyed(cloud, &bob), keyed(cloud, &carol));
    let keys = [
        &own,
        &laptop,
        &bob_keyed,
        &carol_keyed,
        &barred,
        &forged,
        &unlinked,
    ];
    let ada_cert = cert_of(cloud, &ada);
    let from = front.sent().len();

    let finished = send(&ada.machine, channel.id, FIRST);

    assert!(finished.success, "{finished:?}");
    let sent = sent_after(&front, from);
    let recipients: BTreeSet<DeviceId> = sent.iter().map(recipient).collect();
    assert_eq!(
        recipients,
        BTreeSet::from([bob.device, laptop.id]),
        "the recipient devices"
    );
    let envelopes = [
        only_envelope_to(&sent, bob.device),
        only_envelope_to(&sent, laptop.id),
    ];
    let id = msg_id(&envelopes[0]);
    assert_eq!(msg_id(&envelopes[1]), id, "one msg_id for the message");
    let from_ada = person_address(ACME, &ada);
    for (envelope, to) in envelopes.iter().zip([&bob_keyed, &laptop]) {
        assert_sealed_for(envelope, &channel, to, &keys);
        let content = open(envelope, to, &ada_cert);
        assert_content(&content, &from_ada, 1, id, FIRST);
    }
    assert_held(cloud, &envelopes.iter().collect::<Vec<_>>());
    assert!(
        !keys::holds(cloud.rows().concat().as_bytes(), FIRST.as_bytes()),
        "the database holds the text"
    );
    assert_says_stored(&finished, id, &[bob.device, laptop.id]);
}

/// With every `stored` lost by the front, the run retries with the same bytes until its timeout,
/// then fails naming the message and Bob's device; the server stored the envelope. Ada's next
/// `skald send` sends it first, with the same bytes, and the server holds one envelope for it.
#[test]
fn envelope_whose_stored_was_lost_is_sent_again_with_the_same_bytes() {
    let pair = Pair::start(
        [4_122_061_101, 4_122_061_102],
        ["ada-loses-standin", "bob-waits-standin"],
    );
    pair.front.answer(Answers::Lose);

    let first = pair.send(FIRST);

    assert!(pair.front.lost() > 0, "no stored was lost: {first:?}");
    let left = only_envelope_to(&pair.front.sent(), pair.bob.device);
    assert_left_with(&first, msg_id(&left), &[pair.bob.device]);
    assert_held(pair.cloud(), &[&left]);
    pair.front.answer(Answers::Forward);
    pair.assert_sends_what_is_left_first(&left);
}

/// The CLI killed after the server stored its envelope and before it got `stored`: Ada's next
/// `skald send` sends that envelope first, with the same bytes, and the server holds one envelope
/// for it.
#[test]
fn envelope_of_a_cli_killed_before_stored_is_sent_again_with_the_same_bytes() {
    let pair = Pair::start(
        [4_122_061_201, 4_122_061_202],
        ["ada-killed-standin", "bob-patient-standin"],
    );
    pair.front.answer(Answers::Lose);
    let run = pair
        .ada
        .machine
        .start(&strs(&send_args(pair.channel.id, FIRST)));
    kill_once(run, "lost stored", || pair.front.lost() > 0);

    let left = only_envelope_to(&pair.front.sent(), pair.bob.device);
    pair.front.answer(Answers::Forward);
    pair.assert_sends_what_is_left_first(&left);
}

/// The CLI killed after it sent its envelope and before the server got it: the outbox holds the
/// envelope, and Ada's next `skald send` sends it first, with the same bytes.
#[test]
fn envelope_of_a_cli_killed_before_the_server_got_it_is_sent_from_the_outbox() {
    let pair = Pair::start(
        [4_122_061_301, 4_122_061_302],
        ["ada-cut-standin", "bob-later-standin"],
    );
    pair.front.swallow(true);
    let run = pair
        .ada
        .machine
        .start(&strs(&send_args(pair.channel.id, FIRST)));
    kill_once(run, "swallowed send", || pair.front.swallowed() > 0);

    let left = only_envelope_to(&pair.front.sent(), pair.bob.device);
    assert!(
        !database(pair.cloud()).contains(&hex(left.envelope.bytes())),
        "the server got the swallowed envelope"
    );
    pair.front.swallow(false);
    pair.assert_sends_what_is_left_first(&left);
}

/// With the server down for delivery, the run retries for its timeout, then fails naming what is
/// left: the message and Bob's device. Once the server is back, Ada's next `skald send` sends that
/// first, then the new message.
#[test]
fn run_with_the_server_down_fails_after_the_timeout_naming_what_is_left() {
    let pair = Pair::start(
        [4_122_061_401, 4_122_061_402],
        ["ada-offline-standin", "bob-online-standin"],
    );
    pair.front.down(true);
    let started = Instant::now();

    let first = pair.send(FIRST);

    let took = started.elapsed();
    assert!(!first.success, "{first:?}");
    assert!(
        took >= TIMEOUT,
        "failed after {took:?}, before the timeout: {first:?}"
    );
    assert!(
        took < TIMEOUT + Duration::from_secs(30),
        "retried for {took:?}: {first:?}"
    );
    assert!(
        pair.front.refused() >= 2,
        "tried {} times: {first:?}",
        pair.front.refused()
    );
    assert!(pair.front.sent().is_empty(), "a send got through");
    pair.front.down(false);
    let from = pair.front.sent().len();
    let second = pair.send(SECOND);
    assert!(second.success, "{second:?}");
    let sent = sent_after(&pair.front, from);
    let envelopes = envelopes_to(&sent, pair.bob.device);
    assert_eq!(
        envelopes.len(),
        2,
        "the left envelope, then the new one: {envelopes:?}"
    );
    let (left, next) = (&envelopes[0], &envelopes[1]);
    assert_left_with(&first, msg_id(left), &[pair.bob.device]);
    pair.assert_from_ada(&pair.open(left), 1, msg_id(left), FIRST);
    pair.assert_from_ada(&pair.open(next), 2, msg_id(next), SECOND);
    assert_says_stored(&second, msg_id(next), &[pair.bob.device]);
    assert_held(pair.cloud(), &[left, next]);
}

/// Counters go up by one per recipient device across runs: Bob's device gets 1, 2 and 3; Ada's
/// second device, added after the first message, 1 and 2.
#[test]
fn counters_go_up_by_one_per_recipient_device_across_runs() {
    let pair = Pair::start(
        [4_122_061_501, 4_122_061_502],
        ["ada-counts-standin", "bob-counted-standin"],
    );
    let mut laptop = None;
    for (bob_counter, text) in [(1, FIRST), (2, SECOND), (3, THIRD)] {
        if bob_counter == 2 {
            let issuer = Issuer::Device(&pair.ada);
            laptop = Some(add_device(
                pair.cloud(),
                pair.ada.account,
                "ada-laptop",
                &issuer,
            ));
        }
        let from = pair.front.sent().len();

        let finished = pair.send(text);

        assert!(finished.success, "{finished:?}");
        let sent = sent_after(&pair.front, from);
        let to_bob = only_envelope_to(&sent, pair.bob.device);
        let id = msg_id(&to_bob);
        pair.assert_from_ada(&pair.open(&to_bob), bob_counter, id, text);
        let mut stored = vec![pair.bob.device];
        if let Some(laptop) = &laptop {
            let to_laptop = only_envelope_to(&sent, laptop.id);
            let content = open(&to_laptop, laptop, &pair.ada_cert);
            pair.assert_from_ada(&content, bob_counter - 1, id, text);
            stored.push(laptop.id);
        }
        assert_says_stored(&finished, id, &stored);
    }
}

/// In a channel without another admitted device, Bob a member of the space outside it and Ada's
/// second device without a certificate, the run sends nothing, fails and says no device can
/// receive the message.
#[test]
fn channel_without_another_admitted_device_sends_nothing() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_122_061_601, "ada-alone-standin", "ada-workstation");
    let bob = world.sign_up(4_122_061_602, "bob-elsewhere-standin", "bob-workstation");
    add_device(cloud, ada.account, "ada-unlinked", &Issuer::Nobody);
    let channel = acme(cloud, &ada, &[&bob], &[], &[]);

    let finished = send(&ada.machine, channel.id, FIRST);

    assert!(!finished.success, "{finished:?}");
    assert!(
        finished.shown().to_lowercase().contains(NO_DEVICE),
        "does not say no device can receive it: {finished:?}"
    );
    assert!(
        front.sent().is_empty(),
        "sent {} envelopes",
        front.sent().len()
    );
}

/// An envelope the server refuses, with a problem the client knows or one it does not, is not sent
/// again; the run shows its problem. Ada's next `skald send` sends only the new message.
#[test]
fn refused_envelope_is_not_sent_again_and_its_problem_is_shown() {
    let pair = Pair::start(
        [4_122_061_701, 4_122_061_702],
        ["ada-refused-standin", "bob-unreached-standin"],
    );
    let mut refused = Vec::new();
    for (problem, text) in [("not-a-member", FIRST), ("made-up-problem", SECOND)] {
        pair.front.answer(Answers::Refuse(problem.to_owned()));
        let from = pair.front.sent().len();

        let finished = pair.send(text);

        let sent = sent_after(&pair.front, from);
        assert!(!sent.is_empty(), "nothing sent: {finished:?}");
        assert!(
            sent.iter().all(|sent| !refused.contains(&msg_id(sent))),
            "a refused envelope is sent again"
        );
        let id = msg_id(&only_envelope_to(&sent, pair.bob.device));
        assert!(
            finished.shown().contains(problem),
            "{problem} is not shown: {finished:?}"
        );
        refused.push(id);
    }
    pair.front.answer(Answers::Forward);
    let from = pair.front.sent().len();

    let finished = pair.send(THIRD);

    let sent = sent_after(&pair.front, from);
    assert!(
        sent.iter().all(|sent| !refused.contains(&msg_id(sent))),
        "a refused envelope is sent again"
    );
    let next = only_envelope_to(&sent, pair.bob.device);
    pair.assert_from_ada(&pair.open(&next), 3, msg_id(&next), THIRD);
    assert_says_stored(&finished, msg_id(&next), &[pair.bob.device]);
}

/// As an agent in `~/work/clientA`, a message to clienta's channel has `from` the agent's address
/// in clienta; one to clientb's channel is refused, nothing is sent, and the output names nothing
/// of clientb.
#[test]
fn agent_sends_only_in_the_space_of_its_folder() {
    let (world, front) = fronted();
    let desk = Desk::sign_up(world, 4_122_061_801, "ada-agent-standin");
    let (cloud, ada) = (&desk.world.cloud, &desk.ada);
    let mut channels = Vec::new();
    for name in [CLIENT_A, CLIENT_B] {
        let created = run(&ada.machine, &[CREATE_CHANNEL, name]);
        assert!(created.success, "a channel in {name}: {created:?}");
        let space = space_id(cloud, &ada.token, name);
        channels.push(Channel {
            space,
            id: last_channel(cloud, ada, space).1,
        });
    }
    let laptop = add_device(cloud, ada.account, "ada-laptop", &Issuer::Device(ada));
    let folder = desk.dir("work/clientA");
    desk.add_rule(&folder, CLIENT_A);
    let as_agent = |dir: &Path, channel: ChannelId, text: &str| {
        let mut args = vec![AGENT.to_owned(), CLAUDE.to_owned()];
        args.extend(send_args(channel, text));
        ada.machine.start_in(dir, &strs(&args)).finish()
    };
    let from = front.sent().len();

    let own = as_agent(&folder, channels[0].id, FIRST);

    assert!(own.success, "{own:?}");
    let sent = sent_after(&front, from);
    let envelope = only_envelope_to(&sent, laptop.id);
    assert_eq!(
        sent.iter().map(recipient).collect::<BTreeSet<_>>(),
        BTreeSet::from([laptop.id])
    );
    assert_sealed_for(&envelope, &channels[0], &laptop, &[&keyed(cloud, ada)]);
    let content = open(&envelope, &laptop, &cert_of(cloud, ada));
    let address = format!("{CLIENT_A}/{}/{CLAUDE}@{HOST}", ada.handle);
    assert_content(&content, &address, 1, msg_id(&envelope), FIRST);
    assert_says_stored(&own, msg_id(&envelope), &[laptop.id]);

    let from = front.sent().len();
    let other = as_agent(&folder, channels[1].id, SECOND);

    assert!(!other.success, "{other:?}");
    assert!(
        sent_after(&front, from).is_empty(),
        "sent to clientb's channel"
    );
    let shown = other.shown();
    assert!(
        !shown.contains(CLIENT_B) && !shown.contains(&channels[1].space.to_string()),
        "the refusal names clientb: {other:?}"
    );
}
