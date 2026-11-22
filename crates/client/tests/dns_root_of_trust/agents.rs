//! A run as an agent with another resolver, which gives other records, changes nothing the
//! person's later runs with Bob's resolver show.

use std::process::Stdio;

use crate::doh::Doh;
use crate::machine::{Finished, Run};
use crate::world::{ACME, ADD_RULE, AGENT, MEMBERS, dir_of, path, run};
use crate::{CLAUDE, LONG_TTL, RECORDS, RESOLVER, Scene, listed, listed_elsewhere};

/// Bob's folder bound to `acme`, then `skald --agent claude members acme` there with the resolver
/// `other`; fails unless the run succeeds and looked the records up at `other`.
fn agent_run_with(scene: &Scene, other: &Doh) -> Finished {
    let machine = &scene.bob.machine;
    let folder = dir_of(machine, "work/acme");
    let bound = run(machine, &[ADD_RULE, path(&folder), ACME]);
    assert!(bound.success, "not bound: {bound:?}");
    let mut command = machine.command_in(&folder, &[AGENT, CLAUDE, MEMBERS, ACME]);
    command.stdin(Stdio::null()).env(RESOLVER, other.url());
    let ran = Run::spawn(&mut command).finish();
    assert!(ran.success, "{ran:?}");
    assert!(
        other.txt_queries(RECORDS) > 0,
        "the agent's run looked nothing up at its resolver: {:?}",
        other.queries()
    );
    ran
}

/// Bob's resolver lists Ada with another fingerprint; the agent's lists Bob and Ada with theirs.
/// After the agent's run, Bob's next `skald members acme` warns once naming Ada and a later one
/// does not; neither shows her listed.
#[test]
fn an_agent_run_whose_resolver_lists_the_member_changes_nothing_later_runs_show() {
    let scene = Scene::new(
        [4_187_501_001, 4_187_501_002],
        ["bob-whose-agent-asks-another", "ada-listed-for-the-agent"],
    );
    let (bob, ada) = (&scene.bob, &scene.ada);
    scene.name_domain(|_| {});
    scene.records(LONG_TTL, &[listed(bob), listed_elsewhere(ada)]);
    let other = Doh::start();
    other.set_txt(RECORDS, LONG_TTL, &[listed(bob), listed(ada)]);

    agent_run_with(&scene, &other);

    scene.ada_warned_once();
}

/// Bob's resolver lists Bob and Ada with their fingerprints; the agent's lists Ada with another and
/// not Bob. After the agent's run, Bob's next two runs of `skald members acme` show both listed
/// and warn of neither.
#[test]
fn an_agent_run_whose_resolver_lists_another_fingerprint_changes_nothing_later_runs_show() {
    let scene = Scene::named(
        [4_187_502_001, 4_187_502_002],
        ["bob-whose-agent-is-misled", "ada-misplaced-for-the-agent"],
        |bob, ada| vec![listed(bob), listed(ada)],
    );
    let other = Doh::start();
    other.set_txt(RECORDS, LONG_TTL, &[listed_elsewhere(&scene.ada)]);

    agent_run_with(&scene, &other);

    for _ in 0..2 {
        scene.assert_both_listed(&scene.members());
    }
}
