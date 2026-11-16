//! `skald set-history-period`: an admin's device signs the next revision with the policy's
//! `history`, and rotation follows the new period; a period under a day is refused, and so is the
//! command on the device of a member that is no admin, each sending nothing; with the answer to the
//! revision lost, the next run leaves one revision with the change.

use std::thread;
use std::time::{Duration, Instant};

use skald_crypto::accept_next;
use skald_proto::{ChannelMember, Mode, Role, Roster, SpaceId};

use crate::aged::{
    DAY, HOUR, ahead, assert_bob_entered_by_empty_commits, assert_no_commit,
    assert_one_commit_each, epochs,
};
use crate::cloud::Cloud;
use crate::edits::{
    SPACE_CHANGE, assert_refused, assert_refused_as_no_admin, last_two, store_request,
};
use crate::fixtures::fronted;
use crate::front::Front;
use crate::kit::{channels, create_space, latest, received, rosters, stale_once};
use crate::machine::{Finished, Machine};
use crate::relay::Lose;
use crate::talk::{ADA_HOST, BOB_HOST, Talk};
use crate::world::{
    ACME, MEMBER, Person, assert_role, members, next_by, next_with, run, set_role, space_certs,
    store,
};

/// The command that sets the period.
pub const SET_HISTORY_PERIOD: &str = "set-history-period";

/// What the command says when it refuses a period under a day.
pub const UNDER_A_DAY: &str = "day";

/// Runs `skald set-history-period <space> <period>` on `machine`.
fn set_period(machine: &Machine, space: &str, period: &str) -> Finished {
    run(machine, &[SET_HISTORY_PERIOD, space, period])
}

/// Fails unless `next`, a revision the server holds, is the one after `previous` that `signer`'s
/// device signed with the policy's `history` `seconds` and no other change, and the signer accepts
/// it.
#[track_caller]
fn assert_period(
    cloud: &Cloud,
    (previous, next): (&Roster, &Roster),
    signer: &Person,
    seconds: u64,
) {
    let expected = next_with(previous, (signer.device, &signer.key), |body| {
        body.policy.history = seconds;
        body.timestamp = next.body().timestamp;
    });
    assert_eq!(
        next.body(),
        expected.body(),
        "not the next revision, signed by the signer's device, with the period alone"
    );
    let space = expected.body().space;
    let certs = space_certs(cloud, &signer.token, space);
    let accepted = accept_next(previous, next, &certs, signer.account, &signer.master);
    assert_eq!(accepted, Ok(()), "the signer does not accept it");
}

/// Fails unless, since `since` and the front's first `sent` sends, the run `ran` sent no request
/// that changes a space and no envelope, and `space` still has the revisions `held`, for the
/// session `token`.
#[track_caller]
fn assert_nothing_sent(
    (cloud, front): (&Cloud, &Front),
    (since, sent): (Instant, usize),
    (space, token, held): (SpaceId, &str, &[Roster]),
    ran: &Finished,
) {
    let now = (
        cloud.count_sent(since, SPACE_CHANGE),
        front.sent().len(),
        rosters(cloud, token, space),
    );
    assert_eq!(now, (0, sent, held.to_vec()), "it sent something: {ran:?}");
}

/// Two days after the epochs began, with the default period of 30 days, Ada's `skald recv` sends no
/// Commit. Ada's `skald set-history-period acme 3d` then has the server hold the next revision,
/// signed by her device, with a period of 3 days and no other change. Two days on her runs still
/// send no Commit; four days on, `skald recv` commits one empty Commit in each channel, which Bob's
/// device enters.
#[test]
fn an_admin_sets_the_period_and_rotation_follows_it() {
    let mut talk = Talk::new(
        &[4_162_206_001, 4_162_206_002],
        &["ada-sets-period", "bob-follows-period"],
        false,
    );
    let (before, from) = (epochs(&talk), talk.front.sent().len());
    talk.ada = ahead(talk.ada, 2 * DAY);
    received(&talk.ada.machine);
    assert_no_commit(&talk, before, from);
    let (space, token) = (talk.space.id, talk.ada.token.clone());
    let held = rosters(talk.cloud(), &token, space).len();

    let set = set_period(&talk.ada.machine, ACME, "3d");

    assert!(set.success, "{set:?}");
    let (previous, next) = last_two(talk.cloud(), &token, space, held);
    assert_period(talk.cloud(), (&previous, &next), &talk.ada, 3 * DAY);
    received(&talk.ada.machine);
    assert_no_commit(&talk, before, from);
    talk.ada = ahead(talk.ada, 4 * DAY);
    received(&talk.ada.machine);
    assert_one_commit_each(&talk, before, from);
    assert_bob_entered_by_empty_commits(&mut talk, before);
}

