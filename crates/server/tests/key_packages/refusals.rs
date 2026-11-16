//! KeyPackages the server refuses: of another account's device, of a device without a valid
//! certificate or of another kind than `client`, and those that fail the checks of "KeyPackages".
//! After a refused upload, the device's count stays as it was.

use std::time::Duration;

use crate::api::{Session, assert_refused};
use crate::common::devices::Device;
use crate::common::times::DAY;
use crate::fixture::{Stand, fresh, last_resort, made_at, mls, now, status, upload, uploaded};
use crate::foreign::{self, CHACHA, Made, PROFILE};

/// 60 days: the longest lifetime a KeyPackage may span.
const LIFETIME: Duration = Duration::from_secs(60 * DAY);

/// Fails unless the server refuses `key_packages` and the last-resort `last` of `device` from
/// `session`, and the device's count stays `(unused, replace)` for `owner`, a session of its
/// account; `what` names the case.
#[track_caller]
fn assert_upload_refused(
    session: &Session,
    owner: &Session,
    device: &Device,
    (key_packages, last): (&[Vec<u8>], Option<&[u8]>),
    before: (u64, bool),
    what: &str,
) {
    assert_refused(&upload(session, device.id, key_packages, last), what);
    assert_eq!(
        status(owner, device.id),
        before,
        "{what}: the count changed"
    );
}

/// Bob's session uploads KeyPackages of alice's laptop, valid ones the laptop made: refused. So is
/// a request without a session.
#[test]
fn key_packages_of_another_accounts_device_are_refused() {
    let stand = Stand::new(3_130_020_001);
    let (alice, bob) = (&stand.team.alice, &stand.team.bob);
    let mut device = mls(&alice.laptop);
    let key_packages = fresh(&mut device, 2);
    let last = last_resort(&mut device);

    for (session, what) in [
        (&bob.session, "from another account's session"),
        (&Session::none(&stand.team.flow), "without a session"),
    ] {
        assert_upload_refused(
            session,
            &alice.session,
            &alice.laptop,
            (&key_packages, Some(&last)),
            (0, true),
            what,
        );
    }

    uploaded(&alice.session, alice.laptop.id, &key_packages, Some(&last));
    assert_eq!(status(&alice.session, alice.laptop.id), (2, false));
}

/// Alice's session uploads valid KeyPackages of its device registered without a certificate, and
/// of a `browser` device of its account with a valid certificate: refused.
#[test]
fn key_packages_of_a_device_without_a_certificate_or_not_a_client_are_refused() {
    let stand = Stand::new(3_130_020_101);
    let alice = &stand.team.alice;
    let browser = stand.browser(alice);

    for (device, what) in [
        (&alice.uncertified, "a device without a certificate"),
        (&browser, "a browser device"),
    ] {
        let mut made = mls(device);
        let key_packages = fresh(&mut made, 1);
        let last = last_resort(&mut made);
        assert_refused(
            &upload(&alice.session, device.id, &key_packages, None),
            what,
        );
        assert_refused(
            &upload(&alice.session, device.id, &[], Some(&last)),
            &format!("{what}: its last-resort one"),
        );
    }
}

/// Each KeyPackage of alice's laptop that fails a check of "KeyPackages" is refused, as one of the
/// device's KeyPackages and as its last-resort one: a broken signature, a lifetime of more than 60
/// days, one that does not hold yet or any more, ciphersuite `0x0003`, the `identity` of another
/// device, and another `signature_key` with the laptop's `identity`. Valid ones are taken after.
#[test]
fn key_packages_that_fail_the_checks_are_refused() {
    let stand = Stand::new(3_130_020_201);
    let (alice, bob) = (&stand.team.alice, &stand.team.bob);
    let laptop = &alice.laptop;
    let mut device = mls(laptop);
    let at = now();
    let seed = &laptop.signing.secret;
    let made = |seed, identity, suite, span| {
        foreign::key_package(&Made {
            seed,
            identity,
            suite,
            start: at,
            span,
        })
    };

    let valid = made(seed, laptop.id, PROFILE, LIFETIME);
    let mut broken = fresh(&mut device, 1).remove(0);
    // A KeyPackage ends with its signature.
    let last_byte = broken.len() - 1;
    broken[last_byte] ^= 0x01;
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (broken, "a broken signature"),
        (
            made(seed, laptop.id, PROFILE, LIFETIME + Duration::from_secs(1)),
            "a lifetime of 60 days and a second",
        ),
        (
            made(seed, laptop.id, PROFILE, Duration::from_secs(365 * DAY)),
            "a lifetime of 365 days",
        ),
        (
            // Its lifetime starts an hour after now.
            made_at(&mut device, 1, at + 2 * 3_600).remove(0),
            "a lifetime that does not hold yet",
        ),
        (
            // Its lifetime ended a day and an hour ago.
            made_at(&mut device, 1, at - 61 * DAY).remove(0),
            "a lifetime that holds no more",
        ),
        (
            made(seed, laptop.id, CHACHA, LIFETIME),
            "ciphersuite 0x0003",
        ),
        (
            made(seed, bob.laptop.id, PROFILE, LIFETIME),
            "the identity of another device",
        ),
        (
            made(&bob.laptop.signing.secret, laptop.id, PROFILE, LIFETIME),
            "another signature_key",
        ),
    ];
    for (key_package, what) in &cases {
        assert_upload_refused(
            &alice.session,
            &alice.session,
            laptop,
            (std::slice::from_ref(key_package), None),
            (0, true),
            what,
        );
        assert_upload_refused(
            &alice.session,
            &alice.session,
            laptop,
            (&[], Some(key_package)),
            (0, true),
            &format!("{what}, as the last-resort one"),
        );
    }

    let last = last_resort(&mut device);
    uploaded(
        &alice.session,
        laptop.id,
        std::slice::from_ref(&valid),
        Some(&last),
    );
    assert_eq!(
        status(&alice.session, laptop.id),
        (1, false),
        "a valid one of 60 days and a last-resort one"
    );
}
