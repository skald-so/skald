//! A token reads its account's anchors objects with its device's proof, each as the bytes the
//! server got, and those of no other account; without the proof, with another device's, or once it
//! has ended, it reads none.

use crate::anchors::{ANCHORS, assert_stored};
use crate::bot::{challenge, https, token};
use crate::common::Response;
use crate::common::device_flow::{json, text};
use crate::common::devices::b64;
use crate::https::{AUTH_REQUIRED, DEVICE_REFUSED, PROOF, assert_problem, proof};
use crate::tokens::{RECEIVE, SEND, Stage, bearer, call};

/// The problem of a request whose bearer token is of no device session or service token that has
/// not ended.
const NOT_SIGNED_IN: &str = "not-signed-in";

/// `GET /account/anchors` with `token` and `proof`, if given.
fn read(stage: &Stage, token: &str, proof: Option<&str>) -> Response {
    let bearer = bearer(token);
    let mut headers = vec![("authorization", bearer.as_str())];
    if let Some(proof) = proof {
        headers.push((PROOF, proof));
    }
    call(stage.base(), "get", ANCHORS, &headers, None)
}

/// alice's laptop and eve's store anchors objects. The bot's token with the bot's proof gets
/// alice's, whole, and not eve's; without a proof it is refused with `401` and `auth-required`,
/// with the proof of alice's laptop with `device-refused`; once revoked, with `not-signed-in`.
#[test]
fn a_token_reads_its_accounts_anchors_objects_only_with_its_devices_proof() {
    let stage = Stage::new(3_545_200_301);
    let (alice, eve) = (&stage.f.team.alice, &stage.f.eve);
    let held = alice.anchors_of(&alice.laptop, 1);
    assert_stored(&alice.session, &held, "alice's laptop");
    assert_stored(
        &eve.session,
        &eve.anchors_of(&eve.laptop, 2),
        "eve's laptop",
    );
    let created = token(&stage, &[SEND, RECEIVE]);
    let https = https(&stage, &created.token);

    let response = read(&stage, &created.token, Some(&https.proof()));
    assert_eq!(response.status, 200, "{}", text(&response));
    assert_eq!(json(&response)["anchors"], serde_json::json!([b64(&held)]));

    let response = read(&stage, &created.token, None);
    assert_problem(&response, 401, Some(AUTH_REQUIRED));
    let laptop = proof(
        &https.challenge(),
        alice.laptop.id,
        &alice.laptop.signing.key,
        https.public(),
    );
    let response = read(&stage, &created.token, Some(&laptop));
    assert_problem(&response, 401, Some(DEVICE_REFUSED));

    let earlier = challenge(&stage, &created.token);
    let response = stage.revoke_as(alice, &created.id);
    assert_eq!(response.status, 204, "revoked: {}", text(&response));
    let response = read(&stage, &created.token, Some(&https.proof_on(&earlier)));
    assert_problem(&response, 401, Some(NOT_SIGNED_IN));
}
