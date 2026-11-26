//! A device of the newcomer's account whose certificate is chained to another master key than the
//! acceptance's: the newcomer is admitted, the device is not added, and when it joins by itself a
//! member device removes it with a warning.

use crate::chain::joins_by_itself;
use crate::joins::{Intruder, intruder};
use crate::kit::{received, warnings_naming};
use crate::played::Played;
use crate::world::ACME;
use crate::{Scene, group_members};

/// The server's database holds a second `client` device of Bob's account, its certificate chained
/// to another master key, as a server would set up a device of its own, with KeyPackages on the
/// server. Bob accepts the invite. Ada's `skald recv` admits Bob and adds his first device to both
/// channels, not the second one. The second device then joins the default channel by itself; Ada's
/// next `skald recv` warns naming it and removes it, leaving Ada's, Carol's and Bob's first device
/// in the group.
#[test]
fn newcomer_s_device_chained_to_another_master_key_is_not_added_and_removed_when_it_joins() {
    let scene = Scene::new(
        ACME,
        &[
            (4_855_301_001, "ada-checks-bob-devices"),
            (4_855_301_002, "carol-watches-devices"),
            (4_855_301_003, "bob-with-a-planted-device"),
        ],
    );
    let cloud = scene.cloud();
    let Intruder { person, .. } = intruder(cloud, &scene.bob.person, "bob-server-device");
    let planted = person.device;
    let mut stray = Played::new(cloud, person);
    let made = scene.invite();
    scene.bob_accepts(&made);
    let previous = scene.latest();

    received(&scene.ada.machine);

    scene.assert_bob_admitted(&previous, scene.ada.device, &made);
    let devices = [scene.ada.device, scene.carol.device(), scene.bob.device()];
    for at in scene.both() {
        assert_eq!(
            group_members(cloud, &scene.ada.token, at),
            devices.into_iter().collect(),
            "the group of {}",
            at.1
        );
    }
    let [default, _] = scene.both();
    joins_by_itself(cloud, &mut stray, default);

    let ran = received(&scene.ada.machine);

    assert!(
        !warnings_naming(&ran, planted).is_empty(),
        "no warning naming the planted device: {ran:?}"
    );
    assert_eq!(
        group_members(cloud, &scene.ada.token, default),
        devices.into_iter().collect(),
        "the planted device is still in the default channel's group"
    );
}
