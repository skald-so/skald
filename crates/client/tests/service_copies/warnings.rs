//! The sending device warns its user when it starts sending a channel's copies to a service agent
//! it sent none of them before, naming the agent and the channel: once per agent and channel.

use skald_proto::ChannelId;

use crate::agents::{BOT, create_agent, secrets_of};
use crate::machine::Finished;
use crate::scene::{OTHER_BOT, Scene};
use crate::world::{ACME, warnings};

/// The warning lines of `finished` that name the agent `agent` and `channel`'s ID.
fn copy_warnings(finished: &Finished, agent: &str, channel: ChannelId) -> Vec<String> {
    let channel = channel.to_string().to_lowercase();
    warnings(finished)
        .into_iter()
        .filter(|line| line.contains(agent) && line.contains(&channel))
        .collect()
}

/// Fails unless `finished` warns of `agent` in `channel`; `what` names the run.
#[track_caller]
fn assert_warns(finished: &Finished, agent: &str, channel: ChannelId, what: &str) {
    let warned = copy_warnings(finished, agent, channel);
    assert!(
        !warned.is_empty(),
        "{what}: no warning names {agent} and {channel}: {finished:?}"
    );
}

/// Fails if a warning of `finished` names `agent`; `what` names the run.
#[track_caller]
fn assert_no_warning_of(finished: &Finished, agent: &str, what: &str) {
    let warned: Vec<String> = warnings(finished)
        .into_iter()
        .filter(|line| line.contains(agent))
        .collect();
    assert!(warned.is_empty(), "{what}: warns of {agent}: {warned:?}");
}

/// `ci/deploy-bot` has a grant for the talk's channel and the default one. Ada's first message in
/// the talk's channel warns, naming the agent and that channel; her second there does not; her
/// first in the default channel warns again, naming it. Once `ci/review-bot` gets a grant for the
/// talk's channel, her next message there warns of it, and not of `ci/deploy-bot`.
#[test]
fn the_sender_warns_once_when_it_starts_sending_a_channel_s_copies_to_an_agent() {
    let scene = Scene::with(
        [3_155_520_001, 3_155_520_002],
        ["ada-warned-standin", "bob-warned-standin"],
        skald_proto::Role::Member,
        |talk| vec![talk.channel, talk.space.default],
        &[],
    );
    let (channel, default) = (scene.talk.channel, scene.talk.space.default);

    let first = scene.send_all(channel, "the first copy of the channel");
    assert_warns(&first, BOT, channel, "the first message in the channel");

    let second = scene.send_all(channel, "the second copy of the channel");
    assert_no_warning_of(&second, BOT, "the second message in the channel");

    let other_channel = scene.send_all(default, "the first copy of the default channel");
    assert_warns(
        &other_channel,
        BOT,
        default,
        "the first message in the default channel",
    );

    let created = create_agent(&scene.talk.ada.machine, ACME, OTHER_BOT, &[channel], &[]);
    secrets_of(&created);
    let other_agent = scene.send_all(channel, "the first copy to another agent");
    assert_warns(
        &other_agent,
        OTHER_BOT,
        channel,
        "the first message to another agent",
    );
    assert_no_warning_of(&other_agent, BOT, "the first message to another agent");
}
