//! A revision that adds service agents beyond the setting is refused, and one that reaches it
//! taken; unset, no revision is refused for its agents.

use serde_json::json;

use crate::agents::{Acme, LIMIT, LIMIT_ENV};

/// With the limit 2: a revision that adds the space's first service agent is taken, and so is one
/// that adds its second, reaching the limit. One that adds a third is refused with the problem,
/// also when sent again, and stores nothing; the space still has its two agents.
#[test]
fn a_revision_that_reaches_the_limit_is_taken_and_one_that_adds_an_agent_beyond_it_refused() {
    let mut acme = Acme::with(3_359_100_001, &[LIMIT, "2"]);

    acme.add_agents(None, 1);
    acme.add_agents(None, 1);
    acme.assert_refused_adding(None, 1, "a third agent over the limit of 2");
    assert_eq!(acme.agents(), 2);
}

/// With the limit 3 and one agent: a revision that adds three at once, four in all, is refused,
/// and one that adds two at once, reaching the limit, is taken.
#[test]
fn a_revision_that_adds_several_agents_beyond_the_limit_at_once_is_refused() {
    let mut acme = Acme::with(3_359_100_011, &[LIMIT, "3"]);
    acme.add_agents(None, 1);

    acme.assert_refused_adding(None, 3, "three more agents, four over the limit of 3");
    acme.add_agents(None, 2);
    assert_eq!(acme.agents(), 3);
}

/// The setting comes from its environment variable too: on an instance with
/// `SKALD_SERVICE_AGENTS_LIMIT=1`, a space with one agent cannot add another.
#[test]
fn the_limit_comes_from_its_environment_variable_too() {
    let mut acme = Acme::with(3_359_100_021, &[]);
    acme.add_agents(None, 1);
    let limited = acme.f.team.flow.stand.instance(&[], &[(LIMIT_ENV, "1")]);

    acme.assert_refused_adding(Some(&limited), 1, "a second agent over the limit of 1");
}

/// Without the setting, as on a self-hosted server, a space takes 12 service agents, more than
/// the cloud's 10, one revision at a time and then three at once, and its usage gives them with no
/// limit.
#[test]
fn without_the_setting_a_space_holds_any_number_of_service_agents_with_no_limit_in_its_usage() {
    let mut acme = Acme::with(3_359_100_031, &[]);

    for _ in 0..9 {
        acme.add_agents(None, 1);
    }
    acme.add_agents(None, 3);
    assert_eq!(acme.agents(), 12);
    let alice = acme.f.team.alice.session.clone();
    assert_eq!(acme.usage_of_agents(&alice), json!({ "count": 12 }));
}
