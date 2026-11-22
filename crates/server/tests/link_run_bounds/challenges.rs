//! A challenge for a device's session works once, for the device it was issued for, for less than a
//! minute: refused otherwise, with no session given, and a refused request uses it up.

use skald_proto::DeviceKind;

use crate::assert_problem;
use crate::by_key::{ask, body_by, challenge, token_of};
use crate::common::devices::Device;
use crate::kit::{Fresh, pair};
use crate::upload::issued_by;

/// alice's tablet, whose run her session paired, and its certificate from her laptop.
fn tablet(f: &Fresh) -> (Device, Vec<u8>) {
    let tablet = Device::new("Tablet");
    let run = f.open(&tablet);
    pair(&f.alice.session, &run.pairing);
    let cert = issued_by(
        &tablet,
        f.alice.account,
        DeviceKind::Client,
        &f.alice.laptop,
    );
    (tablet, cert)
}

#[test]
fn an_expired_challenge_is_refused() {
    let f = Fresh::start(3_175_390_111);
    let (tablet, cert) = tablet(&f);
    let nobody = f.nobody();
    let challenge = challenge(&nobody, tablet.id);
    f.flow
        .stand
        .db
        .execute("UPDATE device_challenges SET expires_at = now() - interval '1 second'");
    let public = &f.flow.stand.public;
    let body = body_by(&tablet.signing.key, &challenge, public, Some(&cert));
    assert_problem(
        &ask(&nobody, tablet.id, &body),
        400,
        "challenge-refused",
        "expired",
    );
}

#[test]
fn a_challenge_works_for_one_request_of_its_own_device() {
    let f = Fresh::start(3_175_390_121);
    let (tablet, cert) = tablet(&f);
    let nobody = f.nobody();
    let public = &f.flow.stand.public;
    let key = &tablet.signing.key;

    // Issued for another device.
    let phones = challenge(&nobody, Device::new("Phone").id);
    let body = body_by(key, &phones, public, Some(&cert));
    assert_problem(
        &ask(&nobody, tablet.id, &body),
        400,
        "challenge-refused",
        "another device's challenge",
    );

    // Used by a refused request: signed over another origin.
    let used = challenge(&nobody, tablet.id);
    let refused = body_by(key, &used, "https://elsewhere.example", Some(&cert));
    assert_problem(
        &ask(&nobody, tablet.id, &refused),
        400,
        "signature-refused",
        "another origin",
    );
    let body = body_by(key, &used, public, Some(&cert));
    assert_problem(
        &ask(&nobody, tablet.id, &body),
        400,
        "challenge-refused",
        "a challenge a refused request used",
    );

    // Used by a request that got a session: the same request again works, another does not.
    let used = challenge(&nobody, tablet.id);
    let body = body_by(key, &used, public, Some(&cert));
    assert_eq!(ask(&nobody, tablet.id, &body).status, 201);
    let other = body_by(key, &used, public, None);
    assert_problem(
        &ask(&nobody, tablet.id, &other),
        400,
        "challenge-refused",
        "another request on a used challenge",
    );
    let again = ask(&nobody, tablet.id, &body);
    assert_eq!(again.status, 200, "the same request again");
    token_of(&again);
}

#[test]
fn a_challenge_for_a_device_that_is_no_id_is_refused() {
    let f = Fresh::start(3_175_390_131);
    let response = f
        .nobody()
        .call("post", "/devices/not-an-id/challenge", None);
    assert_problem(&response, 400, "invalid-request", "no device ID");
}
