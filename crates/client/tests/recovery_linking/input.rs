//! Where the recovery key is typed: only by the person at a terminal. Through a pipe, or run as an
//! agent, the run refuses and signs nothing.

use std::thread;
use std::time::{Duration, Instant};

use crate::at_terminal::as_agent;
use crate::browser::APPROVE;
use crate::cloud::Cloud;
use crate::fixtures::strs;
use crate::machine::{Machine, RUN_LIMIT};
use crate::world::World;
use crate::{
    ADA_HOST, LOST, NEW_HOST, assert_links_later, assert_signed_nothing, play, recovery_args,
    sign_up, start_recovery,
};

/// With its input and outputs pipes, the run that links Ada's new machine with the recovery key,
/// naming her first device lost, is given the key on its input. Her code, if it shows one, she
/// approves. It exits failing, and no machine signed anything; at a terminal, a later run on that
/// machine links it.
#[test]
fn through_a_pipe_linking_with_the_recovery_key_refuses() {
    let world = World::start();
    let cloud: &Cloud = &world.cloud;
    let ada = sign_up(&world, (4_176_300_001, "ada-recovery-pipe"), ADA_HOST);
    let first = ada.person.device;
    let machine = Machine::new().with_host_name(NEW_HOST);
    let since = cloud.relay.mark();
    let grants = cloud.grants().len();

    let args = recovery_args(cloud, &[], &[(LOST, first)]);
    let mut run = machine.start_asking(&strs(&args));
    run.answer(&format!("{}\n", ada.key));
    let (started, mut approved) = (Instant::now(), false);
    while !run.exited() {
        assert!(
            started.elapsed() < RUN_LIMIT,
            "skald did not finish: {}",
            run.shown()
        );
        if !approved && cloud.grants().len() > grants {
            let grant = cloud.last_grant();
            run.wait_to_show(&[&grant.user_code]);
            cloud.decide(&world.browser.page(), ada.user(), &grant, APPROVE);
            approved = true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let shown = run.finish();

    assert!(!shown.success, "{shown:?}");
    assert_signed_nothing(cloud, &ada.person.token, (since, &[first]), &shown.shown());
    assert_links_later(&world, &ada, &machine);
}

/// Run as an agent at a terminal, the run that links Ada's new machine with the recovery key,
/// naming her first device lost, is typed the key if it asks for it. It exits failing, and no
/// machine signed anything; run by the person, a later run on that machine links it.
#[test]
fn as_an_agent_linking_with_the_recovery_key_refuses() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = sign_up(&world, (4_176_300_101, "ada-recovery-agent"), ADA_HOST);
    let first = ada.person.device;
    let machine = Machine::new().with_host_name(NEW_HOST);
    let since = cloud.relay.mark();

    let (mut run, grants) = start_recovery(&world, &machine, &as_agent(&[]), &[(LOST, first)]);
    play(
        &world,
        ada.user(),
        &mut run,
        (grants, Some(&ada.key)),
        || false,
    );
    let shown = run.finish();

    assert!(!shown.success, "{shown:?}");
    assert_signed_nothing(cloud, &ada.person.token, (since, &[first]), &shown.text);
    assert_links_later(&world, &ada, &machine);
}
