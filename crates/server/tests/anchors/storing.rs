//! Storing anchors objects: once, also when sent twice; the last one per device, replaced by the
//! device's next one and kept once the device is revoked; given byte for byte, and only to the
//! account's sessions.

use crate::common::device_flow::text;
use crate::fixture::Fixture;
use crate::kit::{by_bobs_laptop, sorted};
use crate::objects::{assert_invalid, assert_stored, signed};
use crate::rosters::with_later_field;

/// Bob's laptop's anchors object, with a field of a later version, is stored when sent, `201`, and
/// taken as stored when sent again, `200`; bob's phone's is stored beside it. Each device's next
/// one replaces its last, which `GET /account/anchors` then no longer gives, and the same bytes
/// again are taken as stored. Bob's session gets the last one of each of his devices as sent;
/// alice's and eve's get none of them, and eve's gets her own alone.
#[test]
fn an_anchors_object_is_stored_once_given_byte_for_byte_and_replaced_by_the_devices_next_one() {
    let f = Fixture::new(3_172_020_101);
    let t = &f.team;
    let bob = &t.bob;
    let phone = by_bobs_laptop(&f, "Phone");

    let first = signed(
        &with_later_field(&bob.anchors_body(&bob.laptop, 1)),
        &bob.laptop.signing.key,
    );
    assert_stored(&bob.session, &first, "the laptop's first");
    let again = bob.session.store_anchors(&first);
    assert_eq!(
        again.status,
        200,
        "the laptop's first, again: {}",
        text(&again)
    );
    assert_eq!(bob.session.anchors(), sorted(&[&first]), "once, as sent");

    let phones = bob.anchors_of(&phone, 1);
    assert_stored(&bob.session, &phones, "the phone's first");
    assert_eq!(bob.session.anchors(), sorted(&[&first, &phones]));

    let next = bob.anchors_of(&bob.laptop, 2);
    assert_stored(&bob.session, &next, "the laptop's next");
    let again = bob.session.store_anchors(&next);
    assert_eq!(
        again.status,
        200,
        "the laptop's next, again: {}",
        text(&again)
    );
    assert_eq!(
        bob.session.anchors(),
        sorted(&[&next, &phones]),
        "the laptop's next replaced its first"
    );
    let phones_next = bob.anchors_of(&phone, 3);
    assert_stored(&bob.session, &phones_next, "the phone's next");
    assert_eq!(bob.session.anchors(), sorted(&[&next, &phones_next]));

    let eve = &f.eve;
    let eves = eve.anchors_of(&eve.laptop, 1);
    assert_stored(&eve.session, &eves, "eve's");
    assert_eq!(t.alice.session.anchors(), sorted(&[]), "to alice");
    assert_eq!(eve.session.anchors(), sorted(&[&eves]), "to eve");
    assert_eq!(
        bob.session.anchors(),
        sorted(&[&next, &phones_next]),
        "to bob, after eve's"
    );
}

/// Bob's phone stores an anchors object, then bob's laptop revokes it. The phone's last anchors
/// object stays and is given to bob's session; a later one of the phone is refused and not
/// stored, while the laptop's still is.
#[test]
fn a_revoked_devices_last_anchors_object_stays_and_is_given() {
    let f = Fixture::new(3_172_020_201);
    let bob = &f.team.bob;
    let phone = by_bobs_laptop(&f, "Phone");
    let last = bob.anchors_of(&phone, 1);
    assert_stored(&bob.session, &last, "the phone's");

    bob.revoke(&bob.by_device(&bob.laptop, phone.id));

    assert_eq!(
        bob.session.anchors(),
        sorted(&[&last]),
        "after the revocation"
    );
    assert_invalid(
        &bob.session.store_anchors(&bob.anchors_of(&phone, 2)),
        "a revoked device's next",
    );
    assert_eq!(bob.session.anchors(), sorted(&[&last]), "after its next");
    let laptops = bob.anchors_of(&bob.laptop, 2);
    assert_stored(&bob.session, &laptops, "the laptop's");
    assert_eq!(bob.session.anchors(), sorted(&[&last, &laptops]));
}
