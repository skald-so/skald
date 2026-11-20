//! A revision that removes an account but lists its `service` device is refused.

use crate::agents::{Acme, agent_of};
use crate::common::devices::assert_taken;
use crate::listed::{add, assert_refused_revision, drop_account, drop_device};

/// carol's service agent is in `acme`, in `general`'s `services` too. A revision that removes
/// carol but keeps her agent, in its entry or only in `services`, is refused; one that removes both
/// is taken.
#[test]
fn a_revision_that_removes_an_account_but_keeps_its_service_device_is_refused() {
    let mut acme = Acme::with(3_358_100_101, &[]);
    let agent = agent_of(&acme.f.team.carol, acme.f.db(), 101);
    add(&mut acme, &agent);
    let carol = acme.f.team.carol.account;

    assert_refused_revision(
        &acme,
        |body| drop_account(body, carol),
        "a revision that keeps a removed account's service device",
    );
    assert_refused_revision(
        &acme,
        |body| {
            drop_account(body, carol);
            body.service_agents
                .retain(|entry| entry.device != agent.device);
        },
        "a revision that keeps a removed account's service device in services",
    );
    let response = acme.propose(None, |body| {
        drop_account(body, carol);
        drop_device(body, agent.device);
    });
    assert_taken(&response);
}
