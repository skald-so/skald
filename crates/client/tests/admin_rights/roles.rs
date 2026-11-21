//! What the tests share: the role commands, the revision each makes, a space of the CLI's machine
//! with members, refusals that send nothing, and the lines that tell of a role change.

use std::time::Instant;

use skald_crypto::accept_next;
use skald_proto::{AccountId, DeviceId, Name, NonEmpty, Role, Roster, RosterBody, SpaceId};

use crate::cloud::Cloud;
use crate::edits::{SPACE_CHANGE, last_two};
use crate::fixtures::strs;
use crate::front::Front;
use crate::kit::{Change, create_space, issue_channel, latest, received, revise, rosters};
use crate::machine::{Finished, Machine};
use crate::world::{
    ADMIN, Person, next_by, next_with, run, set_role, space_certs, store, words,
};

/// The role commands.
pub const MAKE_ADMIN: &str = "make-admin";
pub const HAND_OVER_ADMIN: &str = "hand-over-admin";

/// What the only-admin warning says beside the command it names.
pub const RECOVERY_KEY: &str = "recovery key";

/// A role command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleCommand {
    /// Makes the member an admin.
    MakeAdmin,
    /// Makes the member an admin and the device's account a member.
    HandOver,
}

impl RoleCommand {
    /// The command's name.
    pub fn name(self) -> &'static str {
        match self {
            RoleCommand::MakeAdmin => MAKE_ADMIN,
            RoleCommand::HandOver => HAND_OVER_ADMIN,
        }
    }

    /// Its arguments on the space named `space`, about the member of `handle`.
    pub fn args(self, space: &str, handle: &Name) -> Vec<String> {
        [self.name(), space, handle.as_str()]
            .map(str::to_owned)
            .to_vec()
    }

    /// The roles a revision of it sets, run on a device of `runner`'s account about `member`.
    pub fn roles(self, runner: AccountId, member: AccountId) -> Vec<(AccountId, Role)> {
        match self {
            RoleCommand::MakeAdmin => vec![(member, Role::Admin)],
            RoleCommand::HandOver => vec![(member, Role::Admin), (runner, Role::Member)],
        }
    }
}

/// Runs `command` on `machine` on the space named `space`, about `member`.
pub fn role(machine: &Machine, command: RoleCommand, space: &str, member: &Person) -> Finished {
    run(machine, &strs(&command.args(space, &member.handle)))
}

/// As [`role`]; fails unless it succeeds.
#[track_caller]
pub fn role_set(machine: &Machine, command: RoleCommand, space: &str, member: &Person) -> Finished {
    let ran = role(machine, command, space, member);
    assert!(
        ran.success,
        "{} {space} {}: {ran:?}",
        command.name(),
        member.handle
    );
    ran
}

/// `body` with its members in account order.
fn in_order(body: &RosterBody) -> RosterBody {
    let mut body = body.clone();
    let mut members = body.members.clone().into_vec();
    members.sort_by_key(|member| member.account);
    body.members = NonEmpty::from_vec(members).expect("a member");
    body
}

/// Fails unless `next`, a revision the server holds, is the one after `previous` that the device
/// `signer` signed with `roles` set and nothing else changed, and each of `accepting`'s devices, a
/// member's, accepts it with the certificates the server gives its session.
#[track_caller]
pub fn assert_roles(
    cloud: &Cloud,
    (previous, next): (&Roster, &Roster),
    signer: DeviceId,
    roles: &[(AccountId, Role)],
    accepting: &[&Person],
) {
    let mut expected = previous.body().clone();
    expected.revision += 1;
    expected.prev = Some(previous.hash());
    expected.signer = signer;
    expected.timestamp = next.body().timestamp;
    let mut members = expected.members.clone().into_vec();
    for (account, role) in roles {
        set_role(&mut members, *account, *role);
    }
    expected.members = NonEmpty::from_vec(members).expect("a member");
    assert_eq!(
        in_order(next.body()),
        in_order(&expected),
        "not the next revision, signed by {signer}, with the roles {roles:?} alone"
    );
    let space = expected.space;
    for person in accepting {
        let certs = space_certs(cloud, &person.token, space);
        let accepted = accept_next(previous, next, &certs, person.account, &person.master);
        assert_eq!(
            accepted,
            Ok(()),
            "{}'s device does not accept it",
            person.handle
        );
    }
}

/// Runs `command` about `member` on `runner`'s machine on the space `(space, name)`; fails unless it
/// succeeds and the server then holds one more revision, the one after the last with the command's
/// roles alone, signed by `runner`'s device, which `runner`'s and each of `others`' devices accept.
/// Gives what the run showed.
#[track_caller]
pub fn role_changed(
    cloud: &Cloud,
    (runner, command, member): (&Person, RoleCommand, &Person),
    (space, name): (SpaceId, &str),
    others: &[&Person],
) -> Finished {
    let held = rosters(cloud, &runner.token, space).len();
    let ran = role_set(&runner.machine, command, name, member);
    let (previous, next) = last_two(cloud, &runner.token, space, held);
    let mut accepting = vec![runner];
    accepting.extend_from_slice(others);
    let roles = command.roles(runner.account, member.account);
    assert_roles(cloud, (&previous, &next), runner.device, &roles, &accepting);
    ran
}

