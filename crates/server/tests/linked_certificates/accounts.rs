//! What the tests of this binary share: storing a certificate from a person's session, issuing one
//! with a person's laptop, and the certificates the account's devices hold.

use skald_proto::DeviceKind;

use crate::api::{Person, Session};
use crate::common::Response;
use crate::common::device_flow::DeviceFlow;
use crate::common::devices::{DEVICES, Device, assert_taken, devices_of};
use crate::upload::{issued_by, put_cert};

/// Checks that a response refuses a certificate: a `4xx` other than `401`, with a problem details
/// object.
pub use crate::kit::assert_refused_link as assert_refused_cert;

/// `PUT /devices/<device>/cert` of `cert` from `session` on `flow`'s server.
pub fn put(flow: &DeviceFlow, session: &Session, device: &Device, cert: &[u8]) -> Response {
    put_cert(flow.stand.server.base(), session.token(), device.id, cert)
}

/// Registers a new `client` device named `name` on `person`'s account, without a certificate;
/// gives it.
pub fn register(person: &Person, name: &str) -> Device {
    let device = Device::new(name);
    assert_taken(
        &person
            .session
            .call("post", DEVICES, Some(&device.registration())),
    );
    device
}

/// The certificate of `device`, registered on `person`'s account, issued by `person`'s laptop.
pub fn by_laptop(person: &Person, device: &Device) -> Vec<u8> {
    issued_by(device, person.account, DeviceKind::Client, &person.laptop)
}

/// The certificate `GET /devices` gives `person`'s session for `device`: `None` without one.
#[track_caller]
pub fn held(person: &Person, device: &Device) -> Option<Vec<u8>> {
    let listed = devices_of(&person.session.call("get", DEVICES, None));
    listed
        .get(&device.id.to_string())
        .unwrap_or_else(|| panic!("{} is not listed: {listed:?}", device.name))
        .clone()
}

/// The number of devices `GET /devices` gives `person`'s session with a certificate.
pub fn certified(person: &Person) -> usize {
    let listed = devices_of(&person.session.call("get", DEVICES, None));
    listed.values().filter(|cert| cert.is_some()).count()
}
