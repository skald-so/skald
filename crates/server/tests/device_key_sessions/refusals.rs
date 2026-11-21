//! Refused, with no session given and no certificate stored: a device without a certificate, a
//! revoked one, one of another account, a request not signed with the device's key, and a
//! certificate over the devices limit.

use skald_proto::DeviceKind;

use crate::api::assert_refused;
use crate::common::devices::{DEVICES, Device, Key, devices_of, random};
use crate::kit::{ask, body_by, challenge, sign_in_by_key, token_of};
use crate::linked::Linked;
use crate::upload::{assert_over_limit, issued_by};

/// The setting of how many devices with a certificate an account may hold.
const DEVICES_LIMIT: &str = "--devices-limit";

/// Checks that `response` gives no session: a `4xx` with a problem details object.
#[track_caller]
fn assert_no_session(response: &crate::common::Response, what: &str) {
    assert_refused(response, what);
    assert!(
        crate::common::device_flow::json(response)
            .get("token")
            .is_none(),
        "{what}: no token"
    );
}

/// Checks that alice's account lists no session of the tablet and holds no certificate of it.
#[track_caller]
fn assert_nothing_given(l: &Linked, what: &str) {
    assert_eq!(l.tablet_sessions().len(), 0, "{what}: {:#?}", l.sessions());
    let held = devices_of(&l.fresh.alice.session.call("get", DEVICES, None));
    assert_eq!(
        held.get(&l.tablet.id.to_string()).cloned().flatten(),
        None,
        "{what}: no certificate of the tablet"
    );
}

#[test]
fn a_device_without_a_certificate_gets_no_session() {
    let l = Linked::start(3_175_310_111);
    let nobody = l.nobody();
    let public = &l.fresh.flow.stand.public;
    let challenge = challenge(&nobody, l.tablet.id);
    let body = body_by(&l.tablet.signing.key, &challenge, public, None);
    assert_no_session(&ask(&nobody, l.tablet.id, &body), "the paired tablet");
    assert_nothing_given(&l, "the paired tablet");

    let phone = Device::new("Phone");
    let challenge = crate::kit::challenge(&nobody, phone.id);
    let body = body_by(&phone.signing.key, &challenge, public, None);
    assert_no_session(
        &ask(&nobody, phone.id, &body),
        "a device the server does not have",
    );

    // Its certificate gets it one.
    token_of(&l.ask());
}

#[test]
fn a_request_not_signed_with_the_devices_key_gets_no_session() {
    let l = Linked::start(3_175_310_121);
    let nobody = l.nobody();
    let public: &str = &l.fresh.flow.stand.public;
    let cert = Some(l.cert.as_slice());
    let tablet = &l.tablet;
    let other = Key::generate();
    for (key, origin, issued, what) in [
        (&other.key, public, true, "another key"),
        (
            &l.fresh.alice.laptop.signing.key,
            public,
            true,
            "the issuing device's key",
        ),
        (
            &tablet.signing.key,
            "https://elsewhere.example",
            true,
            "another origin",
        ),
        (
            &tablet.signing.key,
            public,
            false,
            "a challenge the server did not issue",
        ),
    ] {
        let challenge = if issued {
            challenge(&nobody, tablet.id)
        } else {
            random().to_vec()
        };
        let body = body_by(key, &challenge, origin, cert);
        assert_no_session(&ask(&nobody, tablet.id, &body), what);
        assert_nothing_given(&l, what);
    }

    // Signed with its key, the request gets a session.
    token_of(&l.ask());
}

#[test]
fn a_revoked_device_gets_no_session() {
    let l = Linked::start(3_175_310_131);
    let alice = &l.fresh.alice;
    token_of(&l.ask());
    alice.revoke(&alice.by_device(&alice.laptop, l.tablet.id));
    assert_no_session(&l.ask(), "the revoked tablet");
    assert_eq!(l.tablet_sessions().len(), 0, "{:#?}", l.sessions());
}

#[test]
fn a_device_of_another_account_gets_no_session() {
    let l = Linked::start(3_175_310_141);
    let flow = &l.fresh.flow;
    let nobody = l.nobody();
    let (alice, bob) = (&l.fresh.alice, &l.fresh.bob);

    let by_bob = issued_by(&l.tablet, alice.account, DeviceKind::Client, &bob.laptop);
    assert_no_session(
        &sign_in_by_key(flow, &nobody, &l.tablet, &by_bob),
        "alice's tablet certified by bob's laptop",
    );
    let on_bobs = issued_by(&l.tablet, bob.account, DeviceKind::Client, &alice.laptop);
    assert_no_session(
        &sign_in_by_key(flow, &nobody, &l.tablet, &on_bobs),
        "a certificate on bob's account by alice's laptop",
    );
    assert_nothing_given(&l, "certificates of another account");

    // bob's device, registered on his account, with a certificate on alice's account.
    let bobs = &bob.uncertified;
    let on_alices = issued_by(bobs, alice.account, DeviceKind::Client, &alice.laptop);
    assert_no_session(
        &sign_in_by_key(flow, &nobody, bobs, &on_alices),
        "bob's device certified on alice's account",
    );
    let held = devices_of(&alice.session.call("get", DEVICES, None));
    assert!(
        !held.contains_key(&bobs.id.to_string()),
        "alice's account does not hold bob's device: {held:?}"
    );
}

#[test]
fn a_certificate_over_the_devices_limit_gives_no_session() {
    // alice's laptop takes the only place.
    let l = Linked::with(3_175_310_151, &[DEVICES_LIMIT, "1"]);
    let refused = l.ask();
    assert_over_limit(&refused, "the tablet over the limit");
    assert!(
        crate::common::device_flow::json(&refused)
            .get("token")
            .is_none(),
        "no token"
    );
    assert_nothing_given(&l, "over the limit");
}