/// The lines of `runs` that tell of `member`'s new role `role`, `admin` or `member`, in a revision
/// that a device of `by`'s account signed: each holds the three as words.
pub fn told(runs: &[&Finished], (member, role, by): (&Name, &str, &Name)) -> Vec<String> {
    (runs.iter())
        .flat_map(|run| run.shown().lines().map(str::to_owned).collect::<Vec<_>>())
        .filter(|line| {
            let held = words(line);
            held.contains(&member.as_str()) && held.contains(&role) && held.contains(&by.as_str())
        })
        .collect()
}

/// Fails unless `runs` tell once that `member` is an admin by a revision of `by`'s.
#[track_caller]
pub fn assert_told_admin_once(runs: &[&Finished], member: &Name, by: &Name) {
    let lines = told(runs, (member, ADMIN, by));
    assert_eq!(
        lines.len(),
        1,
        "not told once that {member} is an admin by {by}'s revision: {lines:?} in {runs:?}"
    );
}

/// `signer`'s device signs the revision after the latest of `space` with `account`'s role `role`,
/// stored as `signer`'s session. Gives it.
pub fn role_by(
    cloud: &Cloud,
    space: SpaceId,
    signer: &Person,
    (account, role): (AccountId, Role),
) -> Roster {
    let previous = latest(cloud, &signer.token, space);
    let next = next_by(&previous, signer, |members| {
        set_role(members, account, role)
    });
    store(cloud, &signer.token, &next);
    next
}

/// `signer`'s device signs the revision after the latest of `space` with `change` made to its body,
/// stored as `signer`'s session. Gives it.
pub fn signed_by(
    cloud: &Cloud,
    space: SpaceId,
    signer: &Person,
    change: impl FnOnce(&mut RosterBody),
) -> Roster {
    let previous = latest(cloud, &signer.token, space);
    let next = next_with(&previous, (signer.device, &signer.key), change);
    store(cloud, &signer.token, &next);
    next
}

/// The space `name` that `creator`'s machine creates, with `members` added with their roles in its
/// default channel and in a channel with the creator and them, which revision 3, signed by the
/// creator's device, adds, and a channel with the creator alone, which revision 4 adds. The
/// creator's machine accepts both. Gives the space and the two channels' IDs.
pub fn space_with(
    cloud: &Cloud,
    creator: &Person,
    name: &str,
    members: &[(&Person, Role)],
) -> (SpaceId, [skald_proto::ChannelId; 2]) {
    let space = create_space(cloud, creator, name).id;
    let shared = issue_channel(cloud, &creator.token, space);
    let mut in_shared = vec![creator];
    in_shared.extend(members.iter().map(|(person, _)| *person));
    let change = Change {
        members: members.to_vec(),
        channels: vec![(shared, in_shared)],
        ..Change::default()
    };
    revise(
        cloud,
        &latest(cloud, &creator.token, space),
        creator,
        &change,
    );
    let alone = issue_channel(cloud, &creator.token, space);
    let change = Change {
        channels: vec![(alone, vec![creator])],
        ..Change::default()
    };
    revise(
        cloud,
        &latest(cloud, &creator.token, space),
        creator,
        &change,
    );
    received(&creator.machine);
    (space, [shared, alone])
}

/// Fails unless `refused` failed saying `admin`, beside the role commands' names, and, since
/// `since` and the front's first `from` sends, no run sent a request that changes a space or an
/// envelope, and `space` still has the revisions `held`, for the session `token`.
#[track_caller]
pub fn assert_refused_sending_nothing(
    (cloud, front): (&Cloud, &Front),
    (since, from): (Instant, usize),
    (space, token, held): (SpaceId, &str, &[Roster]),
    refused: &Finished,
) {
    assert!(!refused.success, "{refused:?}");
    // Not the role commands' names: an unknown command names them too.
    let said = (refused.shown().to_lowercase())
        .replace(MAKE_ADMIN, "")
        .replace(HAND_OVER_ADMIN, "");
    assert!(said.contains(ADMIN), "it does not say admin: {refused:?}");
    assert_eq!(
        cloud.count_sent(since, SPACE_CHANGE),
        0,
        "a request that changes a space: {refused:?}"
    );
    assert_eq!(front.sent().len(), from, "an envelope sent: {refused:?}");
    assert_eq!(rosters(cloud, token, space), held, "the space's revisions");
}