/// On Ada's device, an admin's, `skald set-history-period acme 23h` fails, saying the period is
/// under a day, and sends nothing; `36h` then has the server hold the next revision with a period
/// of 36 hours.
#[test]
fn a_period_under_a_day_is_refused_and_one_in_hours_from_a_day_on_is_set() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_162_207_001, "ada-short-period", ADA_HOST);
    let space = create_space(cloud, &ada, ACME).id;
    let held = rosters(cloud, &ada.token, space);
    let marks = (cloud.relay.mark(), front.sent().len());

    let refused = set_period(&ada.machine, ACME, "23h");

    assert_refused(&refused, UNDER_A_DAY);
    assert_nothing_sent((cloud, &front), marks, (space, &ada.token, &held), &refused);
    let set = set_period(&ada.machine, ACME, "36h");
    assert!(set.success, "{set:?}");
    let (previous, next) = last_two(cloud, &ada.token, space, held.len());
    assert_period(cloud, (&previous, &next), &ada, 36 * HOUR);
}

/// Bob's machine creates `acme`, and Bob's device signs the revision that adds Ada as an admin and
/// makes Bob a member, which Bob's device accepts. There `skald set-history-period acme 2d` fails,
/// saying Bob is no admin, and sends nothing.
#[test]
fn the_device_of_a_member_that_is_no_admin_refuses_and_sends_nothing() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_162_208_001, "bob-steps-down", BOB_HOST);
    let ada = world.sign_up(4_162_208_002, "ada-takes-over", ADA_HOST);
    let space = create_space(cloud, &bob, ACME).id;
    let handed_over = next_by(&latest(cloud, &bob.token, space), &bob, |list| {
        list.push(ada.member(Role::Admin));
        set_role(list, bob.account, Role::Member);
    });
    store(cloud, &bob.token, &handed_over);
    received(&bob.machine);
    assert_role(&members(&bob.machine, ACME), &bob.handle, MEMBER);
    let held = rosters(cloud, &bob.token, space);
    let marks = (cloud.relay.mark(), front.sent().len());

    let refused = set_period(&bob.machine, ACME, "2d");

    assert_refused_as_no_admin(&refused);
    assert_nothing_sent((cloud, &front), marks, (space, &bob.token, &held), &refused);
}

/// Bob, an admin too, stores a revision that changes the policy's ceilings while Ada's device sees
/// the revisions as they were before it. Ada's `skald set-history-period acme 2d` signs a revision
/// that the server refuses, accepts Bob's and signs the period on top of it: the server holds Bob's
/// revision, then Ada's with the period alone.
#[test]
fn a_period_set_after_another_admins_revision_came_first_is_kept() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_162_210_001, "ada-period-on-top", ADA_HOST);
    let bob = world.sign_up(4_162_210_002, "bob-period-first", BOB_HOST);
    let space = create_space(cloud, &ada, ACME).id;
    let with_bob = next_with(
        &latest(cloud, &ada.token, space),
        (ada.device, &ada.key),
        |body| {
            body.members.push(bob.member(Role::Admin));
            body.channels[0].members.push(ChannelMember {
                account: bob.account,
                history: true,
            });
        },
    );
    store(cloud, &ada.token, &with_bob);
    received(&ada.machine);
    let held = rosters(cloud, &ada.token, space).len();
    let bobs = next_with(
        &latest(cloud, &bob.token, space),
        (bob.device, &bob.key),
        |body| {
            body.policy.ceilings.member = Mode::Full;
        },
    );
    store(cloud, &bob.token, &bobs);
    stale_once(cloud, space, held);

    let set = set_period(&ada.machine, ACME, "2d");

    assert!(set.success, "{set:?}");
    let all = rosters(cloud, &ada.token, space);
    assert_eq!(
        all.len(),
        held + 2,
        "not Bob's revision and one more: {all:?}"
    );
    assert_eq!(all[held], bobs, "Bob's revision is not kept");
    assert_period(cloud, (&all[held], &all[held + 1]), &ada, 2 * DAY);
}

/// The relay loses the server's answer to the revision of Ada's `skald set-history-period acme 2d`
/// as `how` says: it closes the connection, or it withholds the answer and the test kills the CLI.
/// The next run, `skald channels acme`, leaves the space with one more revision, the period alone,
/// signed by Ada's device.
fn one_revision_after_its_answer_is_lost(how: Lose, id: u64, login: &str) {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(id, login, ADA_HOST);
    let space = create_space(cloud, &ada, ACME).id;
    let held = rosters(cloud, &ada.token, space).len();
    let request = store_request(space);
    cloud.relay.lose_answer(&request, how);

    let started = ada.machine.start(&[SET_HISTORY_PERIOD, ACME, "2d"]);
    match how {
        // It may try again itself and finish.
        Lose::Close => drop(started.finish()),
        Lose::Withhold => cloud.kill_once_lost(started, &request),
    }
    // Signed again a second later, a revision has another timestamp, and so other bytes.
    thread::sleep(Duration::from_millis(1100));
    assert_eq!(cloud.relay.lost(), 1, "{request} lost no answer");
    channels(&ada.machine, ACME);

    let (previous, next) = last_two(cloud, &ada.token, space, held);
    assert_period(cloud, (&previous, &next), &ada, 2 * DAY);
}

#[test]
fn an_answer_dropped_by_the_proxy_leaves_one_revision_with_the_period() {
    one_revision_after_its_answer_is_lost(Lose::Close, 4_162_209_001, "ada-period-dropped");
}

#[test]
fn an_answer_lost_with_the_cli_killed_leaves_one_revision_with_the_period() {
    one_revision_after_its_answer_is_lost(Lose::Withhold, 4_162_209_101, "ada-period-killed");
}
