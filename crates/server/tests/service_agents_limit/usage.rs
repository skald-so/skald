//! The space's usage gives the service agents of its latest revision, with the limit; without the
//! setting: `limit.rs`.

use serde_json::json;

use crate::agents::{Acme, LIMIT};
use crate::common::devices::assert_taken;

/// With the limit 3, `acme`'s usage gives its admin alice `0 of 3` before any agent, `2 of 3` after
/// a revision adds two, and `1 of 3` after the next removes one; carol, a member who is no admin,
/// gets the same.
#[test]
fn usage_counts_the_service_agents_of_the_latest_revision_with_their_limit() {
    let mut acme = Acme::with(3_359_300_001, &[LIMIT, "3"]);
    let team = &acme.f.team;
    let (alice, carol) = (team.alice.session.clone(), team.carol.session.clone());
    assert_eq!(
        acme.usage_of_agents(&alice),
        json!({ "count": 0, "limit": 3 })
    );

    acme.add_agents(None, 2);
    assert_eq!(
        acme.usage_of_agents(&alice),
        json!({ "count": 2, "limit": 3 })
    );

    let response = acme.propose(None, |body| {
        body.service_agents.pop();
    });
    assert_taken(&response);
    assert_eq!(acme.agents(), 1);
    for session in [&alice, &carol] {
        assert_eq!(
            acme.usage_of_agents(session),
            json!({ "count": 1, "limit": 3 })
        );
    }
}
