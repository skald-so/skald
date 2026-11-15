//! A run that cannot keep the device's KeyPackages warns of it and goes on.

use crate::kit::{Pair, RECV};
use crate::world::run;

/// With the server's table of KeyPackages gone, its count fails: Ada's `skald recv` warns that
/// the device's KeyPackages are not kept, and still gets the mailbox.
#[test]
fn run_that_cannot_keep_key_packages_warns_and_goes_on() {
    let pair = Pair::start(
        [4_130_030_001, 4_130_030_002],
        ["ada-kp-warned-standin", "bob-kp-warned-standin"],
    );
    pair.cloud().execute("DROP TABLE key_packages");

    let finished = run(&pair.ada.machine, &[RECV]);

    assert!(finished.success, "{finished:?}");
    assert!(
        finished
            .stderr
            .contains("warning: this device's KeyPackages on the server are not kept"),
        "{finished:?}"
    );
}
