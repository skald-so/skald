//! A token ends at its expiry, at its revocation, once its device is revoked and once the latest
//! revision lists its device in no service agent entry: the server refuses it and lists it no more.

use std::thread;
use std::time::Duration;

use crate::common::device_flow::text;
use crate::tokens::{Stage, grant};
use crate::wire::now;

/// Checks that the server refuses `token` and that alice's session no longer lists `id`; `what`
/// names the case.
#[track_caller]
fn assert_ended(stage: &Stage, token: &str, id: &str, what: &str) {
    assert!(!stage.works(token), "{what}: the token still works");
    let listed = stage.listed(&stage.f.team.alice);
    assert!(
        !listed.iter().any(|listed| listed == id),
        "{what}: still listed: {listed:?}"
    );
}

/// A token of the bot whose grant expires 4 seconds after it is signed works until then and is
/// refused after it; a token of a grant for a day still works.
#[test]
fn a_token_ends_at_its_expiry() {
    let stage = Stage::new(3_535_300_001);
    let alice = &stage.f.team.alice;
    let lasting = stage.create();
    let expires = now() + 4;
    let soon = stage.create_with(&grant(
        stage.bot.id,
        &[stage.f.general],
        &alice.laptop,
        expires,
    ));
    assert!(stage.works(&soon.token), "before its expiry");

    while now() <= expires {
        thread::sleep(Duration::from_millis(200));
    }
    assert_ended(&stage, &soon.token, &soon.id, "after its expiry");
    assert!(stage.works(&lasting.token), "a token that has not expired");
    assert_eq!(stage.listed(alice), vec![lasting.id]);
}

/// alice's session revokes one of two tokens of the bot: `204`; that one is refused, the other
/// still works.
#[test]
fn a_revoked_token_ends() {
    let stage = Stage::new(3_535_300_011);
    let revoked = stage.create();
    let kept = stage.create();

    let response = stage.revoke_as(&stage.f.team.alice, &revoked.id);
    assert_eq!(response.status, 204, "{}", text(&response));
    assert_ended(&stage, &revoked.token, &revoked.id, "revoked");
    assert!(stage.works(&kept.token), "the other token");
    assert_eq!(stage.listed(&stage.f.team.alice), vec![kept.id]);
}

/// Once alice's master key revokes the bot, its token is refused.
#[test]
fn a_token_ends_once_its_device_is_revoked() {
    let stage = Stage::new(3_535_300_021);
    let created = stage.create();
    assert!(stage.works(&created.token), "before the revocation");

    let alice = &stage.f.team.alice;
    alice.revoke(&alice.by_master(stage.bot.id, &[]));
    assert_ended(&stage, &created.token, &created.id, "its device revoked");
}

/// Once a revision drops the bot's service agent entry, its token is refused; another `service`
/// device whose entry stays keeps its token.
#[test]
fn a_token_ends_once_the_latest_revision_drops_its_entry() {
    let mut stage = Stage::new(3_535_300_031);
    let other = {
        let alice = &stage.f.team.alice;
        alice.certified_by(
            stage.db(),
            &alice.laptop,
            skald_proto::DeviceKind::Service,
            "Other",
        )
    };
    stage.list_agent(&other, "other-bot");
    let dropped = stage.create();
    let other_grant = grant(
        other.id,
        &[stage.f.general],
        &stage.f.team.alice.laptop,
        now() + 3_600,
    );
    let staying = stage.create_as(&stage.f.team.alice, other.id, &other_grant, &["send"]);

    stage.drop_entry();
    assert_ended(&stage, &dropped.token, &dropped.id, "its entry dropped");
    assert!(stage.works(&staying.token), "a token whose entry stays");
}
