//! Sending a message in the CLI: `skald send` sends a text to a channel, by its ID, of a space the
//! device follows; as an agent, only in the space of its folder. It first accepts the space's
//! latest roster revisions and brings the channel's group in line, then encrypts the content in the
//! group and sends one `mls` envelope without `recipients`, with `report` true, which only the
//! group's other devices read (`docs/protocol/README.md`, "Scheme `mls`"). The content has `from`
//! the person address, or the agent's address, `hops` 0, a `counter` that counts the device's
//! messages in the channel from 1, and an `a2a` body: an A2A message with the text whose
//! `messageId` is the `msg_id` (`docs/protocol/README.md`, "Content"). The envelope goes into the
//! outbox before anything is sent; the CLI sends it over its device's connection and retries with
//! the same bytes until it gets `stored` or `send-refused`, for as long as its request timeout,
//! then names what is left and fails; a later `skald send` sends that first
//! (`docs/protocol/delivery.md`, "Send").
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay and, before it, the front of `front.rs` at the server's public address,
//! `http://localhost:<port>`; a test plays the person on the verification page in headless Chrome.
//! Each machine keeps its state in a directory of its own and its secrets in a file there. The
//! tests sign fixture revisions with the keys of the machines' devices, read from their secrets
//! files, and store them through the API, as an admin's device would. Devices beside the machines'
//! are written to the server's database, as linking will register them. The other members' devices
//! are played by the tests with an MLS state of their own (`channel_groups/played.rs`), reading the
//! messages they decrypt.
//!
//! The CLI, as these tests use it, beside the commands of the tests of roster revisions, of
//! creating channels and of agent isolation:
//! - `skald send --all --channel <channel> <text>`: sends `<text>` to the channel whose ID is
//!   `<channel>`; it says the message's ID and a line that says `stored` once the server has stored
//!   its envelope, or the problem the server refused it with; as an agent, once stored, only
//!   `sent #<ref>`, as the tests of compact output say (`compact_output/main.rs`);
//! - a run that is left with an envelope the server has not answered fails, its output holding the
//!   message's ID;
//! - with no other device in the channel's group, it fails and says `no device` can receive the
//!   message, sending nothing;
//! - `SKALD_TIMEOUT`: the request timeout, in seconds, which a run retries for.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
mod fixtures;
mod front;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../channel_groups/kit.rs"]
mod kit;
mod limits;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../channel_groups/played.rs"]
mod played;
mod rate;
mod rate_said;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../channel_messages/talk.rs"]
mod talk;
#[path = "../roster_revisions/world.rs"]
mod world;

use std::time::{Duration, Instant};

use cloud::Cloud;
use fixtures::{
    Issuer, TIMEOUT, acme, add_device, database, fronted, hex, kill_once, msg_id, person_address,
    send, send_args, sent_after, strs,
};
use front::{Answers, Sent};
use kit::{Change, create_space, latest, revise, with_channel};
use lines::assert_says_sent;
use machine::Finished;
use played::Played;
use skald_proto::{MessageId, Role, Scheme};
use talk::{CAROL_HOST, Talk, assert_read, messages_sent, one_message_sent};
use world::{ACME, ADD_RULE, AGENT, CLIENT_A, CLIENT_B, HOST, World, dir_of, path, run};

/// What the CLI says of an envelope the server stored, and when no device can receive a message.
const STORED: &str = "stored";
const NO_DEVICE: &str = "no device";

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "deploy the staging build at nine-forty";
const SECOND: &str = "the staging build is green, ship it";
const THIRD: &str = "rolling back the canary in five minutes";

/// The agent the tests send as.
const CLAUDE: &str = "claude-k3s8vq";

/// Fails unless `sent` holds no envelope under `hpke`.
#[track_caller]
fn assert_all_mls(sent: &[Sent]) {
    for one in sent {
        let header = &one.envelope.envelope().header;
        assert_eq!(header.scheme, Scheme::Mls, "{header:?}");
    }
}

