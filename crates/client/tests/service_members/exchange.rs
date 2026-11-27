//! A member service agent gets into its channel's group and exchanges messages there as a device
//! does: with gap detection on both sides, its grant and a counter in each of its messages, and a
//! history entry of each.

use std::num::NonZeroU64;

use skald_proto::{DeviceId, MessageId, RawEnvelope};

use crate::fixtures::send;
use crate::gaps::{assert_reported_once, bob_in, msg_id_of, second_dropped};
use crate::joins::drop_entry;
use crate::kit::{RECV, assert_listed_with, channels, latest, line_of, received, warnings_naming};
use crate::member::{Bot, JSON, assert_not_shown, shown_once, texts_of};
use crate::service_kit::{BOT, entry_of};
use crate::world::{ACME, run};

/// What Ada, Bob and the agent say; none is anywhere else.
const ADA_SAYS: &str = "member bot test: ada asks for the staging deploy";
const BOB_SAYS: [&str; 3] = [
    "member bot test: bob queues the first migration",
    "member bot test: bob's second line, which the server drops",
    "member bot test: bob queues the third migration",
];
const AGENT_SAYS: [&str; 3] = [
    "member bot test: deploy-bot starts the rollout",
    "member bot test: deploy-bot's second line",
    "member bot test: deploy-bot finished the rollout",
];

/// The command that shows a channel's history.
const HISTORY: &str = "history";

/// The level of a `service` device's messages.
const SERVICE: &str = "service";

/// Ada's machine creates `ci/deploy-bot` as a member of the talk's channel: the revision it signs
/// lists the agent's device in that channel's `services` and in no other channel's. Runs of the
/// agent and of Ada's machine, with Bob's device taking in its mailbox, bring it into the channel's
/// group, added by Ada's device or by itself: Bob's device holds Ada's, Bob's and the agent's
/// devices there, also after Ada's next run; no run of Ada's warns of the agent's device; and Ada's
/// channel list shows `ci/deploy-bot` in the channel's group.
#[test]
fn a_member_agent_is_listed_in_its_channel_and_gets_into_the_group() {
    let mut bot = Bot::created(
        [3_135_710_001, 3_135_710_002],
        ["ada-member-bot-standin", "bob-member-bot-standin"],
    );
    let revision = latest(bot.talk.cloud(), &bot.talk.ada.token, bot.talk.space.id);
    assert_eq!(entry_of(&revision, BOT), Some(bot.device), "{revision:?}");
    for channel in &revision.body().channels {
        let wanted: &[DeviceId] = if channel.id == bot.channel() {
            &[bot.device]
        } else {
            &[]
        };
        assert_eq!(channel.services, wanted, "the services of {}", channel.id);
    }

    let mut ada_runs = bot.brought_in();

    ada_runs.push(received(&bot.talk.ada.machine));
    bot.bob_takes_in();
    let mut members = bot.talk.played[0].members(bot.channel());
    members.sort();
    let mut wanted = vec![bot.talk.ada.device, bot.talk.played[0].device(), bot.device];
    wanted.sort();
    assert_eq!(members, wanted, "the channel's group at Bob's device");
    for ran in &ada_runs {
        let warned = warnings_naming(ran, bot.device);
        assert!(warned.is_empty(), "Ada's run warns of the agent: {ran:?}");
    }
    let listed = channels(&bot.talk.ada.machine, ACME);
    assert!(listed.success, "{listed:?}");
    assert_listed_with(&listed, bot.channel(), &[], &[]);
    // The agent's name holds a `/`, which splits it into two words of `assert_listed_with`.
    let line = line_of(&listed, bot.channel());
    let names: Vec<&str> = line.split([' ', ',']).collect();
    assert!(
        names.contains(&BOT),
        "{} without {BOT}: {listed:?}",
        bot.channel()
    );
}

/// With the agent in the group, its device on Ada's account, an admin's, Ada says a line: the
/// agent's run shows it once and writes nothing to stderr, nor does the run after it.
#[test]
fn a_run_of_an_admin_s_member_agent_with_nothing_to_warn_of_writes_nothing_to_stderr() {
    let mut bot = Bot::created(
        [3_135_713_001, 3_135_713_002],
        ["ada-member-quiet-standin", "bob-member-quiet-standin"],
    );
    bot.brought_in();
    let (ada, channel) = (&bot.talk.ada, bot.channel());
    let sent = send(&ada.machine, channel, ADA_SAYS);
    assert!(sent.success, "Ada's send failed: {sent:?}");

    let got = bot.runner.received();

    shown_once(
        &got,
        ADA_SAYS,
        (channel, &bot.talk.address(ada), ada.device),
    );
    assert_eq!(got.stderr, "", "{got:?}");
    let again = bot.runner.received();
    assert_not_shown(&again, ADA_SAYS);
    assert_eq!(again.stderr, "", "{again:?}");
}

