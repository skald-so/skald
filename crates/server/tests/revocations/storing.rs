//! Storing counting revocations once and giving them with the certificates, byte for byte.

use crate::common::device_flow::text;
use crate::common::devices::{DEVICES, devices_of};
use crate::fixture::Fixture;
use crate::kit::{body, by_bobs_laptop, signed_with_later_field, sorted};

/// Revocations by another device of bob's account, by the revoked device itself and by bob's
/// master key, one with a field of a later version, are each stored when sent, `201`, and taken as
/// stored when sent again, `200`. Bob's devices, and the devices of a space bob is a member of,
/// come with each of them once, as sent, and the devices keep their certificates. Another
/// account's devices, and a space it is not in, come without them.
#[test]
fn revocations_by_a_device_by_itself_and_by_the_master_key_are_stored_once_and_given() {
    let f = Fixture::new(3_131_320_101);
    let db = f.db();
    let t = &f.team;
    let bob = &t.bob;
    let phone = by_bobs_laptop(&f, "Phone");
    let tablet = by_bobs_laptop(&f, "Tablet");
    let watch = bob.certified_by_key(db, &bob.master, "Watch");
    let certs = devices_of(&bob.session.call("get", DEVICES, None));

    let by_laptop = bob.by_device(&bob.laptop, phone.id);
    let by_itself = bob.by_device(&tablet, tablet.id);
    let by_master = signed_with_later_field(&body(bob, watch.id, None), &bob.master.key);
    for (revocation, what) in [
        (&by_laptop, "by another device"),
        (&by_itself, "by the device itself"),
        (&by_master, "by the master key, with a later field"),
    ] {
        let first = bob.session.revoke(revocation);
        assert_eq!(first.status, 201, "{what}: {}", text(&first));
        let again = bob.session.revoke(revocation);
        assert_eq!(again.status, 200, "{what}, again: {}", text(&again));
    }

    let eve = &f.eve;
    let eves = eve.by_master(eve.laptop.id, &[]);
    eve.revoke(&eves);

    let all = sorted(&[&by_laptop, &by_itself, &by_master]);
    assert_eq!(bob.session.revocations(), all, "bob's devices");
    assert_eq!(
        devices_of(&bob.session.call("get", DEVICES, None)),
        certs,
        "bob's certificates"
    );
    assert_eq!(
        t.alice.session.space_revocations(t.space),
        all,
        "acme's devices, to alice"
    );
    assert_eq!(
        bob.session.space_revocations(t.space),
        all,
        "acme's devices, to bob"
    );
    assert_eq!(
        t.alice.session.revocations(),
        sorted(&[]),
        "alice's devices"
    );
    assert_eq!(eve.session.revocations(), sorted(&[&eves]), "eve's devices");
}