/// Fails unless the server's database holds each of `envelopes` as its bytes.
#[track_caller]
fn assert_held(cloud: &Cloud, envelopes: &[&Sent]) {
    let held = database(cloud);
    for sent in envelopes {
        assert!(
            held.contains(&hex(sent.envelope.bytes())),
            "the server does not hold the envelope of {}",
            msg_id(sent)
        );
    }
}

/// Fails unless `finished` succeeded, said `msg_id`, and said `stored`.
#[track_caller]
fn assert_says_stored(finished: &Finished, msg_id: MessageId) {
    assert!(finished.success, "{finished:?}");
    let shown = finished.shown();
    assert!(
        shown.contains(&msg_id.to_string()),
        "no message ID: {finished:?}"
    );
    assert!(
        shown.to_lowercase().contains(STORED),
        "the message is not said to be stored: {finished:?}"
    );
}

/// Fails unless `finished` failed, naming `msg_id`, the message it was left with.
#[track_caller]
fn assert_left_with(finished: &Finished, msg_id: MessageId) {
    assert!(!finished.success, "{finished:?}");
    assert!(
        finished.shown().contains(&msg_id.to_string()),
        "no message ID: {finished:?}"
    );
}

/// The one message envelope `talk`'s front saw sent in its channel; fails unless there is one.
#[track_caller]
fn only_message(talk: &Talk) -> Sent {
    let mut sent = messages_sent(&talk.front, talk.channel, 0);
    assert_eq!(sent.len(), 1, "not one message: {sent:?}");
    sent.remove(0)
}

/// Ada's next `skald send` of [`SECOND`] first sends the message of [`FIRST`] the last run left,
/// with the same bytes as `left`, if given, then the new one: the server holds both, the run says
/// the new one is stored, and Bob's device reads each once, with the counters 1 and 2. Gives the
/// left one.
fn assert_sends_what_is_left_first(talk: &mut Talk, left: Option<&Sent>) -> Sent {
    let from = talk.front.sent().len();

    let second = talk.send(SECOND);

    let [first, next]: [Sent; 2] = messages_sent(&talk.front, talk.channel, from)
        .try_into()
        .unwrap_or_else(|sent| panic!("not the left message, then the new one: {sent:?}"));
    if let Some(left) = left {
        assert_eq!(
            first.envelope.bytes(),
            left.envelope.bytes(),
            "not the left envelope first, with the same bytes"
        );
    }
    let (first_id, next_id) = (msg_id(&first), msg_id(&next));
    assert_ne!(first_id, next_id, "the new message has the left one's ID");
    assert_says_stored(&second, next_id);
    assert_held(talk.cloud(), &[&first, &next]);
    talk.bob_reads_from_ada(&[(first_id, 1, FIRST), (next_id, 2, SECOND)]);
    let all = messages_sent(&talk.front, talk.channel, 0);
    assert_eq!(all.len(), 2, "messages in the channel: {all:?}");
    first
}

/// Ada's device sends to a channel of `acme` whose members are Ada's account and Bob's, Bob added
/// by a revision Ada's machine has not accepted yet; Carol, a member of the space, is outside the
/// channel. The run adds Bob's device to the channel's group, then sends one `mls` envelope, and no
/// `hpke` one, with `report` true and without `recipients`: the server holds it, and nothing in the
/// database holds the text. Bob's device reads it once, with the person address, `hops` 0,
/// `counter` 1 and the text in an A2A message whose `messageId` is the `msg_id`; Carol's device
/// reads nothing in the channel. The run says the message's ID and that it is stored.
#[test]
fn message_is_encrypted_once_for_the_group_of_the_channel() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_122_061_001, "ada-sends-standin", "ada-workstation");
    let bob = world.sign_up(4_122_061_002, "bob-receives-standin", "bob-workstation");
    let mut bob = Played::new(cloud, bob);
    let carol = world.sign_up(4_122_061_003, "carol-outside-standin", CAROL_HOST);
    let mut carol = Played::new(cloud, carol);
    let people = [&bob.person, &carol.person];
    let channel = acme(cloud, &ada, &people, &[&bob.person], &[]);
    let from = front.sent().len();

    let finished = send(&ada.machine, channel.id, FIRST);

    assert!(finished.success, "{finished:?}");
    assert_all_mls(&sent_after(&front, from));
    let sent = messages_sent(&front, channel.id, from);
    assert_eq!(sent.len(), 1, "not one message: {sent:?}");
    let header = &sent[0].envelope.envelope().header;
    assert_eq!(
        (header.space, header.recipients.as_ref()),
        (channel.space, None),
        "{header:?}"
    );
    assert!(sent[0].report, "sent with report false: {header:?}");
    assert_held(cloud, &[&sent[0]]);
    assert!(
        !keys::holds(cloud.rows().concat().as_bytes(), FIRST.as_bytes()),
        "the database holds the text"
    );
    let id = header.msg_id;
    assert_says_stored(&finished, id);
    let from_ada = person_address(ACME, &ada);
    bob.take_in(cloud);
    assert_read(&bob, channel.id, (ada.device, &from_ada), &[(id, 1, FIRST)]);
    carol.take_in(cloud);
    assert!(
        carol.read_in(channel.id).is_empty(),
        "Carol's device read a message of the channel"
    );
}

