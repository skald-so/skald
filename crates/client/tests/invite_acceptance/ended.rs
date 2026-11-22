//! When the invite ends before a revision admits the newcomer, the newcomer's device says so and
//! follows the space no more.

use crate::kit::received;
use crate::world::{ACME, run, spaces};
use crate::{Scene, follows};

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
