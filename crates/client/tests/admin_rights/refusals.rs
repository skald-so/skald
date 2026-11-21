//! The role commands are the person's: run as an agent, each refuses and sends nothing.

use skald_proto::Role;

use crate::edits::SPACE_CHANGE;
use crate::fixtures::{fronted, strs};
use crate::kit::rosters;
use crate::roles::{RoleCommand, space_with};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::{ACME, ADD_RULE, AGENT, dir_of, path, run};

/// The agent the test runs the commands as.
const AGENT_NAME: &str = "claude-r0l3s";

/// Ada's machine created `acme`, with Bob a member, and binds `~/work/acme` to it. Run as an agent
/// there, `skald make-admin acme <bob>` and `skald hand-over-admin acme <bob>` each fail, saying
/// they are the person's commands, which an agent cannot run, send no request that changes a space
/// and leave `acme` with its revisions.
#[test]
fn as_an_agent_each_role_command_refuses_and_sends_nothing() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_301_105_001, "ada-runs-an-agent", ADA_HOST);
    let bob = world.sign_up(4_301_105_002, "bob-not-promoted", BOB_HOST);
    let (space, _) = space_with(cloud, &ada, ACME, &[(&bob, Role::Member)]);
    let folder = dir_of(&ada.machine, "work/acme");
    let bound = run(&ada.machine, &[ADD_RULE, path(&folder), ACME]);
    assert!(bound.success, "{bound:?}");
    let held = rosters(cloud, &ada.token, space);

    for command in [RoleCommand::MakeAdmin, RoleCommand::HandOver] {
        let since = cloud.relay.mark();
        let mut args = vec![AGENT.to_owned(), AGENT_NAME.to_owned()];
        args.extend(command.args(ACME, &bob.handle));

        let refused = ada.machine.start_in(&folder, &strs(&args)).finish();

        assert!(!refused.success, "{refused:?}");
        let said = refused.stderr.to_lowercase();
        assert!(
            said.contains(&format!("skald {}", command.name()))
                && said.contains("person's command")
                && said.contains("an agent cannot run"),
            "{command:?}: {refused:?}"
        );
        assert_eq!(
            cloud.count_sent(since, SPACE_CHANGE),
            0,
            "a request that changes a space: {refused:?}"
        );
        assert_eq!(
            rosters(cloud, &ada.token, space),
            held,
            "the space's revisions"
        );
    }
}
