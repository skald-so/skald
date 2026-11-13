//! Checks 2 to 4 of "Roster" on the server, and on revision 1 checks 3 and 4, with the
//! certificates it holds: a revision that fails one is refused and not stored, since a stored
//! revision that every client rejects would block the space for good.

use skald_proto::{ChannelId, InviteId, Member, NonEmpty, Role, RosterBody};

use crate::api::{ACME, Team};
use crate::rosters::{channel, first, member, name, next, service_agent};

/// Changes `body`'s members with `edit`; at least one must remain.
fn edit_members(body: &mut RosterBody, edit: impl FnOnce(&mut Vec<Member>)) {
    let mut members = body.members.clone().into_vec();
    edit(&mut members);
    body.members = NonEmpty::from_vec(members).expect("a member");
}

/// A change that makes a revision fail checks 3 or 4, on revision 1 or a later one.
type Break = fn(&Team, &mut RosterBody);

/// Changes that break any revision, by what they break.
const ANY_REVISION: [(&str, Break); 8] = [
    ("a repeated account", |team, body| {
        let mut again = member(&team.alice, Role::Admin);
        again.handle = name("alice-again");
        body.members.push(again);
    }),
    ("a repeated handle", |team, body| {
        let mut dave = member(&team.dave, Role::Member);
        dave.handle = team.alice.handle.clone();
        body.members.push(dave);
    }),
    ("a repeated invite", |team, body| {
        let invite = InviteId::generate();
        edit_members(body, |members| members[0].invite = Some(invite));
        let mut dave = member(&team.dave, Role::Member);
        dave.invite = Some(invite);
        body.members.push(dave);
    }),
    ("a repeated excluded device", |team, body| {
        let device = team.alice.uncertified.id;
        edit_members(body, |members| {
            members[0].excluded = NonEmpty::from_vec(vec![device, device]);
        });
    }),
    ("a policy whose history is under a day", |_, body| {
        body.policy.history = 86_399;
    }),
    (
        "a service agent on a device without a certificate",
        |team, body| {
            body.service_agents
                .push(service_agent(&team.alice.uncertified));
        },
    ),
    ("a service agent on a client device", |team, body| {
        body.service_agents.push(service_agent(&team.alice.laptop));
    }),
    ("a service agent of an unknown device", |_, body| {
        body.service_agents
            .push(service_agent(&crate::common::devices::Device::new("CI")));
    }),
];

#[test]
fn a_revision_one_that_fails_checks_three_or_four_is_refused_and_not_stored() {
    let team = Team::new(3_120_820_001);
    for (what, change) in ANY_REVISION {
        let mut body = first(team.space, ACME, &team.alice);
        change(&team, &mut body);
        team.refused(&team.alice, &team.alice.sign(&body), &[], what);
    }
    // The checks refused those, not the revision 1 they started from.
    team.with_members();
}

#[test]
fn a_next_revision_that_fails_checks_three_or_four_is_refused_and_not_stored() {
    let team = Team::new(3_120_820_011);
    let stored = team.with_members().to_vec();
    let base = || next(&stored[1], &team.alice.laptop);
    for (what, change) in ANY_REVISION {
        let mut body = base();
        change(&team, &mut body);
        team.refused(&team.alice, &team.alice.sign(&body), &stored, what);
    }

    let mut no_admin = base();
    edit_members(&mut no_admin, |members| {
        for member in members {
            member.role = Role::Member;
        }
    });
    let id: ChannelId = team.alice.session.channel(team.space);
    let (alice, bob, dave) = (&team.alice, &team.bob, &team.dave);
    let channels = [
        (
            "a channel member that is no member",
            vec![channel(id, &[alice, dave])],
        ),
        (
            "a repeated channel",
            vec![channel(id, &[alice]), channel(id, &[bob])],
        ),
        (
            "a repeated channel member",
            vec![channel(id, &[alice, alice])],
        ),
    ];
    let mut cases = vec![("no admin", no_admin)];
    for (what, listed) in channels {
        let mut body = base();
        body.channels = listed;
        cases.push((what, body));
    }
    for (what, body) in cases {
        team.refused(alice, &team.alice.sign(&body), &stored, what);
    }

    // The checks refused those, not the revision they started from.
    team.stored_by(&team.alice, &team.alice.sign(&base()));
}