/// With every `stored` lost by the front, the run retries with the same bytes until its timeout,
/// then fails naming the message, and no broken connection; the server stored the envelope.
/// Ada's next `skald send` sends it first, with the same bytes, and the server holds one envelope
/// for it.
#[test]
fn envelope_whose_stored_was_lost_is_sent_again_with_the_same_bytes() {
    let mut talk = Talk::new(
        &[4_122_061_101, 4_122_061_102],
        &["ada-loses-standin", "bob-waits-standin"],
        true,
    );
    talk.front.answer(Answers::Lose);

    let first = talk.send(FIRST);

    assert!(talk.front.lost() > 0, "no stored was lost: {first:?}");
    let left = only_message(&talk);
    assert_left_with(&first, msg_id(&left));
    assert!(
        !first.shown().contains("connection to the server broke"),
        "{first:?}"
    );
    assert_held(talk.cloud(), &[&left]);
    talk.front.answer(Answers::Forward);
    assert_sends_what_is_left_first(&mut talk, Some(&left));
}

/// The CLI killed after the server stored its envelope and before it got `stored`: Ada's next
/// `skald send` sends that envelope first, with the same bytes, and the server holds one envelope
/// for it.
#[test]
fn envelope_of_a_cli_killed_before_stored_is_sent_again_with_the_same_bytes() {
    let mut talk = Talk::new(
        &[4_122_061_201, 4_122_061_202],
        &["ada-killed-standin", "bob-patient-standin"],
        true,
    );
    talk.front.answer(Answers::Lose);
    let run = talk
        .ada
        .machine
        .start(&strs(&send_args(talk.channel, FIRST)));
    kill_once(run, "lost stored of the message", || talk.front.lost() > 0);

    let left = only_message(&talk);
    talk.front.answer(Answers::Forward);
    assert_sends_what_is_left_first(&mut talk, Some(&left));
}

/// The CLI killed after it sent its envelope and before the server got it: the outbox holds the
/// envelope, and Ada's next `skald send` sends it first, with the same bytes.
#[test]
fn envelope_of_a_cli_killed_before_the_server_got_it_is_sent_from_the_outbox() {
    let mut talk = Talk::new(
        &[4_122_061_301, 4_122_061_302],
        &["ada-cut-standin", "bob-later-standin"],
        true,
    );
    talk.front.swallow(true);
    let run = talk
        .ada
        .machine
        .start(&strs(&send_args(talk.channel, FIRST)));
    kill_once(run, "swallowed send of the message", || {
        !messages_sent(&talk.front, talk.channel, 0).is_empty()
    });

    let left = only_message(&talk);
    assert!(
        !database(talk.cloud()).contains(&hex(left.envelope.bytes())),
        "the server got the swallowed envelope"
    );
    talk.front.swallow(false);
    assert_sends_what_is_left_first(&mut talk, Some(&left));
}

