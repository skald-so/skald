//! Requests that create no token, each refused with nothing stored: a session without its device's
//! proof, an account that is no admin, a device the token may not be for, a grant that fails its
//! checks, and no permission. After each case's refusals, the request that differs only in the
//! refused part is taken.

use serde_json::Value;
use skald_proto::DeviceKind;

use crate::api::{Person, assert_refused};
use crate::common::device_flow::text;
use crate::common::devices::Device;
use crate::tokens::{DAY, RECEIVE, SEND, Stage, broken, creation, grant};
use crate::wire::now;

/// Checks that `response` refuses a token, and that alice's session still lists none; `what` names
/// the case.
#[track_caller]
fn assert_none_created(stage: &Stage, response: &crate::common::Response, what: &str) {
    assert_refused(response, what);
    assert_eq!(
        stage.list(&stage.f.team.alice),
        Vec::<Value>::new(),
        "{what}: a token stored"
    );
}

/// The request of the bot's token with `grant`, sending and receiving.
fn of_bot(stage: &Stage, grant: &[u8]) -> Value {
    creation(stage.bot.id, grant, &[SEND, RECEIVE])
}

/// alice's session asks for the bot's token with a valid grant: refused with `401` without a proof,
/// and with a proof by bob's laptop, by her own device without a certificate or by her tablet
/// after her master key revoked it. With her laptop's proof it is taken.
#[test]
fn a_session_without_its_devices_proof_creates_no_token() {
    let stage = Stage::new(3_535_200_001);
    let t = &stage.f.team;
    let alice = &t.alice;
    let tablet = alice.certified_by(stage.db(), &alice.laptop, DeviceKind::Client, "Tablet");
    alice.revoke(&alice.by_master(tablet.id, &[]));
    let body = of_bot(&stage, &stage.bot_grant());

    let mut cases: Vec<(&str, Option<String>)> = vec![("no proof", None)];
    cases.push(("bob's laptop", Some(stage.proof(alice, &t.bob.laptop))));
    cases.push((
        "no certificate",
        Some(stage.proof(alice, &alice.uncertified)),
    ));
    cases.push(("a revoked tablet", Some(stage.proof(alice, &tablet))));
    for (what, proof) in &cases {
        let response = stage.create_response(alice, proof.as_deref(), &body);
        assert_eq!(response.status, 401, "{what}: {}", text(&response));
        assert_none_created(&stage, &response, what);
    }

    let proof = stage.proof(alice, &alice.laptop);
    let response = stage.create_response(alice, Some(&proof), &body);
    assert_eq!(response.status, 201, "alice's laptop: {}", text(&response));
}

/// carol, a member, asks for a token of her `service` device, listed as `ci/carol-bot`, with a
/// grant by alice's laptop: refused. Once a revision makes her an admin, it is taken.
#[test]
fn an_account_that_is_no_admin_creates_no_token() {
    let mut stage = Stage::new(3_535_200_011);
    let carol_bot = {
        let carol = &stage.f.team.carol;
        carol.certified_by(
            stage.db(),
            &carol.laptop,
            DeviceKind::Service,
            "Carol's bot",
        )
    };
    stage.list_agent(&carol_bot, "carol-bot");
    let alice = &stage.f.team.alice;
    let sent = grant(carol_bot.id, &[stage.f.general], &alice.laptop, now() + DAY);
    let carol = &stage.f.team.carol;
    let proof = stage.proof(carol, &carol.laptop);
    let body = creation(carol_bot.id, &sent, &[SEND]);
    let response = stage.create_response(carol, Some(&proof), &body);
    assert_none_created(&stage, &response, "carol, a member");

    let account = stage.f.team.carol.account;
    stage.make_admin(account);
    stage.create_as(&stage.f.team.carol, carol_bot.id, &sent, &[SEND]);
}

