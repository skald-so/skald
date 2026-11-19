//! Past what the other tests of service tokens check: the grant's signer and expiry at their edges,
//! a revoked `service` device, repeated permissions, a revocation through another space or of an
//! ended token, an entry listed again, a creation while a revision drops the entry, and a use
//! recorded once a minute.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use skald_proto::DeviceKind;
use sqlx::Connection as _;

use crate::api::assert_refused;
use crate::common::device_flow::{json, text};
use crate::common::devices::Device;
use crate::fixture::exclude;
use crate::https::PROOF;
use crate::rosters::{next, service_agent};
use crate::tokens::{
    DAY, RECEIVE, SEND, Stage, bearer, call, creation, find, grant, seconds, sorted_strings,
    tokens_path,
};
use crate::wire::now;

/// The latest expiry a grant may have: 9999-12-31T23:59:59Z.
const LATEST: u64 = 253_402_300_799;

/// Checks that alice's session, proved by her laptop, gets no token of the bot with `sent`; `what`
/// names the case.
#[track_caller]
fn assert_no_token(stage: &Stage, sent: &[u8], what: &str) {
    let alice = &stage.f.team.alice;
    let proof = stage.proof(alice, &alice.laptop);
    let body = creation(stage.bot.id, sent, &[SEND]);
    assert_refused(&stage.create_response(alice, Some(&proof), &body), what);
    assert!(stage.list(alice).is_empty(), "{what}: a token stored");
}

/// Grants of the bot signed by alice's other `service` device, her device without a certificate,
/// her revoked tablet, and bob's laptop, which bob's entry excludes: each refused. One by alice's
/// laptop is taken.
#[test]
fn a_grant_by_a_device_that_is_no_admins_device_creates_no_token() {
    let mut stage = Stage::new(3_535_500_001);
    let (other, tablet) = {
        let alice = &stage.f.team.alice;
        let db = stage.db();
        (
            alice.certified_by(db, &alice.laptop, DeviceKind::Service, "Other"),
            alice.certified_by(db, &alice.laptop, DeviceKind::Client, "Tablet"),
        )
    };
    let (bob, bob_laptop) = (stage.f.team.bob.account, stage.f.team.bob.laptop.id);
    stage.f.revise(|body| exclude(body, bob, &[bob_laptop]));
    let alice = &stage.f.team.alice;
    alice.revoke(&alice.by_master(tablet.id, &[]));
    let signed_by = |signer: &Device| grant(stage.bot.id, &[stage.f.general], signer, now() + DAY);
    let t = &stage.f.team;
    for (what, signer) in [
        ("a service device", &other),
        ("no certificate", &alice.uncertified),
        ("a revoked tablet", &tablet),
        ("an excluded laptop", &t.bob.laptop),
    ] {
        assert_no_token(&stage, &signed_by(signer), what);
    }

    stage.create_with(&signed_by(&alice.laptop));
}

/// A grant of the bot that expires this second, and one that expires after the year 9999, are
/// refused; one that expires at the end of 9999 is taken with that expiry.
#[test]
fn a_grant_expiring_now_or_after_the_year_9999_creates_no_token() {
    let stage = Stage::new(3_535_500_011);
    let alice = &stage.f.team.alice;
    let until = |expires| grant(stage.bot.id, &[stage.f.general], &alice.laptop, expires);
    assert_no_token(&stage, &until(now()), "this second");
    assert_no_token(&stage, &until(LATEST + 1), "after 9999");
    assert_no_token(&stage, &until(u64::MAX), "the largest");

    let created = stage.create_with(&until(LATEST));
    let expires = seconds(stage.db(), &created.body["expires_at"]);
    assert_eq!(
        expires,
        i64::try_from(LATEST).expect("a time"),
        "{}",
        created.body
    );
}

/// Once alice's master key revokes the bot, which the latest revision still lists, it gets no
/// token.
#[test]
fn a_revoked_service_device_gets_no_token() {
    let stage = Stage::new(3_535_500_021);
    let alice = &stage.f.team.alice;
    let sent = stage.bot_grant();
    alice.revoke(&alice.by_master(stage.bot.id, &[]));
    assert_no_token(&stage, &sent, "revoked");
}

/// A permission given twice is the token's once.
#[test]
fn a_permission_given_twice_is_had_once() {
    let stage = Stage::new(3_535_500_031);
    let alice = &stage.f.team.alice;
    let created = stage.create_as(
        alice,
        stage.bot.id,
        &stage.bot_grant(),
        &[SEND, RECEIVE, SEND],
    );
    let own = json(&stage.own(&created.token));
    assert_eq!(
        sorted_strings(&own["permissions"]),
        [RECEIVE, SEND],
        "{own}"
    );
}

/// eve, globex's admin, revokes acme's token through globex: `404`, and it still works. alice's
/// first revocation of it is taken, her second and one of an ID that is no token's get `404`.
#[test]
fn a_token_is_revoked_only_through_its_space_and_once() {
    let stage = Stage::new(3_535_600_001);
    let (globex, _) = stage.other_channel();
    let created = stage.create();
    let eve = bearer(stage.f.eve.session.token());
    let path = format!("{}/{}", tokens_path(globex), created.id);
    let response = call(
        stage.base(),
        "delete",
        &path,
        &[("authorization", &eve)],
        None,
    );
    assert_eq!(response.status, 404, "{}", text(&response));
    assert!(stage.works(&created.token), "revoked through globex");

    let alice = &stage.f.team.alice;
    assert_eq!(stage.revoke_as(alice, &created.id).status, 204);
    for id in [created.id.as_str(), "not-a-token"] {
        let response = stage.revoke_as(alice, id);
        assert_eq!(response.status, 404, "{id}: {}", text(&response));
    }
}