/// With the server down for delivery, the run retries for its timeout, then fails naming the
/// message it is left with. Once the server is back, Ada's next `skald send` sends that first, then
/// the new message.
#[test]
fn run_with_the_server_down_fails_after_the_timeout_naming_what_is_left() {
    let mut talk = Talk::new(
        &[4_122_061_401, 4_122_061_402],
        &["ada-offline-standin", "bob-online-standin"],
        true,
    );
    talk.front.down(true);
    let started = Instant::now();

    let first = talk.send(FIRST);

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
        talk.front.refused() >= 2,
        "tried {} times: {first:?}",
        talk.front.refused()
    );
    assert!(
        messages_sent(&talk.front, talk.channel, 0).is_empty(),
        "a send got through"
    );
    talk.front.down(false);
    let left = assert_sends_what_is_left_first(&mut talk, None);
    assert_left_with(&first, msg_id(&left));
}

/// The counter counts Ada's device's messages in the channel across runs: Bob's device reads 1, 2
/// and 3; Carol's, whose account a revision adds to the channel after the first message, 2 and 3.
#[test]
fn counters_go_up_by_one_per_message_in_the_channel_across_runs() {
    let mut talk = Talk::new(
        &[4_122_061_501, 4_122_061_502],
        &["ada-counts-standin", "bob-counted-standin"],
        false,
    );
    let carol = talk
        .world
        .sign_up(4_122_061_503, "carol-counted-standin", CAROL_HOST);
    let mut carol = Played::new(talk.cloud(), carol);
    let mut read = Vec::new();
    for (counter, text) in [(1, FIRST), (2, SECOND), (3, THIRD)] {
        if counter == 2 {
            let previous = latest(talk.cloud(), &talk.ada.token, talk.space.id);
            let change = Change {
                members: vec![(&carol.person, Role::Member)],
                join: vec![(talk.channel, &carol.person)],
                ..Change::default()
            };
            revise(talk.cloud(), &previous, &talk.ada, &change);
        }
        let from = talk.front.sent().len();

        let finished = talk.send(text);

        let id = one_message_sent(&talk.front, talk.channel, from);
        assert_says_stored(&finished, id);
        read.push((id, counter, text));
    }
    talk.bob_reads_from_ada(&read);
    carol.take_in(talk.cloud());
    let from_ada = talk.address(&talk.ada);
    assert_read(
        &carol,
        talk.channel,
        (talk.ada.device, &from_ada),
        &read[1..],
    );
}

/// In a channel whose group holds no other device, Bob a member of the space outside it and Ada's
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
    let from = front.sent().len();

    let finished = send(&ada.machine, channel.id, FIRST);

    assert!(!finished.success, "{finished:?}");
    assert!(
        finished.shown().to_lowercase().contains(NO_DEVICE),
        "does not say no device can receive it: {finished:?}"
    );
    assert_all_mls(&sent_after(&front, from));
    let sent = messages_sent(&front, channel.id, from);
    assert!(sent.is_empty(), "sent {sent:?}");
}

/// An envelope the server refuses, with a problem the client knows or one it does not, is not sent
/// again; the run shows its problem. Ada's next `skald send` sends only the new message, which
/// Bob's device reads with the counter 3, after the refused ones: the front refuses them in place
/// of the server, which stored them.
#[test]
fn refused_envelope_is_not_sent_again_and_its_problem_is_shown() {
    let mut talk = Talk::new(
        &[4_122_061_701, 4_122_061_702],
        &["ada-refused-standin", "bob-unreached-standin"],
        false,
    );
    let mut refused = Vec::new();
    for (problem, text) in [("not-a-member", FIRST), ("made-up-problem", SECOND)] {
        talk.front.answer(Answers::Refuse(problem.to_owned()));
        let from = talk.front.sent().len();

        let finished = talk.send(text);

        let sent = sent_after(&talk.front, from);
        assert!(
            sent.iter().all(|sent| !refused.contains(&msg_id(sent))),
            "a refused envelope is sent again"
        );
        let id = one_message_sent(&talk.front, talk.channel, from);
        assert!(
            finished.shown().contains(problem),
            "{problem} is not shown: {finished:?}"
        );
        refused.push(id);
    }
    talk.front.answer(Answers::Forward);
    let from = talk.front.sent().len();

    let finished = talk.send(THIRD);

    let sent = sent_after(&talk.front, from);
    assert!(
        sent.iter().all(|sent| !refused.contains(&msg_id(sent))),
        "a refused envelope is sent again"
    );
    let next = one_message_sent(&talk.front, talk.channel, from);
    assert_says_stored(&finished, next);
    let read = [
        (refused[0], 1, FIRST),
        (refused[1], 2, SECOND),
        (next, 3, THIRD),
    ];
    talk.bob_reads_from_ada(&read);
}

