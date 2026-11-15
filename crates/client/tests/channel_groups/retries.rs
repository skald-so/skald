//! A device's own Commits after a lost answer: with `stored` lost, or the CLI killed after sending
//! its Commit, the next run sends the same Commit again and ends in the group once.

use std::collections::BTreeSet;

use skald_crypto::{MlsContent, MlsFraming};
use skald_proto::{ChannelId, DeviceId};

use crate::creating::{ADA_HOST, BOB_HOST, Team, assert_one_welcome};
use crate::fixtures::{kill_once, sent_after};
use crate::front::{Answers, Front};
use crate::kit::{RECV, assert_listed_with, assert_members, channels, group, received};
use crate::world::{ACME, run};

/// The different Commits the front saw the machine send in `channel` after its first `from` sends.
fn commits_sent(front: &Front, channel: ChannelId, from: usize) -> BTreeSet<Vec<u8>> {
    sent_after(front, from)
        .into_iter()
        .map(|sent| sent.envelope.envelope().clone())
        .filter(|envelope| envelope.header.channel == channel)
        .filter(|envelope| {
            matches!(
                MlsFraming::read(&envelope.payload),
                Some(MlsFraming::Message {
                    content: MlsContent::Commit,
                    ..
                })
            )
        })
        .map(|envelope| envelope.payload)
        .collect()
}

/// How a run loses the answers to Ada's Commits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lost {
    /// The front loses each `stored`, and the run ends by itself.
    Stored,
    /// The front loses each `stored`, and the test kills the run once it sent a Commit.
    Killed,
}

/// Ada's run that adds Bob's device to the default channel and creates the channel's group with
/// it loses the answers to its Commits as `lost` says. The next run, with the answers forwarded,
/// sends each Commit again with the same bytes: Bob's device gets one Welcome of each group, and
/// each group holds both devices once.
fn ends_in_the_groups_once(lost: Lost, ids: [u64; 2], logins: [&str; 2]) {
    let mut team = Team::new(ids, logins, true);
    let (ada, bob): (DeviceId, DeviceId) = (team.ada.device, team.bob.device());
    let (default, channel) = (team.space.default, team.channel);
    let from = team.front.sent().len();
    team.front.answer(Answers::Lose);

    match lost {
        Lost::Stored => drop(run(&team.ada.machine, &[RECV])),
        Lost::Killed => {
            let started = team.ada.machine.start(&[RECV]);
            let front = &team.front;
            kill_once(started, "a Commit of Ada's device", || {
                !commits_sent(front, default, from).is_empty()
                    || !commits_sent(front, channel, from).is_empty()
            });
        }
    }
    assert!(
        team.front.lost() >= 1 || lost == Lost::Killed,
        "no stored was lost"
    );
    team.front.answer(Answers::Forward);
    received(&team.ada.machine);

    for of in [default, channel] {
        let sent = commits_sent(&team.front, of, from);
        assert_eq!(
            sent.len(),
            1,
            "{of}: not one Commit, sent again with the same bytes"
        );
    }
    let cloud = &team.world.cloud;
    group(cloud, &team.ada.token, team.space.id, default).assert_is(2, &[ada, bob]);
    group(cloud, &team.ada.token, team.space.id, channel).assert_is(1, &[ada, bob]);
    team.bob.take_in(cloud);
    assert_one_welcome(&team.bob.joined_to(default), default, 2, &[ada, bob]);
    assert_one_welcome(&team.bob.joined_to(channel), channel, 1, &[ada, bob]);
    assert_members(&team.bob.members(channel), &[ada, bob]);
    let listed = channels(&team.ada.machine, ACME);
    assert_listed_with(&listed, channel, &[ADA_HOST, BOB_HOST], &[]);
}

#[test]
fn commits_whose_stored_is_lost_end_in_the_groups_once() {
    ends_in_the_groups_once(
        Lost::Stored,
        [4_135_054_001, 4_135_054_002],
        ["ada-stored-lost", "bob-stored-lost"],
    );
}

#[test]
fn commits_of_a_cli_killed_after_sending_end_in_the_groups_once() {
    ends_in_the_groups_once(
        Lost::Killed,
        [4_135_054_101, 4_135_054_102],
        ["ada-commit-killed", "bob-commit-killed"],
    );
}
