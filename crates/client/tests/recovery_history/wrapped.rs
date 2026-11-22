//! Member devices wrap each clean epoch's key to the recovery key of each account among the
//! channel's members that has a record, and no unclean epoch's.

use skald_proto::Role;

use crate::chain::{adds_to_channel, carol};
use crate::joins::{intruder, intruder_joins};
use crate::kit::{group, received};
use crate::{
    assert_wrapped_to_recovery, recovery_epochs, recovery_hpke, talk_with_key, without_record,
};

/// In the talk of Ada and Bob, epoch 1 of the channel holds their devices. Carol signs up, and the
/// server's database then holds no record of her account, as for an account made before records.
/// A revision Ada's device signs adds Carol to the channel, and Ada's next `skald recv` commits the
/// Add of Carol's device: epoch 2. A device the server lets in on Bob's account, its certificate
/// chained to another master key, joins the group by itself: epoch 3, which Ada's next runs judge
/// unclean before they commit its Remove: epoch 4. The server then holds one wrap to Ada's recovery
/// key of each of epochs 1, 2 and 4, each opening with that key, signed by Ada's device and giving
/// the key Bob's device derives; wraps to Bob's recovery key of the same epochs; none of epoch 3;
/// and none to Carol's account.
#[test]
fn each_clean_epoch_is_wrapped_to_the_recovery_key_of_each_member_account_with_a_record() {
    let (mut talk, key) = talk_with_key(
        &[4_177_702_001, 4_177_702_002],
        &["ada-wraps-to-recovery", "bob-wrapped-as-well"],
        Role::Member,
    );
    let carol = carol(&talk, 4_177_702_003, "carol-before-records");
    let cloud = &talk.world.cloud;
    without_record(cloud, carol.person.account);
    adds_to_channel(cloud, &talk.ada, talk.at(), &carol.person);
    received(&talk.ada.machine);
    let intruder = intruder(cloud, &talk.played[0].person, "bob-server-made-device");
    intruder_joins(cloud, &intruder, talk.at());

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    let (space, channel) = talk.at();
    let members = [talk.ada.device, talk.played[0].device(), carol.device()];
    group(talk.cloud(), &talk.ada.token, space, channel).assert_is(4, &members);
    let recovery = recovery_hpke(&key, talk.ada.account);
    let ada = talk.ada.account;
    for epoch in [1, 2, 4] {
        assert_wrapped_to_recovery(&mut talk, (ada, &recovery), epoch);
    }
    let mut ada_s = recovery_epochs(&talk, ada);
    ada_s.sort_unstable();
    assert!(
        !ada_s.contains(&3),
        "epoch 3 wrapped to Ada's recovery key: {ada_s:?}"
    );
    let mut bob_s = recovery_epochs(&talk, talk.played[0].person.account);
    bob_s.sort_unstable();
    for epoch in [1, 2, 4] {
        assert_eq!(
            bob_s.iter().filter(|held| **held == epoch).count(),
            1,
            "not one wrap of epoch {epoch} to Bob's recovery key: {bob_s:?}"
        );
    }
    assert!(
        !bob_s.contains(&3),
        "epoch 3 wrapped to Bob's recovery key: {bob_s:?}"
    );
    let carol_s = recovery_epochs(&talk, carol.person.account);
    assert!(
        carol_s.is_empty(),
        "wraps to the recovery key of Carol's account, which has no record: {carol_s:?}"
    );
}
