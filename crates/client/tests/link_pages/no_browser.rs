//! Without a browser to open: the command says so and to run it at a terminal, exits failing, and
//! takes no step of the link code.

use crate::at_terminal::as_agent;
use crate::linking::{LINK, RELAYED, says};
use crate::machine::{Finished, Run, SERVER, SIGN_IN};
use crate::pages::{NO_BROWSER, PAGE, approve_next_as, missing_program, start_with};
use crate::world::World;

/// Waits for `run` to exit, approving as `user` any device authorization it gets on the way.
fn finish_approving(world: &World, user: &skald_github_stand_in::User, mut run: Run) -> Finished {
    let before = world.cloud.grants().len();
    let deadline = std::time::Instant::now() + crate::machine::RUN_LIMIT;
    while !run.exited() {
        if world.cloud.grants().len() > before {
            approve_next_as(world, user, before, &mut run);
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{}", run.shown());
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    run.finish()
}

/// With `BROWSER` naming no program, as an agent, `skald link --browser` on Ada's first machine and
/// `skald sign-in --browser` on her new one each say on one line that there is no browser to open
/// and to run the command at a terminal, and exit failing; neither sends a request to the link code
/// relay.
#[test]
fn without_a_browser_to_open_the_command_says_so() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_179_501_001, "ada-has-no-browser", "ada-workstation");
    let before = cloud.relay.mark();

    let missing = missing_program(&ada.machine);
    let linking = start_with(&ada.machine, &missing, &as_agent(&[LINK, PAGE])).finish();
    let new = crate::machine::Machine::new().with_host_name("ada-laptop");
    let missing = missing_program(&new);
    let args = as_agent(&[SIGN_IN, SERVER, &cloud.url, PAGE]);
    let signing_in = finish_approving(&world, &ada.user, start_with(&new, &missing, &args));

    for (what, finished) in [("skald link", linking), ("skald sign-in", signing_in)] {
        assert!(!finished.success, "{what}: {finished:?}");
        assert!(says(&finished.shown(), &NO_BROWSER), "{what}: {finished:?}");
    }
    let path = RELAYED.to_lowercase();
    assert!(
        !(cloud.relay.sent_since(before).iter()).any(|sent| sent.head.contains(&path)),
        "a request to the link code relay without a page"
    );
}
