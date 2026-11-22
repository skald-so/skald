//! The device writes the acceptance before it sends it and resends it until the server takes it:
//! with the server's answer lost, or the CLI killed after writing, the next run leaves one
//! acceptance.

use crate::fixtures::retrying_for_the_timeout;
use crate::kit::received;
use crate::relay::Lose;
use crate::{
    Scene, YES, accept_at_terminal, acceptance_request, asked_for_link, questions, start,
    wait_until,
};

/// The relay loses, as `how` says, the server's answer to the first request that stores Bob's
/// acceptance; Bob's runs retry for the tests' timeout. `skald accept-invite` at a terminal, given
/// the link, confirmed; then `skald recv` on his machine succeeds. The server took the acceptance
/// more than once and holds one waiting acceptance, Bob's, and his anchors object names `acme`.
fn one_acceptance_after_a_lost_answer(how: Lose, ada: (u64, &str), bob: (u64, &str)) {
    let mut scene = Scene::new(ada, bob);
    scene.bob = retrying_for_the_timeout(scene.bob);
    let made = scene.invite();
    let cloud = scene.cloud();
    let request = acceptance_request(made.id());
    let since = cloud.relay.mark();
    cloud.relay.lose_answer(&request, how);

    // The run may fail or succeed, as it may get no answer.
    accept_at_terminal(&scene.bob.machine, &made.link, &[], (YES, &[]));
    received(&scene.bob.machine);

    assert_eq!(cloud.relay.lost(), 1, "the server never answered {request}");
    assert!(
        cloud.count_sent(since, &request) >= 2,
        "the acceptance was not sent again: {:?}",
        cloud.relay.sent_since(since)
    );
    scene.assert_bob_accepted(&made, scene.bob.handle.as_str());
    assert_eq!(
        scene.bob_anchors(),
        Some(made.revision.hash()),
        "Bob's anchors object does not name acme"
    );
}

#[test]
fn answer_dropped_by_the_proxy_leaves_one_acceptance() {
    one_acceptance_after_a_lost_answer(
        Lose::Close,
        (7_305_406_001, "ada-whose-answer-drops"),
        (7_305_406_002, "bob-loses-the-answer"),
    );
}

#[test]
fn answer_withheld_by_the_proxy_leaves_one_acceptance() {
    one_acceptance_after_a_lost_answer(
        Lose::Withhold,
        (7_305_406_101, "ada-whose-answer-is-withheld"),
        (7_305_406_102, "bob-waits-for-the-answer"),
    );
}

/// The relay holds Bob's request that stores the acceptance until a second one comes. `skald
/// accept-invite` at a terminal, given the link, confirmed, is killed once the relay holds it: the
/// server holds no acceptance. Then `skald recv` on Bob's machine succeeds: the server holds one
/// waiting acceptance, Bob's, and his anchors object names `acme`.
#[test]
fn cli_killed_after_writing_leaves_one_acceptance_at_the_next_run() {
    let scene = Scene::new(
        (7_305_406_201, "ada-whose-newcomer-crashes"),
        (7_305_406_202, "bob-crashes-while-accepting"),
    );
    let made = scene.invite();
    let cloud = scene.cloud();
    cloud.relay.hold(&acceptance_request(made.id()), 2);

    let mut terminal = start(&scene.bob.machine, &[]);
    asked_for_link(&mut terminal);
    let before = questions(&terminal.shown());
    terminal.type_line(&made.link);
    terminal.wait_for("the invite and a question to confirm it", |shown| {
        questions(&shown.replace(&made.link, "")) > before
    });
    terminal.type_line(YES);
    wait_until("the acceptance's request", || cloud.relay.held() > 0);
    terminal.kill();
    assert_eq!(
        scene.waiting(),
        Vec::new(),
        "the server took the acceptance"
    );

    received(&scene.bob.machine);

    scene.assert_bob_accepted(&made, scene.bob.handle.as_str());
    assert_eq!(
        scene.bob_anchors(),
        Some(made.revision.hash()),
        "Bob's anchors object does not name acme"
    );
}
