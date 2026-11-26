//! One revision admits the newcomer: when another admin's device admits it at once, and when the
//! answer to the revision is lost or the CLI is killed.

use std::thread;
use std::time::Duration;

use crate::fixtures::await_running;
use crate::kit::{RECV, received};
use crate::relay::Lose;
use crate::world::{ACME, store, store_request};
use skald_proto::Roster;

use crate::{Made, Newcomer, Scene, admitting, lists};

/// How long a test waits before a run that may sign again: a revision signed again has another
/// timestamp, and so other bytes.
const NEW_SECOND: Duration = Duration::from_millis(1100);

/// Bob accepts the invite. The relay holds the revision Ada's `skald recv` sends; meanwhile
/// Carol's device, the other admin's, stores the revision that admits Bob. Once the relay lets
/// Ada's go on, her run and the next one leave Carol's revision the one revision that admits Bob,
/// with no other after it.
#[test]
fn two_admins_admitting_the_newcomer_at_once_leave_one_revision() {
    let (scene, made, previous) = accepted(&[
        (4_855_501_001, "ada-admits-at-once"),
        (4_855_501_002, "carol-admits-at-once"),
        (4_855_501_003, "bob-admitted-once"),
    ]);
    let (cloud, ada) = (scene.cloud(), &scene.ada);
    cloud.relay.hold_next(&store_request(scene.space.id));
    let mut run = ada.machine.start(&[RECV]);
    await_running(&mut run, "held revision of Ada's device", || {
        cloud.relay.held() > 0
    });
    let carol = &scene.carol.person;
    let bob = &scene.bob.person;
    let newcomer = Newcomer {
        person: bob,
        handle: bob.handle.as_str(),
    };
    let by_carol = admitting(&previous, carol, &made, &newcomer);
    store(cloud, &carol.token, &by_carol);

    cloud.relay.let_go();
    let ran = run.finish();
    thread::sleep(NEW_SECOND);
    received(&ada.machine);

    assert!(ran.success, "{ran:?}");
    let all = scene.rosters();
    let admitted: Vec<u64> = (all.iter())
        .filter(|roster| lists(roster, bob))
        .map(|roster| roster.body().revision)
        .collect();
    assert_eq!(
        admitted,
        [by_carol.body().revision],
        "not Carol's revision alone admits Bob, the last"
    );
    assert_eq!(
        all.last().map(|roster| roster.hash()),
        Some(by_carol.hash()),
        "a revision after Carol's"
    );
}

/// Bob accepts the invite. The relay loses the server's answer to the revision Ada's `skald recv`
/// sends, as `how` says: it closes the connection, or it withholds the answer and the test kills
/// the CLI. Her next `skald recv` leaves one more revision, signed by her device, that admits Bob.
fn one_revision_after_a_lost_answer(people: &[(u64, &str)], how: Lose) {
    let (scene, made, previous) = accepted(people);
    let cloud = scene.cloud();
    let sent = store_request(scene.space.id);
    cloud.relay.lose_answer(&sent, how);

    let run = scene.ada.machine.start(&[RECV]);
    if how == Lose::Withhold {
        cloud.kill_once_lost(run, &sent);
    } else {
        // It may send again itself and finish, or fail.
        let _ = run.finish();
    }
    let lost = cloud.relay.lost();
    thread::sleep(NEW_SECOND);
    received(&scene.ada.machine);

    assert_eq!(lost, 1, "the answer to {sent} is not lost");
    scene.assert_bob_admitted(&previous, scene.ada.device, &made);
}

/// The scene in `acme` of `people`, as [`Scene::new`] takes them, in which Bob has accepted an
/// invite; the invite, and the last revision before Ada's machine runs.
fn accepted(people: &[(u64, &str)]) -> (Scene, Made, Roster) {
    let scene = Scene::new(ACME, people);
    let made = scene.invite();
    scene.bob_accepts(&made);
    let previous = scene.latest();
    (scene, made, previous)
}

#[test]
fn admitting_revision_whose_answer_the_proxy_drops_is_stored_once() {
    one_revision_after_a_lost_answer(
        &[
            (4_855_502_001, "ada-loses-the-answer"),
            (4_855_502_002, "carol-sees-one"),
            (4_855_502_003, "bob-admitted-after-a-loss"),
        ],
        Lose::Close,
    );
}

#[test]
fn admitting_revision_of_a_killed_cli_is_stored_once() {
    one_revision_after_a_lost_answer(
        &[
            (4_855_503_001, "ada-killed-mid-admission"),
            (4_855_503_002, "carol-sees-one-again"),
            (4_855_503_003, "bob-admitted-after-a-kill"),
        ],
        Lose::Withhold,
    );
}
