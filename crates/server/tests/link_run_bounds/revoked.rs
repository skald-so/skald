//! A device that checks 3 and 4 of "Device checks" revoke gets no session by its key: one whose
//! issuer is revoked, whether the server holds its certificate or it sends one, which the server
//! then does not store.

use skald_proto::DeviceKind;

use crate::assert_problem;
use crate::by_key::{sign_in_by_key, token_of};
use crate::common::devices::{DEVICES, Device, devices_of};
use crate::kit::Fresh;
use crate::upload::issued_by;

#[test]
fn a_device_certified_by_a_revoked_device_gets_no_session() {
    let f = Fresh::start(3_175_390_211);
    let alice = &f.alice;
    let nobody = f.nobody();
    let ask = |device: &Device, cert: &[u8]| sign_in_by_key(&f.flow, &nobody, device, cert);

    // The tablet, certified by alice's laptop, and the phone, certified by the tablet.
    let tablet = Device::new("Tablet");
    let tablet_cert = issued_by(&tablet, alice.account, DeviceKind::Client, &alice.laptop);
    token_of(&ask(&tablet, &tablet_cert));
    let phone = Device::new("Phone");
    let phone_cert = issued_by(&phone, alice.account, DeviceKind::Client, &tablet);
    token_of(&ask(&phone, &phone_cert));

    alice.revoke(&alice.by_device(&alice.laptop, tablet.id));

    assert_problem(
        &ask(&phone, &phone_cert),
        403,
        "device-revoked",
        "the phone, whose certificate the server holds",
    );
    // Checks 3 and 4 refuse the held certificate, not only the column the revocation set.
    f.flow.stand.db.execute(&format!(
        "UPDATE devices SET revoked = false WHERE id = '{}'",
        phone.id
    ));
    assert_problem(
        &ask(&phone, &phone_cert),
        403,
        "device-revoked",
        "the phone, held, with its revoked column cleared",
    );
    let watch = Device::new("Watch");
    let watch_cert = issued_by(&watch, alice.account, DeviceKind::Client, &tablet);
    assert_problem(
        &ask(&watch, &watch_cert),
        403,
        "device-revoked",
        "a certificate the revoked tablet issued",
    );
    let held = devices_of(&alice.session.call("get", DEVICES, None));
    assert!(
        !held.contains_key(&watch.id.to_string()),
        "the watch's certificate is not stored: {held:?}"
    );
}
