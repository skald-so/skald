//! Each member's linked devices: those with a certificate that no counting revocation revokes, by
//! the server's checks (`docs/protocol/membership.md`, "Revocation" and "Device checks").

use std::collections::BTreeMap;

use skald_proto::DeviceKind;

use crate::listed::{Web, members};

/// The number of linked devices of each member that `cookie`'s web session gets, by handle.
fn counts(web: &Web, cookie: &str) -> BTreeMap<String, u64> {
    members(&web.api(), web.team.space, cookie)
        .into_iter()
        .map(|(handle, member)| (handle, member.linked_devices))
        .collect()
}

/// Alice, bob and carol each have `Laptop`, certified by their master key, and `Desktop`, waiting
/// for linking. Alice's laptop also certifies `Tablet` and `Phone`, which certifies `Watch`; then
/// it revokes `Phone`, and with it `Watch`. Bob's master key certifies `Old laptop` and revokes it.
/// Alice has 2 linked devices, bob and carol 1 each. Once alice's master key revokes her laptop,
/// keeping `Tablet` and `Phone`, it voids the laptop's revocation: `Tablet`, `Phone` and `Watch`
/// are linked, 3.
#[test]
fn counts_leave_out_revoked_devices_and_those_waiting_for_linking() {
    let web = Web::new(3_225_300_001);
    let team = &web.team;
    let db = &team.flow.stand.db;
    let (alice, bob) = (&team.alice, &team.bob);
    team.with_members();
    let count_of = |alice: u64, bob: u64, carol: u64| {
        BTreeMap::from([
            (team.alice.handle.to_string(), alice),
            (team.bob.handle.to_string(), bob),
            (team.carol.handle.to_string(), carol),
        ])
    };
    assert_eq!(counts(&web, &web.alice), count_of(1, 1, 1), "laptops only");

    let tablet = alice.certified_by(db, &alice.laptop, DeviceKind::Client, "Tablet");
    let phone = alice.certified_by(db, &alice.laptop, DeviceKind::Client, "Phone");
    alice.certified_by(db, &phone, DeviceKind::Client, "Watch");
    alice.revoke(&alice.by_device(&alice.laptop, phone.id));
    let old = bob.certified_by_key(db, &bob.master, "Old laptop");
    bob.revoke(&bob.by_master(old.id, &[]));
    for (cookie, who) in [(&web.alice, "alice"), (&web.carol, "carol")] {
        assert_eq!(counts(&web, cookie), count_of(2, 1, 1), "{who}");
    }

    alice.revoke(&alice.by_master(alice.laptop.id, &[tablet.id, phone.id]));
    assert_eq!(
        counts(&web, &web.bob),
        count_of(3, 1, 1),
        "alice's laptop revoked by her master key, keeping the tablet and the phone"
    );
}