/// A revision drops the bot's entry and the next lists it again: its token stays refused, and a new
/// one works.
#[test]
fn a_token_stays_ended_once_its_entry_is_listed_again() {
    let mut stage = Stage::new(3_535_600_011);
    let created = stage.create();
    let bot = stage.bot.id;
    stage
        .f
        .revise(|body| body.service_agents.retain(|agent| agent.device != bot));
    let entry = service_agent(&stage.bot);
    stage.f.revise(|body| body.service_agents.push(entry));

    assert!(!stage.works(&created.token), "back with its entry");
    let again = stage.create();
    assert!(stage.works(&again.token), "a new token");
    assert_eq!(stage.listed(&stage.f.team.alice), vec![again.id]);
}

/// A token's first request records its use, which its own record and the list show; a request
/// within the minute leaves it as it is.
#[test]
fn a_tokens_use_is_recorded_once_a_minute() {
    let stage = Stage::new(3_535_600_021);
    let created = stage.create();
    let first = json(&stage.own(&created.token))["last_used_at"].clone();
    assert!(first.is_string(), "no use recorded: {first}");
    let alice = &stage.f.team.alice;
    assert_eq!(find(&stage.list(alice), &created.id)["last_used_at"], first);

    let second = json(&stage.own(&created.token))["last_used_at"].clone();
    assert_eq!(second, first, "a second write within the minute");
    assert_eq!(find(&stage.list(alice), &created.id)["last_used_at"], first);
}

/// alice's session revokes the bot's token past its expiry, which the test moves back in the
/// database, and another once alice's master key revokes the bot: `404` each time.
#[test]
fn an_ended_token_is_not_revoked() {
    let stage = Stage::new(3_535_600_031);
    let (expired, other) = (stage.create(), stage.create());
    stage.db().execute(&format!(
        "UPDATE service_tokens SET expires_at = now() - interval '1 second' WHERE id = '{}'",
        expired.id
    ));
    let alice = &stage.f.team.alice;
    let response = stage.revoke_as(alice, &expired.id);
    assert_eq!(response.status, 404, "expired: {}", text(&response));

    alice.revoke(&alice.by_master(stage.bot.id, &[]));
    let response = stage.revoke_as(alice, &other.id);
    assert_eq!(
        response.status,
        404,
        "its device revoked: {}",
        text(&response)
    );
}

/// alice's session asks for the bot's token while a revision that drops the bot's entry is being
/// stored, the space's row locked as storing a revision locks it: the request waits for the
/// revision and gets no token. Once the next revision lists the bot again, still no token works.
#[test]
fn a_token_asked_for_while_a_revision_drops_its_entry_is_not_given() {
    let mut stage = Stage::new(3_535_600_041);
    let bot = stage.bot.id;
    let alice = &stage.f.team.alice;
    let mut body = next(stage.f.revisions.last().expect("a revision"), &alice.laptop);
    body.service_agents.retain(|agent| agent.device != bot);
    let revision = alice.sign(&body);
    let number = i64::try_from(body.revision).expect("a revision number");

    let proof = stage.proof(alice, &alice.laptop);
    let request = creation(bot, &stage.bot_grant(), &[SEND]);
    let (base, path) = (stage.base().to_owned(), tokens_path(stage.space()));
    let auth = bearer(alice.session.token());
    let (locked, wait) = mpsc::channel();
    let asking = thread::spawn(move || {
        wait.recv().expect("the space locked");
        let headers = [("authorization", auth.as_str()), (PROOF, proof.as_str())];
        call(&base, "post", &path, &headers, Some(&request))
    });
    let space = stage.space().to_string();
    stage
        .db()
        .query(async |conn| {
            let mut tx = conn.begin().await?;
            sqlx::query("SELECT 1 FROM spaces WHERE id = $1::uuid FOR NO KEY UPDATE")
                .bind(&space)
                .execute(&mut *tx)
                .await?;
            locked.send(()).expect("the request");
            tokio::time::sleep(Duration::from_secs(1)).await;
            sqlx::query(
                "INSERT INTO roster_revisions (space_id, revision, roster)
                 VALUES ($1::uuid, $2, $3)",
            )
            .bind(&space)
            .bind(number)
            .bind(&revision)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO roster_members (space_id, revision, account_id, admin)
                 SELECT space_id, $2, account_id, admin FROM roster_members
                 WHERE space_id = $1::uuid AND revision = $2 - 1",
            )
            .bind(&space)
            .bind(number)
            .execute(&mut *tx)
            .await?;
            tx.commit().await
        })
        .expect("store the revision");
    let response = asking.join().expect("the request");
    assert_refused(&response, "while its entry is dropped");

    stage.f.revisions.push(revision);
    let entry = service_agent(&stage.bot);
    stage.f.revise(|body| body.service_agents.push(entry));
    assert!(stage.list(&stage.f.team.alice).is_empty(), "a token given");
}