/// As an agent in `~/work/clientA`, a message to clienta's channel has `from` the agent's address
/// in clienta, and the run says only `sent` and the message's reference; one to clientb's channel
/// is refused, nothing is sent, and the output names nothing of clientb.
#[test]
fn agent_sends_only_in_the_space_of_its_folder() {
    let mut talk = Talk::in_space(
        CLIENT_A,
        &[4_122_061_801, 4_122_061_802],
        &["ada-agent-standin", "bob-agent-standin"],
        false,
    );
    let cloud = talk.cloud();
    let other = create_space(cloud, &talk.ada, CLIENT_B);
    let elsewhere = with_channel(cloud, other.id, &talk.ada, &[]);
    let folder = dir_of(&talk.ada.machine, "work/clientA");
    let bound = run(&talk.ada.machine, &[ADD_RULE, path(&folder), CLIENT_A]);
    assert!(bound.success, "not bound: {bound:?}");
    let as_agent = |channel, text: &str| {
        let mut args = vec![AGENT.to_owned(), CLAUDE.to_owned()];
        args.extend(send_args(channel, text));
        talk.ada.machine.start_in(&folder, &strs(&args)).finish()
    };
    let from = talk.front.sent().len();

    let own = as_agent(talk.channel, FIRST);

    let id = one_message_sent(&talk.front, talk.channel, from);
    assert!(own.success, "{own:?}");
    assert_says_sent(&own.shown(), id);
    let address = format!("{CLIENT_A}/{}/{CLAUDE}@{HOST}", talk.ada.handle);
    let bob = &mut talk.played[0];
    bob.take_in(&talk.world.cloud);
    assert_read(
        bob,
        talk.channel,
        (talk.ada.device, &address),
        &[(id, 1, FIRST)],
    );

    let from = talk.front.sent().len();
    let refused = as_agent(elsewhere, SECOND);

    assert!(!refused.success, "{refused:?}");
    assert!(
        sent_after(&talk.front, from).is_empty(),
        "sent to clientb's channel"
    );
    let shown = refused.shown();
    for named in [CLIENT_B.to_owned(), other.id.to_string()] {
        assert!(
            !shown.contains(&named),
            "the refusal names {named}: {refused:?}"
        );
    }
}

/// A server that closes each connection for a policy violation: without a front, a machine reaches
/// the server through the relay, at another origin than the server's public address, so the server
/// refuses each proof (`signature-refused`, 1008). Ada's `skald send`, with a timeout of 20 s, tries
/// each envelope once and fails, saying why, well before the timeout.
#[test]
fn send_whose_connection_the_server_closes_for_a_policy_violation_stops_before_the_timeout() {
    let world = World::start();
    let cloud = &world.cloud;
    let timeout = Duration::from_secs(20);
    let mut ada = world.sign_up(4_122_062_001, "ada-refused-standin", "ada-workstation");
    ada.machine = ada
        .machine
        .with_env(fixtures::TIMEOUT_SETTING, timeout.as_secs().to_string());
    let bob = world.sign_up(4_122_062_002, "bob-unreached-standin", "bob-workstation");
    let second = world::create(cloud, &ada, ACME);
    let default = second.body().channels[0].id;
    let with_bob = world::next_by(&second, &ada, |members| {
        members.push(bob.member(Role::Member));
    });
    world::store(cloud, &ada.token, &with_bob);

    let started = Instant::now();
    let refused = send(&ada.machine, default, FIRST);

    let took = started.elapsed();
    assert!(!refused.success, "{refused:?}");
    assert!(
        refused.shown().contains("signature-refused"),
        "it does not say why: {refused:?}"
    );
    assert!(took < timeout / 2, "it ran for {took:?}: {refused:?}");
}
