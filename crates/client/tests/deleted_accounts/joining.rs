//! A device outside a channel's group joins it by itself when the group holds no admitted device
//! but revoked ones and those of a deleted account, and removes them.

use crate::devices::second_machine;
use crate::kit::{assert_members, group, received, warnings_naming};
use crate::linked::{FOURTH_HOST, signed_keeping};
use crate::scene::delete_account;
use crate::talk::Talk;

/// Ada and Bob in a channel of `acme`, each with a device in its group, and Ada's fourth machine,
/// linked by her first, which has not run yet. Ada's first machine's device revokes itself keeping
/// the fourth's, and Bob deletes his account: no admitted device is left in the group. The fourth
/// machine's `skald recv` joins the channel by itself and removes both devices: the server's group
/// holds its device alone, and the run warns of neither.
#[test]
fn group_of_a_revoked_device_and_a_deleted_accounts_is_joined_and_both_removed() {
    let talk = Talk::new(
        &[4_305_107_001, 4_305_107_002],
        &["ada-joins-past-deleted", "bob-deletes-meanwhile"],
        false,
    );
    let fourth = second_machine(&talk.world, &talk.ada, FOURTH_HOST);
    let cloud = talk.cloud();
    let (space, channel) = (talk.space.id, talk.channel);
    let bob = talk.played[0].device();
    signed_keeping(cloud, &talk.ada, talk.ada.device, &[fourth.device]);
    delete_account(cloud, &talk.played[0].person);
    assert_members(
        &group(cloud, &talk.ada.token, space, channel).members,
        &[talk.ada.device, bob],
    );

    let ran = received(&fourth.machine);

    assert_members(
        &group(cloud, &fourth.token, space, channel).members,
        &[fourth.device],
    );
    for device in [talk.ada.device, bob] {
        let warned = warnings_naming(&ran, device);
        assert!(warned.is_empty(), "warns of {device}: {ran:?}");
    }
}