/// With the agent in the group, Ada's message reaches the agent's run once, from Ada's address and
/// device. Bob's device says three lines with the counters 1 to 3, the second never sent: the
/// agent's run shows the first and the third, the third after missing ones, and warns once of a
/// message missing in the channel from Bob's device. The agent says three lines, and the server
/// drops the second from Ada's mailbox: Ada's run shows the first and the third once each, from the
/// agent's address and device, named `ci/deploy-bot`, at the `service` level, the third after
/// missing ones, and warns once of a message missing in the channel from the agent's device.
#[test]
fn a_member_agent_exchanges_messages_with_the_members_with_gap_detection_on_both_sides() {
    let mut bot = Bot::created(
        [3_135_711_001, 3_135_711_002],
        ["ada-member-talk-standin", "bob-member-talk-standin"],
    );
    bot.brought_in();
    let channel = bot.channel();
    bot.ada_reaches_the_agent(ADA_SAYS);

    bot.bob_takes_in();
    let (bob_from, _, bob_device) = bob_in(&bot.talk);
    second_dropped(&mut bot.talk, BOB_SAYS);
    received(&bot.talk.ada.machine);

    let got = bot.runner.received();

    let bob_said = (channel, bob_from.as_str(), bob_device);
    let first = shown_once(&got, BOB_SAYS[0], bob_said);
    let third = shown_once(&got, BOB_SAYS[2], bob_said);
    assert_not_shown(&got, BOB_SAYS[1]);
    assert_eq!(first["after_missing"], false, "{first}");
    assert_eq!(third["after_missing"], true, "{third}");
    assert_reported_once(&got.shown(), channel, bob_device);

    let said: Vec<RawEnvelope> = AGENT_SAYS.iter().map(|text| bot.says(text)).collect();
    drop_entry(bot.talk.cloud(), bot.talk.ada.device, &said[1]);

    let got = run(&bot.talk.ada.machine, &[RECV, JSON]);

    assert!(got.success, "{got:?}");
    let agent_said = (channel, bot.address.as_str(), bot.device);
    let first = shown_once(&got, AGENT_SAYS[0], agent_said);
    let third = shown_once(&got, AGENT_SAYS[2], agent_said);
    for message in [&first, &third] {
        assert_eq!(message["device_name"], BOT, "{message}");
        assert_eq!(message["level"], SERVICE, "{message}");
    }
    assert_not_shown(&got, AGENT_SAYS[1]);
    assert_eq!(first["after_missing"], false, "{first}");
    assert_eq!(third["after_missing"], true, "{third}");
    assert_reported_once(&got.shown(), channel, bot.device);
}

/// The agent says three lines. Bob's device reads them in order from the agent's device, each with
/// `from` the agent's address, the counters 1 to 3, the text, and a grant that passes Bob's
/// device's checks of "Service grant". Ada's run shows each once, at the `service` level. Ada's
/// history of the channel shows each once, on a line with the channel's ID, the agent's address
/// and `ci/deploy-bot`.
#[test]
fn a_member_agent_s_messages_carry_its_grant_and_counter_and_leave_history_entries() {
    let mut bot = Bot::created(
        [3_135_712_001, 3_135_712_002],
        ["ada-member-history-standin", "bob-member-history-standin"],
    );
    bot.brought_in();
    let channel = bot.channel();

    let said: Vec<MessageId> = AGENT_SAYS
        .iter()
        .map(|text| msg_id_of(&bot.says(text)))
        .collect();

    bot.bob_takes_in();
    let read = bot.bob_read_from_the_agent();
    let ids: Vec<MessageId> = read.iter().map(|(msg_id, _)| *msg_id).collect();
    assert_eq!(ids, said, "the agent's messages Bob's device read");
    for ((counter, text), (msg_id, content)) in (1..).zip(AGENT_SAYS).zip(&read) {
        assert_eq!(
            content.from.to_string(),
            bot.address,
            "{msg_id}: {content:?}"
        );
        assert_eq!(
            content.counter.map(NonZeroU64::get),
            Some(counter),
            "{msg_id}: {content:?}"
        );
        assert_eq!(texts_of(content), [text], "{msg_id}");
        bot.assert_granted(content);
    }

    let got = run(&bot.talk.ada.machine, &[RECV, JSON]);
    assert!(got.success, "{got:?}");
    for text in AGENT_SAYS {
        let message = shown_once(&got, text, (channel, &bot.address, bot.device));
        assert_eq!(message["level"], SERVICE, "{message}");
    }

    let history = run(&bot.talk.ada.machine, &[HISTORY, &channel.to_string()]);

    assert!(history.success, "{history:?}");
    for text in AGENT_SAYS {
        assert_eq!(
            history.shown().matches(text).count(),
            1,
            "{text:?} not shown once: {history:?}"
        );
        let line = (history.stdout.lines())
            .find(|line| line.contains(text))
            .unwrap_or_else(|| panic!("{text:?} not on a line of its own: {history:?}"));
        for part in [channel.to_string().as_str(), &bot.address, BOT] {
            assert!(line.contains(part), "{text:?} shown without {part}: {line}");
        }
    }
}
