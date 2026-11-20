//! With every device of Ada's account wiped, her recovery key on a new machine shows the whole
//! history of her channels before the machine is in them; it then joins them, and its agents and a
//! device it links later exchange messages there again.

use skald_proto::{ChannelId, DeviceId, Role, SpaceId};

use crate::aged::{DAY, received_ahead, with_period};
use crate::chain::removes_from_channel;
use crate::devices::{BOB, CAROL, said_by, second_machine};
use crate::fixtures::SEND;
use crate::joins::read_from;
use crate::kit::{RECV, assert_members, group, received};
use crate::reading::{Said, assert_history, says, sends, shown_history};
use crate::talk::{Line, Talk};
use crate::world::{ACME, ADD_RULE, AGENT, HOST, Person, dir_of, path, run};
use crate::{LAPTOP_HOST, Personal, SECOND_HOST, recovers, talk_with_key};

/// The agents of Ada's new machine and of Bob's device.
const ADA_AGENT: &str = "claude-r7k2vd";
const BOB_AGENT: &str = "codex-q4m8zn";

/// What Ada's new machine's agent and Bob's agent say to each other in `acme`'s channel.
const TO_BOB: &str = "ada's agent is back after the recovery";
const TO_ADA: &str = "bob's agent welcomes ada's agent back";

/// What Ada's laptop and her new machine say to each other in her personal space.
const TO_NEW: &str = "the laptop checks in with the recovered machine";
const TO_LAPTOP: &str = "the recovered machine answers the laptop";

/// The messages said in `acme`'s channel and in Ada's personal space's default channel before
/// Ada's devices are wiped, oldest first.
struct Before {
    acme: Vec<Said>,
    personal: Vec<Said>,
}

/// In the talk of Ada, Bob and Carol, Ada's second machine is linked by hand and joins her
/// channels by itself. In `acme`'s channel Carol, Bob and both of Ada's machines each say a line. A
/// revision Ada's first device signs sets the history period to a day, and her first machine's run
/// with its clock two days ahead rotates the key; Bob, Carol and her first machine each say one. A
/// revision her first device signs takes Carol out of the channel, and its run removes her device;
/// Bob and her second machine each say one. In her personal space's default channel each of her
/// machines says a line, and her first one another one at the end. Both machines run last.
fn before_the_wipe(talk: Talk, second: &Person, personal: &Personal) -> (Talk, Before) {
    let mut talk = talk;
    let (at, space) = (talk.at(), talk.space.name.clone());
    let ours = (personal.name.as_str(), personal.channel);
    let mut acme = vec![
        said_by(&mut talk, CAROL, (at, 1), "carol opens the recovery drill").0,
        said_by(&mut talk, BOB, (at, 1), "bob lists the drill steps").0,
        sends(&talk.ada, (&space, at.1), "ada confirms the drill steps"),
        sends(second, (&space, at.1), "ada adds a note from her tablet"),
    ];
    let mut personal_said = vec![
        sends(&talk.ada, ours, "ada notes where the backups live"),
        sends(second, ours, "ada notes the tablet's serial number"),
    ];
    with_period(&talk, DAY);
    let mut talk = received_ahead(talk, 2 * DAY);
    acme.push(
        said_by(
            &mut talk,
            BOB,
            (at, 2),
            "bob reruns the drill after the rotation",
        )
        .0,
    );
    acme.push(said_by(&mut talk, CAROL, (at, 2), "carol signs the drill report").0);
    acme.push(sends(
        &talk.ada,
        (&space, at.1),
        "ada files the drill report",
    ));
    removes_from_channel(talk.cloud(), &talk.ada, at, &talk.played[CAROL].person);
    received(&talk.ada.machine);
    acme.push(
        said_by(
            &mut talk,
            BOB,
            (at, 3),
            "bob closes the drill without carol",
        )
        .0,
    );
    acme.push(sends(
        second,
        (&space, at.1),
        "ada thanks bob from her tablet",
    ));
    personal_said.push(sends(&talk.ada, ours, "ada notes the drill is over"));
    received_by(&talk.ada);
    received_by(second);
    (
        talk,
        Before {
            acme,
            personal: personal_said,
        },
    )
}

/// Runs `skald recv` on `person`'s machine; fails unless it succeeds.
#[track_caller]
fn received_by(person: &Person) {
    received(&person.machine);
}

/// Fails unless the server's group of `at`'s channel, as the session `token` gets it, holds each
/// of `devices` once and no other.
#[track_caller]
fn assert_group(talk: &Talk, (token, at): (&str, (SpaceId, ChannelId)), devices: &[DeviceId]) {
    let held = group(talk.cloud(), token, at.0, at.1);
    assert_members(&held.members, devices);
}

