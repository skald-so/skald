//! As an agent, the CLI's output names nothing of a member's deletion in another space; the
//! person's next run tells it.

use skald_proto::Role;

use crate::fixtures::fronted;
use crate::kit::{
    Change, assert_names_none, bind_client_a, create_space, latest, received, received_as_agent,
    revise,
};
use crate::scene::{assert_told_once, delete_account};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::{CLIENT_A, CLIENT_B};

/// The agent the tests run as.
const CLAUDE: &str = "claude-d3l7ac";

/// Bob's machine created `clientb`, whose next revision, signed by his device, adds Ada as a
/// member, and `clienta`, to which `~/work/clientA` is bound; Ada is not in `clienta`. Ada deletes
/// her account. As an agent in `~/work/clientA`, `skald recv` names nothing of `clientb` or Ada;
/// the person's next `skald recv` tells that Ada is deleted, and the one after it does not.
#[test]
fn agent_names_no_deletion_of_another_space_s_member_which_waits_for_the_person() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_305_106_001, "bob-desk-deletions", BOB_HOST);
    let ada = world.sign_up(4_305_106_002, "ada-client-deleted", ADA_HOST);
    let space = create_space(cloud, &bob, CLIENT_B);
    let change = Change {
        members: vec![(&ada, Role::Member)],
        ..Change::default()
    };
    revise(cloud, &latest(cloud, &bob.token, space.id), &bob, &change);
    create_space(cloud, &bob, CLIENT_A);
    let folder = bind_client_a(&bob.machine);
    received(&bob.machine);
    let handle = ada.handle.to_string();
    let names = [
        CLIENT_B.to_owned(),
        space.id.to_string(),
        space.default.to_string(),
        handle.clone(),
        ada.device.to_string(),
        ADA_HOST.to_owned(),
    ];
    delete_account(cloud, &ada);

    let agent = received_as_agent(&bob.machine, &folder, CLAUDE);

    assert_names_none(&agent, &names);
    let person = received(&bob.machine);
    let later = received(&bob.machine);
    assert_told_once(&[&person, &later], &handle);
}
