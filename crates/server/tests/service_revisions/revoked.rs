//! A revision that keeps a revoked `service` device is refused; one that drops it is taken.

use crate::agents::Acme;
use crate::common::devices::assert_taken;
use crate::listed::{add, assert_refused_revision, drop_device};

/// alice's service agent is in `acme`, in `general`'s `services` too. After alice's laptop revokes
/// its device, the next revision that keeps it, unchanged or with only its entry dropped, is
/// refused, and one that drops it from both is taken.
#[test]
fn a_revision_that_keeps_a_revoked_service_device_is_refused() {
    let mut acme = Acme::with(3_358_100_001, &[]);
    let agent = acme.new_agent();
    add(&mut acme, &agent);
    let alice = &acme.f.team.alice;
    alice.revoke(&alice.by_device(&alice.laptop, agent.device));

    assert_refused_revision(&acme, |_| {}, "a revision that keeps the revoked device");
    assert_refused_revision(
        &acme,
        |body| {
            body.service_agents
                .retain(|entry| entry.device != agent.device)
        },
        "a revision that keeps the revoked device in services",
    );
    let response = acme.propose(None, |body| drop_device(body, agent.device));
    assert_taken(&response);
}
