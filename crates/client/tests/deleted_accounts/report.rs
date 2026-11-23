//! A server that reports an account deleted that is not: the member devices remove its devices,
//! which rejoin by themselves once the report stops.

use skald_proto::Role;

use crate::chain::joins_by_itself;
use crate::fixtures::send;
use crate::kit::received;
use crate::scene::{Scene, assert_read_from, one_message_sent, report_deleted, stop_reporting};

/// What Bob says once Ada's devices are back.
const BACK: &str = "welcome back, the report was wrong";

/// The relay reports Ada's account, which is not deleted, deleted to every machine. Bob's next
/// `skald recv` removes Ada's two devices from both channels: the server's GroupInfo of each holds
/// Bob's and Carol's devices alone. The relay stops the report, and Ada's devices rejoin each
/// channel by themselves, with external Commits. Bob's next `skald recv` commits nothing, and the
/// server's GroupInfo of each channel holds all four devices; Bob's `skald send` in the channel
/// then commits nothing, and both of Ada's devices read his line.
#[test]
fn devices_of_an_account_falsely_reported_deleted_are_removed_and_rejoin_once_the_report_stops() {
    let mut scene = Scene::new(
        [4_305_104_001, 4_305_104_002, 4_305_104_003],
        [
            "bob-misled-once",
            "ada-reported-gone",
            "carol-watches-report",
        ],
        Role::Member,
    );
    let cloud = &scene.world.cloud;
    let space = scene.space.id;
    let (bob, carol) = (scene.bob.device, scene.carol.device());
    report_deleted(cloud, space, scene.ada().account);

    received(&scene.bob.machine);

    scene.assert_groups(&[bob, carol]);
    stop_reporting(cloud, space);
    for one in &mut scene.ada {
        one.take_in(cloud);
        for channel in [scene.space.default, scene.channel] {
            joins_by_itself(cloud, one, (space, channel));
        }
    }
    let from = scene.front.sent().len();

    let ran = received(&scene.bob.machine);

    let mut all = scene.ada_devices();
    all.extend([bob, carol]);
    scene.assert_groups(&all);
    let sent = send(&scene.bob.machine, scene.channel, BACK);
    assert!(sent.success, "{sent:?}");
    scene.assert_no_commit(from, &[&ran, &sent]);
    let msg_id = one_message_sent(&scene.front, scene.channel, from);
    for one in &mut scene.ada {
        one.take_in(cloud);
        assert_read_from(one, scene.channel, msg_id, bob);
    }
}
