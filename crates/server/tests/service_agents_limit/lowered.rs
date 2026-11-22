//! A space over a lowered limit still removes service agents and changes the rest; it adds none.

use skald_proto::Role;

use crate::agents::{Acme, LIMIT};
use crate::common::devices::assert_taken;
use crate::rosters::member;

/// `acme` takes three service agents on a server without the setting; on an instance of it on the
/// same database with the limit 1, below them, the server takes, in turn: a revision that adds
/// eve as a member, keeping the three agents; one that puts a new device in place of an agent's,
/// keeping their number; one that removes an agent, two still over the limit. A revision that
/// then adds an agent back, three again, is refused with the problem.
#[test]
fn over_a_lowered_limit_a_space_still_removes_and_changes_but_adds_no_agents() {
    let mut acme = Acme::with(3_359_200_001, &[]);
    acme.add_agents(None, 3);
    let lowered = acme.f.team.flow.stand.instance(&[LIMIT, "1"], &[]);
    let eve = member(&acme.f.eve, Role::Member);

    let response = acme.propose(Some(&lowered), |body| body.members.push(eve));
    assert_taken(&response);

    let replacement = acme.new_agent().device;
    let response = acme.propose(Some(&lowered), |body| {
        body.service_agents[0].device = replacement;
    });
    assert_taken(&response);
    assert_eq!(acme.agents(), 3);

    let response = acme.propose(Some(&lowered), |body| {
        body.service_agents.pop();
    });
    assert_taken(&response);
    assert_eq!(acme.agents(), 2);

    acme.assert_refused_adding(Some(&lowered), 1, "an agent added over the lowered limit");
    assert_eq!(acme.agents(), 2);
}
