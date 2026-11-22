//! A device of the account that runs no helper when the account is deleted learns it from the
//! server once its session is refused: each command that would reach the server then says so and
//! sends nothing, and its `skald delete-account` deletes its local state.

use crate::deletion::{Held, assert_says_deleted_once, deleted_at_terminal};
use crate::linking::{NewDevice, link};
use crate::refused_sessions::assert_finishes;
use crate::talk::ADA_HOST;
use crate::terminal::Terminal;
use crate::world::{Person, World, run};

/// The host name of Ada's second machine.
const LAPTOP: &str = "ada-laptop-offline";

/// A second machine of `first`'s account, named `name`, linked with a link code from its machine.
fn linked(world: &World, first: &Person, name: &'static str) -> Person {
    let mut second = NewDevice::start(world, first, name);
    link(&world.cloud, first, &mut second);
    second.into_person(&world.cloud, first)
}

/// Ada's second machine, linked with a link code, runs no helper when her first machine's `skald
/// delete-account` deletes her account. The second machine's `skald recv`, then its `skald recv
/// --wait` and its `skald link` at a terminal, each say once that the server reports the account
/// deleted and name the machine's state directory; none sends an envelope or a history upload.
/// Its `skald delete-account` at a terminal then finishes it.
#[test]
fn a_device_off_during_the_deletion_says_it_is_deleted_and_sends_nothing() {
    let (world, front) = crate::fixtures::fronted();
    let ada = world.sign_up(6_120_008_001, "ada-leaves-a-laptop-off", ADA_HOST);
    let laptop = linked(&world, &ada, LAPTOP);
    let held = Held::of(&world.cloud, &laptop);
    let quiet = || (front.sent().len(), front.uploads().len());
    let before = quiet();

    deleted_at_terminal(&world, &ada);

    let machine = &laptop.machine;
    for args in [&["recv"][..], &["recv", "--wait"]] {
        let shown = run(machine, args).shown();
        assert_says_deleted_once(&shown, machine, &format!("skald {}", args.join(" ")));
    }
    let linking = Terminal::start(machine, &["link"]).finish();
    assert!(!linking.success, "{}", linking.text);
    assert_says_deleted_once(&linking.text, machine, "skald link");
    assert_eq!(quiet(), before, "an envelope or a history upload was sent");

    assert_finishes(&world, &laptop, &held);
}
