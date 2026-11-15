//! Adding a member of the space to a channel: the revision has the member with `history` as the
//! command says; Ada's device, a member device of the channel, adds the new member's device, which
//! reads the channel's messages from then on; in a channel Ada's device is not in, it commits
//! nothing.

use skald_proto::Role;

use crate::edits::{Edit, ada_makes, assert_group_is};
use crate::fixtures::sent_after;
use crate::kit::{Change, assert_members, group, issue_channel, latest, received, revise};
use crate::played::Played;
use crate::talk::{CAROL_HOST, Talk, assert_read, commit_sent, one_message_sent};

/// What Ada says in the channel before Carol is in it, and after.
const BEFORE: &str = "the old staging password was hunter2";
const AFTER: &str = "welcome aboard, the runbook is pinned";

/// Ada's device signs the revision that adds Carol, from Carol's own machine, as a member of
/// `acme`, in its default channel; Ada's `skald recv` adds Carol's device there. Gives Carol's
/// device, played by the test, in the default channel's group.
fn carol_in_acme(talk: &Talk, id: u64, login: &str) -> Played {
    let cloud = &talk.world.cloud;
    let mut carol = Played::new(cloud, talk.world.sign_up(id, login, CAROL_HOST));
    let previous = latest(cloud, &talk.ada.token, talk.space.id);
    let change = Change {
        members: vec![(&carol.person, Role::Member)],
        ..Change::default()
    };
    revise(cloud, &previous, &talk.ada, &change);
    received(&talk.ada.machine);
    carol.take_in(cloud);
    let mut devices = vec![talk.ada.device, carol.device()];
    devices.extend(talk.played.iter().map(Played::device));
    assert_members(&carol.members(talk.space.default), &devices);
    carol
}

/// Carol is a member of `acme` but not of the talk's channel, in which Ada's machine says a line.
/// Ada's `skald add-to-channel` adds Carol there without the history: the revision has her with
/// `history` false, and Ada's device commits the Add of Carol's device. Carol's device joins the
/// channel's group from the Welcome, then reads the line Ada's machine says next, and not the one
/// before.
#[test]
fn member_added_to_a_channel_reads_its_messages_from_then_on() {
    let talk = Talk::new(
        &[4_140_103_001, 4_140_103_002],
        &["ada-adds-carol", "bob-sees-carol"],
        false,
    );
    let mut carol = carol_in_acme(&talk, 4_140_103_003, "carol-joins-late");
    let (space, channel, ada) = (talk.space.id, talk.channel, talk.ada.device);
    let said = talk.send(BEFORE);
    assert!(said.success, "{said:?}");

    let from = ada_makes(&talk, Edit::AddToChannel(channel, &carol.person, false));

    let cloud = &talk.world.cloud;
    assert!(
        commit_sent(&talk.front, channel, from),
        "Ada's device committed no Add"
    );
    carol.take_in(cloud);
    assert_eq!(
        carol.joined_to(channel).len(),
        1,
        "not one Welcome of the channel"
    );
    let devices = [ada, talk.played[0].device(), carol.device()];
    let carol_s = (&carol, talk.ada.token.as_str());
    assert_group_is(cloud, carol_s, (space, channel), &devices);
    let mark = talk.front.sent().len();
    let said = talk.send(AFTER);
    assert!(said.success, "{said:?}");
    let msg_id = one_message_sent(&talk.front, channel, mark);
    carol.take_in(cloud);
    let from_ada = talk.address(&talk.ada);
    assert_read(&carol, channel, (ada, &from_ada), &[(msg_id, 2, AFTER)]);
}

/// A channel of Bob's that Ada is not in, its group Bob's device alone. Ada's `skald
/// add-to-channel` adds Carol there with the history: the revision has her with `history` true,
/// and Ada's device, no member device of the channel, sends nothing in it; the channel's group
/// keeps Bob's device alone.
#[test]
fn admin_outside_a_channel_adds_a_member_to_it_and_sends_nothing_there() {
    let mut talk = Talk::new(
        &[4_140_103_101, 4_140_103_102],
        &["ada-adds-outside", "bob-owns-channel"],
        false,
    );
    let space = talk.space.id;
    let cloud = &talk.world.cloud;
    let bobs = issue_channel(cloud, &talk.ada.token, space);
    let previous = latest(cloud, &talk.ada.token, space);
    let change = Change {
        channels: vec![(bobs, vec![&talk.played[0].person])],
        ..Change::default()
    };
    revise(cloud, &previous, &talk.ada, &change);
    talk.played[0].creates_alone(cloud, (space, bobs));
    talk.played[0].uploads_group_info(cloud, (space, bobs));
    received(&talk.ada.machine);
    let carol = carol_in_acme(&talk, 4_140_103_103, "carol-joins-outside");

    let from = ada_makes(&talk, Edit::AddToChannel(bobs, &carol.person, true));

    let cloud = &talk.world.cloud;
    let in_bobs = sent_after(&talk.front, from)
        .iter()
        .filter(|sent| sent.envelope.envelope().header.channel == bobs)
        .count();
    assert_eq!(in_bobs, 0, "Ada's device sent in Bob's channel");
    let bob = talk.played[0].device();
    assert_members(
        &group(cloud, &talk.played[0].person.token, space, bobs).members,
        &[bob],
    );
}
