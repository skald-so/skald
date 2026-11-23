//! As an agent, the CLI's output names nothing of a member's deletion in another space; the
//! person's next run tells it.

use std::thread;

use skald_proto::Role;

use crate::fixtures::{await_running, fronted};
use crate::helped::{Helped, WEBSOCKET};
use crate::kit::{
    Change, RECV, assert_names_none, bind_client_a, create_space, latest, received,
    received_as_agent, revise,
};
use crate::scene::{PUSHED, assert_told_once, delete_account};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::{AGENT, CLIENT_A, CLIENT_B};

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

/// A waiting agent gives its helper a chance to take in Ada's deletion before the person runs
/// any command. The helper keeps the report for the person.
#[test]
fn agent_only_helper_keeps_a_member_deletion_for_the_person() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let bob = world.sign_up(4_305_106_101, "bob-agent-helps", BOB_HOST);
    let ada = world.sign_up(4_305_106_102, "ada-leaves-agent-space", ADA_HOST);
    let space = create_space(cloud, &bob, CLIENT_A);
    let change = Change {
        members: vec![(&ada, Role::Member)],
        ..Change::default()
    };
    revise(cloud, &latest(cloud, &bob.token, space.id), &bob, &change);
    let folder = bind_client_a(&bob.machine);
    received(&bob.machine);
    let helped = Helped::new(&bob.machine);
    let opened = cloud.relay.mark();
    let mut waiting = helped.start_in(&folder, &[], &[AGENT, CLAUDE, RECV, "--wait"]);
    await_running(&mut waiting, "the helper's connection", || {
        cloud.count_sent(opened, WEBSOCKET) > 0
    });
    thread::sleep(PUSHED);

    let before = cloud.relay.mark();
    delete_account(cloud, &ada);

    let devices = format!("GET /spaces/{}/devices ", space.id);
    await_running(&mut waiting, "the helper's device list", || {
        cloud.count_sent(before, &devices) > 0
    });
    let person = helped.received();
    let later = helped.received();
    assert_told_once(&[&person, &later], &ada.handle.to_string());
}
