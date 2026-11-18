//! `--json`: every field the short forms leave out, the full IDs and the reference included, as an
//! agent too.

use skald_proto::MessageId;

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::assert_deleted;
use crate::kit::RECV;
use crate::lines::{JSON_FLAG, assert_fields, holds_text, json_objects, object_of};
use crate::machine::Finished;
use crate::said::send_as;
use crate::talk::{BOB_HOST, DELIVERED, Line};
use crate::world::AGENT;

/// The flag of `skald recv` that waits for what is to come.
const WAIT_FLAG: &str = "--wait";

/// What claude and Bob's device say; none is anywhere else.
const ASKED: &str = "json test: claude asks for the build logs";
const ANSWERED: &str = "json test: bob attached the build logs";
const LATER: &str = "json test: bob adds the test report";

/// What the server answers a stored message with.
const STORED: &str = "stored";

/// Fails unless `finished` succeeded and gives, among its JSON objects, Bob's message `msg_id` with
/// `text`: its reference, the channel, Bob's address, his device's ID and name, and the text.
#[track_caller]
fn assert_json_message(agents: &Agents, finished: &Finished, (msg_id, text): (MessageId, &str)) {
    assert!(finished.success, "{finished:?}");
    let objects = json_objects(&finished.stdout);
    let message = object_of(&objects, msg_id, "text");
    let talk = &agents.talk;
    let fields = [
        ("channel", talk.channel.to_string()),
        ("from", agents.bob()),
        ("device", talk.played[0].device().to_string()),
        ("device_name", BOB_HOST.to_owned()),
        ("text", text.to_owned()),
    ];
    assert_fields(message, &fields);
}

/// Ada's device has run as claude in `~/work/acme`. As claude, `skald send --json` to the channel
/// gives the message's `msg_id`, reference and channel, stored. Bob's device reads it, then says
/// something in the channel. claude's `skald recv --json` gives Bob's message with every field, and
/// the `delivered` of claude's message with its reference, the channel and Bob's device's ID, which
/// the short forms leave out.
#[test]
fn json_gives_an_agent_every_field_of_its_messages_statuses_and_sends() {
    let mut agents = Agents::new(
        [4_149_041_001, 4_149_041_002],
        ["ada-agent-reads-json", "bob-answers-in-json"],
    );
    agents.run_as(CLAUDE);
    let channel = agents.talk.channel.to_string();

    let (sent, envelopes) = send_as(&agents, (CLAUDE, &[JSON_FLAG]), ASKED);

    assert!(sent.success, "{sent:?}");
    let [asked] = envelopes.as_slice() else {
        panic!("not one message sent: {envelopes:?}");
    };
    let asked_id = asked.envelope().header.msg_id;
    let objects = json_objects(&sent.stdout);
    let said = object_of(&objects, asked_id, "channel");
    assert_fields(said, &[("channel", channel.clone())]);
    assert!(holds_text(said, STORED), "not {STORED}: {said}");

    let cloud = &agents.talk.world.cloud;
    agents.talk.played[0].take_in(cloud);
    assert_deleted(cloud, asked);
    let bob = agents.bob();
    let answered = agents.bob_sends(&Line::new(&bob, 1, ANSWERED));
    let (machine, folder) = (&agents.talk.ada.machine, &agents.folder);

    let got = (machine.start_in(folder, &[AGENT, CLAUDE, RECV, JSON_FLAG])).finish();

    assert_json_message(&agents, &got, (answered, ANSWERED));
    let objects = json_objects(&got.stdout);
    let status = object_of(&objects, asked_id, "status");
    let bob_device = agents.talk.played[0].device().to_string();
    let fields = [
        ("channel", channel),
        ("status", DELIVERED.to_owned()),
        ("device", bob_device),
    ];
    assert_fields(status, &fields);
}

/// Ada's device has run as claude in `~/work/acme`. claude's `skald recv --wait --json` waits;
/// Bob's device says something in the channel, and the wait gives it with every field and exits.
#[test]
fn json_of_a_waiting_agent_gives_every_field_of_the_message_that_comes() {
    let mut agents = Agents::new(
        [4_149_042_001, 4_149_042_002],
        ["ada-agent-waits-for-json", "bob-writes-to-a-waiting-agent"],
    );
    agents.run_as(CLAUDE);
    let args = [AGENT, CLAUDE, RECV, WAIT_FLAG, JSON_FLAG];
    let waiting = (agents.talk.ada.machine).start_in(&agents.folder, &args);
    let bob = agents.bob();

    let later = agents.bob_sends(&Line::new(&bob, 1, LATER));

    let got = waiting.finish();
    assert_json_message(&agents, &got, (later, LATER));
}
