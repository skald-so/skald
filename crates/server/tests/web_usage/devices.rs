//! The account's devices, as the devices limit counts them: those with a certificate that no
//! counting revocation revokes, `service` devices aside (`docs/protocol/membership.md`,
//! "Revocation" and "Device checks").

use skald_proto::DeviceKind;

use crate::common::devices::{DEVICES as REGISTER, Device};
use crate::limits::{DEVICES, add_devices};
use crate::upload::{assert_over_limit, issued_by, put_cert};
use crate::usage::{Web, account_usage, of};

/// With the setting at 4: alice's laptop is certified and her desktop waits for linking: 1 of 4.
/// Her laptop certifies `Tablet`, `Phone` and a `service` device: 3 of 4. It revokes `Phone`: 2
/// of 4. Her master key revokes the laptop, keeping `Tablet` and `Phone`, which voids the laptop's
/// revocation of `Phone`: `Tablet` and `Phone`, 2 of 4. With 2 more certified by her master key, 4
/// of 4: her next device registers, its certificate by `Tablet` is refused, and the count stays at
/// 4. Bob's stays at 1 of 4 throughout.
#[test]
fn the_count_leaves_out_devices_waiting_for_linking_revoked_and_service_ones() {
    let web = Web::with(3_226_300_001, &[DEVICES, "4"]);
    let (fixture, api) = (&web.fixture, web.api());
    let (alice, db) = (&fixture.team.alice, fixture.db());
    let devices = |cookie: &str| account_usage(&api, cookie).devices;
    assert_eq!(devices(&web.alice), of(1, 4), "the laptop");

    let by_laptop = |kind, name| alice.certified_by(db, &alice.laptop, kind, name);
    let [tablet, phone] = ["Tablet", "Phone"].map(|name| by_laptop(DeviceKind::Client, name));
    by_laptop(DeviceKind::Service, "Agent");
    assert_eq!(
        devices(&web.alice),
        of(3, 4),
        "with a tablet, a phone and a service"
    );

    alice.revoke(&alice.by_device(&alice.laptop, phone.id));
    assert_eq!(devices(&web.alice), of(2, 4), "the phone revoked");

    alice.revoke(&alice.by_master(alice.laptop.id, &[tablet.id, phone.id]));
    assert_eq!(
        devices(&web.alice),
        of(2, 4),
        "the laptop revoked, the phone kept"
    );

    add_devices(db, alice, 2, DeviceKind::Client, true);
    assert_eq!(devices(&web.alice), of(4, 4), "two more");
    let next = Device::new("One too many");
    let registered = alice
        .session
        .call("post", REGISTER, Some(&next.registration()));
    assert_eq!(registered.status, 201, "a registration at the limit");
    let cert = issued_by(&next, alice.account, DeviceKind::Client, &tablet);
    let base = fixture.team.flow.stand.server.base();
    let refused = put_cert(base, alice.session.token(), next.id, &cert);
    assert_over_limit(&refused, "the next device's certificate");
    assert_eq!(devices(&web.alice), of(4, 4), "at the limit");
    assert_eq!(devices(&web.bob), of(1, 4), "bob");
}
