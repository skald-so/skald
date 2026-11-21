//! A code's attempts and its end (`membership.md`, "Link code"): the new device runs a code at most
//! 3 times, then says it failed 3 times and that someone else may have tried; it shows a fresh code
//! only when its person asks; the linked device stops after 3 failed runs in a row, also in its
//! next runs, until its person confirms the new device shows a new code. A code ends 5 minutes after
//! it was shown, here sooner, and a run of it then fails.

use std::thread;
use std::time::{Duration, Instant};

use crate::linking::{
    EXPIRED, HELLO, LIFETIME_SETTING, LINK, NEW_CODE, NewDevice, OWN_DEVICE, RELAYED, THREE_TIMES,
    YES, codes_in, link_failing, other_code, says, typed, until_not_waiting, waiting,
};
use crate::machine::Machine;
use crate::terminal::Terminal;
use crate::world::World;

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// How long a test watches for what must not happen.
const QUIET: Duration = Duration::from_secs(3);

/// A code counts only as a whole token: the digits of a device ID shaped like one do not.
#[test]
fn codes_are_whole_tokens_not_parts_of_an_id() {
    let text = "Linked: issued by the device 84bc7bab-7813-89aa-b475-365c9d616798.\n\
                Type this code there: VZK-MP5 (or ab1-CDE2, XYZ-ABC-DEF).";
    assert_eq!(codes_in(text), ["VZK-MP5"]);
}

/// Whether a request to the link code relay went through the relay since `since`.
fn relayed_since(world: &World, since: Instant) -> bool {
    let path = RELAYED.to_lowercase();
    (world.cloud.relay.sent_since(since).iter()).any(|sent| sent.head.contains(&path))
}

/// Three runs of Ada's linked device with a wrong code each fail: the new device then says its
/// code failed 3 times and that someone else may have tried, and shows no fresh code and sends no
/// hello until its person asks. The linked device's next run asks first whether the new device
/// shows a new code; answered no, it sends nothing. Asked, the new device shows a fresh code, and
/// the linked device's next run, answered yes, links with it.
#[test]
fn three_failed_runs_end_the_code_and_a_fresh_one_comes_only_when_asked() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_401_001, "ada-tries-three-times", "ada-workstation");
    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    let code = new.code();

    for failed in 1..=3 {
        new.open_run(cloud, &ada);
        link_failing(&ada, &other_code(&code));
        new.wait_for_failures(failed);
    }
    new.run().wait_for("that the code failed 3 times", |shown| {
        says(shown, &THREE_TIMES)
    });
    thread::sleep(QUIET);
    let shown = new.shown();
    let mut distinct = codes_in(&shown);
    distinct.dedup();
    assert_eq!(
        distinct,
        std::slice::from_ref(&code),
        "a fresh code unasked: {shown}"
    );
    assert_eq!(cloud.count_sent(new.started, HELLO), 3, "a hello unasked");
    assert!(!waiting(cloud, &ada, new.device), "a run still open");

    let before = cloud.relay.mark();
    let mut stopped = Terminal::start(&ada.machine, &[LINK]);
    stopped.wait_to_show(NEW_CODE);
    stopped.type_line("no");
    let stopped = stopped.finish();
    assert!(!stopped.success, "{}", stopped.text);
    assert!(
        !relayed_since(&world, before),
        "the stopped device sent a request"
    );

    let fresh = new.fresh_code();
    new.open_run(cloud, &ada);
    let mut resumed = Terminal::start(&ada.machine, &[LINK]);
    resumed.wait_to_show(NEW_CODE);
    resumed.type_line(YES);
    resumed.wait_to_show(OWN_DEVICE);
    resumed.type_line(&typed(&fresh));
    resumed.wait_for("the new device's name", |shown| shown.contains(LAPTOP));
    resumed.type_line(YES);
    let resumed = resumed.finish();
    assert!(resumed.success, "{}", resumed.text);
    assert!(new.finish().success, "the new device failed");
}

/// The new device's code lives 30 seconds here, long enough for a run of its fresh code under
/// load: it then says the code expired and ends its run, and the linked device's run with that code
/// fails. Asked, the new device shows a fresh code, with which it links.
#[test]
fn a_code_expires_and_a_fresh_one_links() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_401_011, "ada-waits-too-long", "ada-workstation");
    let machine = Machine::new()
        .with_host_name(LAPTOP)
        .with_env(LIFETIME_SETTING, "30");
    let mut new = NewDevice::start_on(&world, &ada, machine, LAPTOP);
    let code = new.code();

    new.run()
        .wait_for("that the code expired", |shown| says(shown, &[EXPIRED]));
    until_not_waiting(cloud, &ada, new.device);
    link_failing(&ada, &typed(&code));

    let fresh = new.fresh_code();
    // Its first run ended with the code; this is its second.
    new.runs = 1;
    new.open_run(cloud, &ada);
    let linked = crate::linking::link_with(&ada, &typed(&fresh), LAPTOP, YES);
    assert!(linked.success, "{}", linked.text);
    assert!(new.finish().success, "the new device failed");
}
