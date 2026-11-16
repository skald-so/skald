//! A revoked device's KeyPackages: the server deletes them at revocation, hands out none and takes
//! no more.

use crate::api::assert_refused;
use crate::fixture::{
    Stand, fresh, hand_out, hand_out_one, last_resort, mls, revoke, upload, uploaded,
};

/// Alice's phone, a second `client` device certified by her master key, has KeyPackages and a
/// last-resort one. Once a revocation by her master key revokes it, the database holds none of
/// them, bob gets none, and the server takes no more of the phone's; her laptop's are still handed
/// out.
#[test]
fn revoked_devices_key_packages_are_deleted_and_none_are_handed_out_or_taken() {
    let stand = Stand::new(3_130_040_001);
    let (alice, bob) = (&stand.team.alice, &stand.team.bob);
    let phone = stand.phone(alice);
    let mut phone_mls = mls(&phone);
    let others = fresh(&mut phone_mls, 2);
    let last = last_resort(&mut phone_mls);
    uploaded(&alice.session, phone.id, &others, Some(&last));
    let mut laptop_mls = mls(&alice.laptop);
    let laptop_one = fresh(&mut laptop_mls, 1);
    uploaded(&alice.session, alice.laptop.id, &laptop_one, None);

    revoke(alice, phone.id);

    for bytes in others.iter().chain([&last]) {
        assert!(
            !stand.holds(bytes),
            "the database holds a revoked device's KeyPackage"
        );
    }
    let handed = hand_out(&bob.session, &[phone.id, alice.laptop.id]);
    assert_eq!(handed.get(&phone.id), None, "a revoked device's KeyPackage");
    assert_eq!(handed.get(&alice.laptop.id), Some(&laptop_one[0]));
    assert_eq!(hand_out_one(&bob.session, phone.id), None);

    let more = fresh(&mut phone_mls, 1);
    assert_refused(
        &upload(&alice.session, phone.id, &more, None),
        "KeyPackages of a revoked device",
    );
    assert_refused(
        &upload(
            &alice.session,
            phone.id,
            &[],
            Some(&last_resort(&mut phone_mls)),
        ),
        "a last-resort KeyPackage of a revoked device",
    );
    assert_eq!(hand_out_one(&bob.session, phone.id), None);
}
