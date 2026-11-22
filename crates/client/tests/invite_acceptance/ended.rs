//! When the invite ends before a revision admits the newcomer, the newcomer's device says so and
//! follows the space no more.

use crate::kit::received;
use crate::world::{ACME, run, spaces};
use crate::{Made, Scene, YES, accept_at_terminal, acceptance_request, follows};

/// Deletes `made`'s invite from the server's database: Bob's next `skald recv` says the invite
/// ended, and `skald spaces` on his machine does not list `acme`.
#[track_caller]
fn ends_the_following(scene: &Scene, made: &Made) {
    let cloud = scene.cloud();
    cloud.execute(&format!("DELETE FROM invites WHERE id = '{}'", made.id()));

    let told = run(&scene.bob.machine, &["recv"]);

    assert!(
        told.shown().to_lowercase().contains("ended"),
        "it does not say the invite ended: {told:?}"
    );
    let listed = spaces(&scene.bob.machine);
    assert!(
        !follows(&listed, ACME),
        "acme is still followed: {listed:?}"
    );
}

/// Bob accepts the invite at a terminal; then the invite is deleted from the server's database,
/// and with it his acceptance, before any revision admits him. Bob's next `skald recv` says the
/// invite ended; `skald spaces` on his machine does not list `acme`, and a later `skald recv` sends
/// no request that names the invite or `acme`.
#[test]
fn invite_deleted_before_a_revision_admits_the_newcomer_ends_the_following() {
    let scene = Scene::new(
        (7_305_407_001, "ada-whose-invite-is-deleted"),
        (7_305_407_002, "bob-left-waiting"),
    );
    let made = scene.bob_accepts();
    let cloud = scene.cloud();

    ends_the_following(&scene, &made);
    let since = cloud.relay.mark();
    received(&scene.bob.machine);
    let ids = [made.id().to_string(), scene.space.id.to_string()];
    let naming: Vec<String> = (cloud.relay.sent_since(since).into_iter())
        .map(|sent| sent.head.lines().next().unwrap_or_default().to_owned())
        .filter(|line| ids.iter().any(|id| line.contains(id.as_str())))
        .collect();
    assert_eq!(
        naming,
        Vec::<String>::new(),
        "requests about acme or the invite"
    );
}

/// The relay answers Bob's acceptance itself with each refusal the server gives one, `400`,
/// `403`, `404` and `409`, the server never getting it. `skald accept-invite`, given the link,
/// confirmed, fails each time saying `refused`; `skald spaces` on Bob's machine does not list
/// `acme`, and the server holds no acceptance.
#[test]
fn acceptance_the_server_refuses_leaves_the_space_unfollowed() {
    let scene = Scene::new(
        (7_305_412_001, "ada-whose-server-refuses-bob"),
        (7_305_412_002, "bob-refused-by-the-server"),
    );
    let made = scene.invite();
    let relay = &scene.cloud().relay;
    for status in [
        "400 Bad Request",
        "403 Forbidden",
        "404 Not Found",
        "409 Conflict",
    ] {
        let code = &status[..3];
        let problem = format!(
            r#"{{"type":"about:blank","title":"{status}","status":{code},"detail":"No."}}"#
        );
        relay.refuse_with(&acceptance_request(made.id()), status, &problem);

        let refused = accept_at_terminal(&scene.bob.machine, &made.link, &[], (YES, &[]));

        relay.stop_refusing();
        assert!(!refused.shown.success, "{status}: {}", refused.text());
        assert!(
            refused.text().contains("refused"),
            "{status}: it does not say the server refused: {}",
            refused.text()
        );
        let listed = spaces(&scene.bob.machine);
        assert!(!follows(&listed, ACME), "{status}: {listed:?}");
        assert!(
            scene.waiting().is_empty(),
            "{status}: the server holds an acceptance"
        );
    }
}

/// The relay answers Bob's acceptance itself with `503`, the server never getting it: `skald
/// accept-invite`, given the link, confirmed, fails, saying `skald recv` sends it again. The relay
/// then passes requests on, and the invite is deleted from the server's database. Bob's next
/// `skald recv` says the invite ended, and `skald spaces` on his machine does not list `acme`.
#[test]
fn invite_ending_before_the_server_took_the_acceptance_ends_the_following() {
    let scene = Scene::new(
        (7_305_412_101, "ada-whose-server-was-down"),
        (7_305_412_102, "bob-whose-acceptance-waits"),
    );
    let made = scene.invite();
    let cloud = scene.cloud();
    cloud
        .relay
        .refuse(&acceptance_request(made.id()), "503 Service Unavailable");
    let waiting = accept_at_terminal(&scene.bob.machine, &made.link, &[], (YES, &[]));
    cloud.relay.stop_refusing();
    assert!(!waiting.shown.success, "{}", waiting.text());
    assert!(waiting.text().contains("skald recv"), "{}", waiting.text());

    ends_the_following(&scene, &made);
}
