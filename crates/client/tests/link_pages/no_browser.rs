//! Without a browser to open: the command says so and to run it at a terminal, exits failing, and
//! sends nothing, so nothing listens either. With a browser that does not open: the command says
//! that and to run it at a terminal, and exits failing.

use crate::at_terminal::as_agent;
use crate::linking::{LINK, says};
use crate::machine::{Machine, SERVER, SIGN_IN};
use crate::pages::{
    NO_BROWSER, NOT_OPENED, NOT_SAID_ONCE_RUN, PAGE, failing_program, missing_program, start_with,
};
use crate::world::World;

/// With `BROWSER` naming no program, as an agent, `skald link --browser` on Ada's first machine and
/// `skald sign-in --browser` on her new one each say on one line that there is no browser to open
/// and to run the command at a terminal, and exit failing; neither sends a request to the server,
/// so no device authorization is asked and no page listens, which comes after both.
#[test]
fn without_a_browser_to_open_the_command_says_so() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_179_501_001, "ada-has-no-browser", "ada-workstation");
    let (before, grants) = (cloud.relay.mark(), cloud.grants().len());

    let missing = missing_program(&ada.machine);
    let linking = start_with(&ada.machine, &missing, &as_agent(&[LINK, PAGE])).finish();
    let new = Machine::new().with_host_name("ada-laptop");
    let missing = missing_program(&new);
    let args = as_agent(&[SIGN_IN, SERVER, &cloud.url, PAGE]);
    let signing_in = start_with(&new, &missing, &args).finish();

    for (what, finished) in [("skald link", linking), ("skald sign-in", signing_in)] {
        assert!(!finished.success, "{what}: {finished:?}");
        assert!(says(&finished.shown(), &NO_BROWSER), "{what}: {finished:?}");
    }
    assert_eq!(cloud.grants().len(), grants, "a device authorization");
    let sent = cloud.relay.sent_since(before);
    assert!(sent.is_empty(), "a request without a browser: {sent:?}");
}

/// With `BROWSER` naming a program that exits failing, as an agent, `skald sign-in --browser` on
/// Ada's new machine, which then asks for no code, and `skald link --browser` on her first machine
/// each say on one line that the browser did not open and to run the command at a terminal, and
/// exit failing; neither says that `BROWSER` names no program or that nothing is sent.
#[test]
fn with_a_browser_that_does_not_open_the_command_says_so() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_179_501_002, "ada-browser-fails", "ada-workstation");

    let failing = failing_program(&ada.machine);
    let linking = start_with(&ada.machine, &failing, &as_agent(&[LINK, PAGE])).finish();
    let new = Machine::new().with_host_name("ada-laptop");
    let failing = failing_program(&new);
    let args = as_agent(&[SIGN_IN, SERVER, &cloud.url, PAGE]);
    let grants = cloud.grants().len();
    let signing_in = start_with(&new, &failing, &args).finish();
    assert_eq!(cloud.grants().len(), grants, "a code asked for");

    for (what, finished) in [("skald link", linking), ("skald sign-in", signing_in)] {
        let shown = finished.shown();
        assert!(!finished.success, "{what}: {finished:?}");
        assert!(says(&shown, &NOT_OPENED), "{what}: {finished:?}");
        for said in NOT_SAID_ONCE_RUN {
            assert!(!says(&shown, &[said]), "{what} says {said:?}: {finished:?}");
        }
    }
}