/// Ada's first machine, which made her account at a terminal, and her second, linked by hand, are
/// in `acme`'s channel with Bob's device and in her personal space's default channel, where the
/// messages of [`before_the_wipe`] are said. Then both are wiped. Her new machine, linked with the
/// recovery key naming both devices lost, shows with `skald history` every message of both
/// channels once, oldest first, while the server's groups of both channels do not hold its device.
/// Its runs then join both, and the groups hold its device and Bob's, and its device alone. Its
/// agent in `~/work/acme`, bound to `acme`, says a line in the channel, which Bob's device reads
/// from it; Bob's agent's answer shows in that agent's next `skald recv`. A laptop her new machine
/// links by hand, once the new machine has run since its join, shows every message of the personal
/// space's channel, and the laptop and the new machine each show the line the other says there.
#[test]
fn recovery_key_on_a_new_machine_shows_the_whole_history_and_gets_back_into_the_channels() {
    let (talk, key) = talk_with_key(
        &[4_177_701_001, 4_177_701_002, 4_177_701_003],
        &[
            "ada-wipes-every-device",
            "bob-stays-in-touch",
            "carol-leaves-early",
        ],
        Role::Member,
    );
    let at = talk.at();
    let second = second_machine(&talk.world, &talk.ada, SECOND_HOST);
    received_by(&second);
    received_by(&talk.ada);
    let personal = Personal::of(talk.cloud(), &talk.ada);
    let (first, tablet) = (talk.ada.device, second.device);
    assert_group(&talk, (&talk.ada.token, personal.at()), &[first, tablet]);
    let (mut talk, said) = before_the_wipe(talk, &second, &personal);
    let bob = talk.played[BOB].device();
    assert_group(&talk, (&talk.ada.token, at), &[first, tablet, bob]);

    let new = recovers(&talk, &key, &[first, tablet]);
    let acme_history = shown_history(&new.machine, at.1);
    let personal_history = shown_history(&new.machine, personal.channel);

    assert_history(&acme_history, at.1, &said.acme, &[]);
    assert_history(&personal_history, personal.channel, &said.personal, &[]);
    for of in [at, personal.at()] {
        let held = group(talk.cloud(), &new.token, of.0, of.1);
        assert!(
            !held.members.contains(&new.device),
            "the new device was in {} before it showed its history: {held:?}",
            of.1
        );
    }

    received_by(&new);
    received_by(&new);

    assert_group(&talk, (&new.token, at), &[new.device, bob]);
    assert_group(&talk, (&new.token, personal.at()), &[new.device]);
    let folder = dir_of(&new.machine, "work/acme");
    let bound = run(&new.machine, &[ADD_RULE, path(&folder), ACME]);
    assert!(bound.success, "not bound: {bound:?}");
    let channel = at.1.to_string();
    let to_bob = (new.machine)
        .start_in(
            &folder,
            &[
                AGENT,
                ADA_AGENT,
                SEND,
                "--all",
                "--channel",
                &channel,
                TO_BOB,
            ],
        )
        .finish();
    assert!(to_bob.success, "{to_bob:?}");
    talk.played[BOB].take_in(&talk.world.cloud);
    assert_eq!(
        read_from(&talk.played[BOB], at.1, TO_BOB),
        [new.device],
        "Bob's device did not read the agent's line from the new device"
    );
    let from_bob_agent = format!(
        "{ACME}/{}/{BOB_AGENT}@{HOST}",
        talk.played[BOB].person.handle
    );
    let cloud = &talk.world.cloud;
    says(
        cloud,
        &mut talk.played[BOB],
        at,
        &Line::new(&from_bob_agent, 1, TO_ADA),
    );
    let agent = (new.machine)
        .start_in(&folder, &[AGENT, ADA_AGENT, RECV])
        .finish();
    assert!(agent.success, "{agent:?}");
    assert!(agent.shown().contains(TO_ADA), "{agent:?}");

    let laptop = second_machine(&talk.world, &new, LAPTOP_HOST);
    received_by(&laptop);
    received_by(&new);
    let laptop_history = shown_history(&laptop.machine, personal.channel);

    assert_history(&laptop_history, personal.channel, &said.personal, &[]);
    let ours = (personal.name.as_str(), personal.channel);
    sends(&laptop, ours, TO_NEW);
    let at_new = received(&new.machine);
    assert!(at_new.shown().contains(TO_NEW), "{at_new:?}");
    sends(&new, ours, TO_LAPTOP);
    let at_laptop = received(&laptop.machine);
    assert!(at_laptop.shown().contains(TO_LAPTOP), "{at_laptop:?}");
}