/// Each with a grant that names the device and is signed by an admin's laptop, alice's session asks
/// for a token of: bob's `service` device, listed as `ci/bob-bot`; her tablet, a `client` device;
/// and her second `service` device, which no entry lists. Each is refused. bob's session gets the
/// first; the second device's token is taken once a revision lists it.
#[test]
fn a_token_is_only_for_a_service_device_of_the_account_that_an_entry_lists() {
    let mut stage = Stage::new(3_535_200_021);
    let (bob_bot, tablet, unlisted) = {
        let (alice, bob) = (&stage.f.team.alice, &stage.f.team.bob);
        let db = stage.db();
        (
            bob.certified_by(db, &bob.laptop, DeviceKind::Service, "Bob's bot"),
            alice.certified_by(db, &alice.laptop, DeviceKind::Client, "Tablet"),
            alice.certified_by(db, &alice.laptop, DeviceKind::Service, "Unlisted"),
        )
    };
    stage.list_agent(&bob_bot, "bob-bot");
    let t = &stage.f.team;
    let for_device = |device: &Device| {
        let signer = if device.id == tablet.id {
            &t.bob.laptop
        } else {
            &t.alice.laptop
        };
        grant(device.id, &[stage.f.general], signer, now() + DAY)
    };
    for (what, device) in [
        ("bob's bot", &bob_bot),
        ("a client device", &tablet),
        ("unlisted", &unlisted),
    ] {
        let proof = stage.proof(&t.alice, &t.alice.laptop);
        let body = creation(device.id, &for_device(device), &[SEND]);
        let response = stage.create_response(&t.alice, Some(&proof), &body);
        assert_none_created(&stage, &response, what);
    }

    stage.create_as(&t.bob, bob_bot.id, &for_device(&bob_bot), &[SEND]);
    let unlisted_grant = for_device(&unlisted);
    stage.list_agent(&unlisted, "unlisted-bot");
    stage.create_as(&stage.f.team.alice, unlisted.id, &unlisted_grant, &[SEND]);
}

/// alice's session, proved by her laptop, asks for the bot's token with a grant that: names her
/// unlisted `service` device; lists a channel of eve's space; is signed by carol's laptop, a
/// member's; has a broken signature; or expired a minute ago. Each is refused, and the grant that
/// fails none of these checks is taken.
#[test]
fn a_grant_that_fails_its_checks_creates_no_token() {
    let stage = Stage::new(3_535_200_031);
    let t = &stage.f.team;
    let (alice, carol) = (&t.alice, &t.carol);
    let other = alice.certified_by(stage.db(), &alice.laptop, DeviceKind::Service, "Other");
    let (_, elsewhere) = stage.other_channel();
    let general = stage.f.general;
    let later = now() + DAY;
    let cases = [
        (
            "another device",
            grant(other.id, &[general], &alice.laptop, later),
        ),
        (
            "another space's channel",
            grant(stage.bot.id, &[general, elsewhere], &alice.laptop, later),
        ),
        (
            "a member's laptop",
            grant(stage.bot.id, &[general], &carol.laptop, later),
        ),
        (
            "a broken signature",
            broken(&grant(stage.bot.id, &[general], &alice.laptop, later)),
        ),
        (
            "expired",
            grant(stage.bot.id, &[general], &alice.laptop, now() - 60),
        ),
    ];
    for (what, sent) in &cases {
        let proof = stage.proof(alice, &alice.laptop);
        let response = stage.create_response(alice, Some(&proof), &of_bot(&stage, sent));
        assert_none_created(&stage, &response, what);
    }

    stage.create_with(&grant(stage.bot.id, &[general], &t.bob.laptop, later));
}

/// alice's session, proved by her laptop, asks for the bot's token with a valid grant and no
/// permission: refused; with one, taken.
#[test]
fn a_token_without_a_permission_is_refused() {
    let stage = Stage::new(3_535_200_041);
    let alice: &Person = &stage.f.team.alice;
    let sent = stage.bot_grant();
    let proof = stage.proof(alice, &alice.laptop);
    let response = stage.create_response(alice, Some(&proof), &creation(stage.bot.id, &sent, &[]));
    assert_none_created(&stage, &response, "no permission");

    stage.create_as(alice, stage.bot.id, &sent, &[RECEIVE]);
}
