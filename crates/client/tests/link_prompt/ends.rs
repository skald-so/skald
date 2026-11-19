//! The line stops once the device no longer waits: it linked, its code ended, or the server refused
//! its certificate as over the devices limit.

use skald_server::Settings;

use crate::devices::second_machine;
use crate::fixtures::{fronted, fronted_with};
use crate::kit::RECV;
use crate::linking::{EXPIRED, LIFETIME_SETTING, NewDevice, YES, link_with, says, typed};
use crate::machine::{DEVICES, Machine};
use crate::prompt::{
    LAPTOP, assert_not_prompted, assert_prompted, until_not_waiting, waiting_device,
};
use crate::world::{Person, World, run};

/// The host name of Ada's machine linked by hand.
const DESKTOP: &str = "ada-desktop";

/// The host name of Ada's first machine.
const WORKSTATION: &str = "ada-workstation";

/// Runs `args` on `machine`; fails unless it succeeds. Gives what it showed.
#[track_caller]
fn shown_by(machine: &Machine, args: &[&str]) -> String {
    let finished = run(machine, args);
    assert!(finished.success, "{args:?}: {finished:?}");
    finished.shown()
}

/// Fails unless Ada's first machine's `skald devices` names her waiting laptop.
#[track_caller]
fn assert_ada_prompted(ada: &Person) {
    assert_prompted(&shown_by(&ada.machine, &[DEVICES]), LAPTOP);
}

/// Fails if Ada's first machine's `skald devices` or `skald recv` names her laptop to link.
#[track_caller]
fn assert_ada_not_prompted(ada: &Person) {
    for args in [[DEVICES], [RECV]] {
        assert_not_prompted(&shown_by(&ada.machine, &args), LAPTOP);
    }
}

/// Ada's laptop waits for linking, and her first machine's `skald devices` names it; she types
/// its code into `skald link` there and confirms. Gives the laptop once both runs have ended, each
/// succeeding if `links` and failing otherwise, and the server holds no run of it.
#[track_caller]
fn linked(world: &World, ada: &Person, links: bool) -> NewDevice {
    let mut new = waiting_device(world, ada, LAPTOP);
    assert_ada_prompted(ada);
    let issuing = link_with(ada, &typed(&new.code()), LAPTOP, YES);
    let laptop = new.finish();
    for (who, shown) in [("skald link", issuing), ("the laptop", laptop)] {
        assert_eq!(shown.success, links, "{who}: {}", shown.text);
    }
    until_not_waiting(&world.cloud, ada, &new);
    new
}

/// Ada's laptop waits for linking, and her first machine names it; she links it there with
/// `skald link`. Then neither her first machine's `skald devices` and `skald recv` nor the
/// laptop's own `skald devices` name it to link.
#[test]
fn no_command_names_a_device_once_it_is_linked() {
    let (world, _front) = fronted();
    let ada = world.sign_up(4_178_501_001, "ada-links-the-named-laptop", WORKSTATION);
    let new = linked(&world, &ada, true);

    assert_ada_not_prompted(&ada);
    assert_not_prompted(&shown_by(&new.machine, &[DEVICES]), LAPTOP);
}

/// Ada's laptop, whose codes live 30 seconds, waits for linking, and her first machine names it.
/// Its code expires: her first machine's `skald devices` and `skald recv` no longer name it.
#[test]
fn no_command_names_a_device_once_its_code_ends() {
    let (world, _front) = fronted();
    let ada = world.sign_up(4_178_502_001, "ada-lets-the-code-expire", WORKSTATION);
    let machine = Machine::new()
        .with_host_name(LAPTOP)
        .with_env(LIFETIME_SETTING, "30");
    let mut new = NewDevice::start_on(&world, &ada, machine, LAPTOP);
    new.open_run(&world.cloud, &ada);
    assert_ada_prompted(&ada);

    new.run()
        .wait_for("that the code expired", |shown| says(shown, &[EXPIRED]));
    until_not_waiting(&world.cloud, &ada, &new);

    assert_ada_not_prompted(&ada);
}

/// Under a limit of 2 devices, Ada's account has her first machine and one linked by hand. Her
/// laptop waits for linking, and her first machine names it. Linked there, its certificate is
/// refused as over the limit and the laptop ends its run: her first machine's `skald devices` and
/// `skald recv` no longer name it, nor does the laptop's own next run.
#[test]
fn no_command_names_a_device_once_its_certificate_is_over_the_devices_limit() {
    let (world, _front) = fronted_with(|settings: &mut Settings| settings.devices_limit = Some(2));
    let ada = world.sign_up(
        4_178_503_001,
        "ada-names-a-laptop-at-the-limit",
        WORKSTATION,
    );
    let _desktop = second_machine(&world, &ada, DESKTOP);
    let new = linked(&world, &ada, false);

    assert_ada_not_prompted(&ada);
    let later = run(&new.machine, &[DEVICES]);
    assert_not_prompted(&later.shown(), LAPTOP);
}
