//! A waiting `skald recv --wait`, one-shot or through the helper, admits the newcomer on the
//! acceptance's `space-notice`, with no other command.

use std::thread;
use std::time::{Duration, Instant};

use crate::acceptances_request;
use crate::cloud::WAIT;
use crate::fixtures::await_running;
use crate::helped::{Helped, WEBSOCKET};
use crate::kit::RECV;
use crate::machine::Run;
use crate::world::ACME;
use crate::{Scene, lists};

/// The flag of `skald recv` that waits for what is to come.
const WAIT_FLAG: &str = "--wait";
/// How long a frame takes to reach a run, at most.
const PUSHED: Duration = Duration::from_millis(500);

/// Ada's `skald recv --wait`, which `wait` starts, connects and takes the waiting acceptances of
/// `acme`, none yet. Then Bob accepts the invite, which sends Ada's device a notice of `acme`.
/// With no other command, the server soon holds the revision of Ada's device that admits Bob.
#[track_caller]
fn assert_admits_on_notice(scene: &Scene, wait: impl FnOnce() -> Run) {
    let cloud = scene.cloud();
    let made = scene.invite();
    let previous = scene.latest();
    let since = cloud.relay.mark();
    let mut waiting = wait();
    await_running(&mut waiting, "connection that took the acceptances", || {
        cloud.count_sent(since, WEBSOCKET) > 0
            && cloud.count_sent(since, &acceptances_request(scene.space.id)) > 0
    });
    // A moment for the run to end its catch-up.
    thread::sleep(PUSHED);

    scene.bob_accepts(&made);

    let deadline = Instant::now() + WAIT;
    while !lists(&scene.latest(), &scene.bob.person) {
        let gone = waiting.exited();
        assert!(
            !gone && Instant::now() < deadline,
            "the waiting run did not admit Bob (exited: {gone}): {}",
            waiting.shown()
        );
        thread::sleep(Duration::from_millis(100));
    }
    assert!(
        scene
            .front
            .notices_to(scene.ada.device)
            .contains(&scene.space.id),
        "no notice of acme reached Ada's device"
    );
    waiting.kill();
    scene.assert_bob_admitted(&previous, scene.ada.device, &made);
}

/// [`assert_admits_on_notice`], one-shot.
#[test]
fn waiting_recv_admits_the_newcomer_on_the_acceptance_s_notice() {
    let scene = Scene::new(
        ACME,
        &[
            (4_855_601_001, "ada-waits-for-bob"),
            (4_855_601_002, "carol-waits-too"),
            (4_855_601_003, "bob-accepts-while-ada-waits"),
        ],
    );
    assert_admits_on_notice(&scene, || scene.ada.machine.start(&[RECV, WAIT_FLAG]));
}

/// [`assert_admits_on_notice`], the wait going through a helper, which takes the notice in.
#[test]
fn waiting_recv_through_the_helper_admits_the_newcomer_on_the_acceptance_s_notice() {
    let scene = Scene::new(
        ACME,
        &[
            (4_855_602_001, "ada-waits-helped"),
            (4_855_602_002, "carol-waits-helped"),
            (4_855_602_003, "bob-accepts-while-helped"),
        ],
    );
    // Dropped first: it kills the helper before the machine's directory goes.
    let helped = Helped::new(&scene.ada.machine);
    assert_admits_on_notice(&scene, || helped.start(&[], &[RECV, WAIT_FLAG]));
}
