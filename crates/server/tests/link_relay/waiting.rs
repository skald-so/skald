//! While a run is open, the account's sessions get its device's registered `name` and kind; no
//! session of another account does, and none once the run has ended.

use crate::common::devices::assert_taken;
use crate::kit::{Link, Waiting, abort};

#[test]
fn the_waiting_device_is_given_while_its_run_is_open() {
    let l = Link::start(3_170_300_311);
    assert_eq!(l.alice.session.waiting(), Vec::new(), "before the hello");

    let (session, _) = l.open();
    let desktop = vec![Waiting::of(&l.alice.uncertified)];
    assert_eq!(desktop[0].name, "Desktop");
    assert_eq!(
        l.alice.session.waiting(),
        desktop,
        "to the issuing device's session"
    );
    assert_eq!(l.new_side.waiting(), desktop, "to the new device's session");
    assert_eq!(l.bob.session.waiting(), Vec::new(), "to no other account");

    assert_taken(&l.new_side.send_message(&abort(&session)));
    l.alice.session.until_not_waiting(l.desktop());
    assert_eq!(l.new_side.waiting(), Vec::new(), "once the run has ended");
}
