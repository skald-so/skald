//! Where a run starts its space's roster: from the latest revision that the anchors objects of its
//! account's valid `client` devices name, past its key secret's anchor, when those revisions are on
//! one chain from that anchor; otherwise from the anchor, saying so.

use skald_proto::Roster;

use crate::fixtures::Keyed;
use crate::kit::{latest, received};
use crate::member::texts_of;
use crate::revocations::revoked_by;
use crate::scene::{
    Lost, Scene, add_anchors, anchors_object, assert_stops_at, assert_takes, names_revision,
    other_device, store_anchors,
};
use crate::world::{next, serve_revisions, warnings};

/// What the agent says; nowhere else.
const AGENT_SAYS: &str = "stateless bot test: deploy-bot reports from its start";

/// Bob's device revokes the laptop; then the agent's run sends, warning that the laptop's revision
/// has no effect, naming it and the laptop, and Bob's device opens what it sent.
#[track_caller]
fn stops_at_the_laptop_s(scene: &Scene, lost: &Lost) {
    lost.revoke(scene);
    let sent = scene.agent.sent(scene.channel(), AGENT_SAYS);
    assert_stops_at(&sent, lost.number(), lost.laptop.id);
    bob_reads_the_agent(scene);
}

/// Bob's device revokes the laptop; then the agent's run sends without a warning of the laptop's
/// revision, and Bob's device opens what it sent.
#[track_caller]
fn takes_the_laptop_s(scene: &Scene, lost: &Lost) {
    lost.revoke(scene);
    let sent = scene.agent.sent(scene.channel(), AGENT_SAYS);
    assert_takes(&sent, lost.number());
    bob_reads_the_agent(scene);
}

/// A second `client` device of Ada's account, `ada-old-laptop`, stores an anchors object that
/// names `roster`. Gives the device.
fn old_laptop_anchors(scene: &Scene, roster: &Roster) -> Keyed {
    let (cloud, ada) = (scene.cloud(), scene.ada());
    let old = other_device(cloud, ada, "ada-old-laptop");
    let object = anchors_object((ada.account, old.id, &old.signing), scene.space(), roster);
    store_anchors(cloud, &ada.token, &object);
    old
}

/// From now on, the relay gives the space's revisions with `added` after them.
fn serve_with(scene: &Scene, added: Roster) {
    let mut given = scene.revisions();
    given.push(added);
    serve_revisions(
        scene.cloud(),
        scene.space(),
        &given.iter().collect::<Vec<&Roster>>(),
    );
}

/// Fails unless Bob's device opens one envelope of the agent, which says [`AGENT_SAYS`].
#[track_caller]
fn bob_reads_the_agent(scene: &Scene) {
    let opened = scene.bob_opens();
    let texts: Vec<Vec<String>> = opened.iter().map(texts_of).collect();
    assert_eq!(
        texts,
        [[AGENT_SAYS]],
        "what Bob's device opens of the agent"
    );
}

/// Ada's device makes Bob an admin and Bob's lost laptop signs the next revision, both past the key
/// secret's anchor. Ada's machine accepts them, so her device's anchors object names the second;
/// then Bob's device revokes the laptop. The agent's run sends without a warning of the laptop's
/// revision, and Bob's device opens what it sent.
#[test]
fn a_run_starts_from_the_revision_its_accounts_devices_anchored_and_sends() {
    let scene = Scene::new(
        [3_135_770_001, 3_135_770_002],
        ["ada-stateless-start-standin", "bob-stateless-start-standin"],
    );
    let lost = Lost::set_up(&scene);
    received(&scene.ada().machine);

    takes_the_laptop_s(&scene, &lost);
}

/// As above, but Ada's machine does not run until after the laptop's revocation, so no anchors
/// object names the laptop's revision: the agent's run warns that the revision has no effect,
/// naming it and the laptop, keeps the revision before it and sends, and Bob's device opens what
/// it sent.
#[test]
fn a_revision_no_device_of_the_account_accepted_before_its_signer_s_revocation_stops_the_run() {
    let scene = Scene::new(
        [3_135_771_001, 3_135_771_002],
        [
            "ada-stateless-offline-standin",
            "bob-stateless-offline-standin",
        ],
    );
    let lost = Lost::set_up(&scene);

    stops_at_the_laptop_s(&scene, &lost);
}

/// A second `client` device of Ada's account stores an anchors object that names the laptop's
/// revision, and is revoked; Ada's machine does not run. The object does not move the start: the
/// agent's run warns that the laptop's revision has no effect, and sends what Bob's device opens.
#[test]
fn an_anchors_object_of_a_revoked_device_does_not_move_the_start() {
    let scene = Scene::new(
        [3_135_772_001, 3_135_772_002],
        [
            "ada-stateless-revoked-standin",
            "bob-stateless-revoked-standin",
        ],
    );
    let lost = Lost::set_up(&scene);
    let old = old_laptop_anchors(&scene, &lost.signed);
    revoked_by(scene.cloud(), scene.ada(), old.id);

    stops_at_the_laptop_s(&scene, &lost);
}

/// The relay adds two anchors objects that name the laptop's revision to those of Ada's account:
/// one of Bob's device on his account, and one of Bob's device that claims Ada's account; Ada's
/// machine does not run. Neither moves the start: the agent's run warns that the laptop's revision
/// has no effect, and sends what Bob's device opens.
#[test]
fn an_anchors_object_of_another_account_s_device_does_not_move_the_start() {
    let scene = Scene::new(
        [3_135_773_001, 3_135_773_002],
        ["ada-stateless-other-standin", "bob-stateless-other-standin"],
    );
    let (cloud, ada, bob) = (scene.cloud(), scene.ada(), scene.bob());
    let lost = Lost::set_up(&scene);
    let objects = [bob.account, ada.account].map(|account| {
        anchors_object((account, bob.device, &bob.key), scene.space(), &lost.signed)
    });
    add_anchors(cloud, objects.to_vec());

    stops_at_the_laptop_s(&scene, &lost);
}

/// A second `client` device of Ada's account anchors the space at the revision before the key
/// secret's anchor; Ada's machine accepts the laptop's revision, so her device's anchors object
/// names it; then the laptop is revoked. The earlier anchor does not stop the start from Ada's: the
/// agent's run sends without a warning of the laptop's revision.
#[test]
fn an_anchor_below_the_key_secret_s_does_not_stop_the_start_from_a_later_one() {
    let scene = Scene::new(
        [3_135_774_001, 3_135_774_002],
        [
            "ada-stateless-earlier-standin",
            "bob-stateless-earlier-standin",
        ],
    );
    let revisions = scene.revisions();
    let lost = Lost::set_up(&scene);
    received(&scene.ada().machine);
    old_laptop_anchors(&scene, &revisions[revisions.len() - 2]);

    takes_the_laptop_s(&scene, &lost);
}

/// Ada's machine accepts the laptop's revision, so her device's anchors object names it. A second
/// `client` device of Ada's account anchors another revision of the same number, which Ada's
/// device signed on the revision before, and the relay gives both. As the anchored revisions are
/// not on one chain, the agent's run starts from the key secret's anchor and warns so, naming the
/// anchor's revision; it sends, and Bob's device opens what it sent.
#[test]
fn anchors_off_one_chain_start_the_run_from_the_key_secret_s_anchor_and_say_so() {
    let scene = Scene::new(
        [3_135_775_001, 3_135_775_002],
        ["ada-stateless-fork-standin", "bob-stateless-fork-standin"],
    );
    let ada = scene.ada();
    let anchor = scene
        .revisions()
        .last()
        .expect("a revision")
        .body()
        .revision;
    let lost = Lost::set_up(&scene);
    received(&ada.machine);
    let fork = next(&lost.promoted, (ada.device, &ada.key), |_| {});
    old_laptop_anchors(&scene, &fork);
    serve_with(&scene, fork);
    lost.revoke(&scene);

    let got = scene.agent.sent(scene.channel(), AGENT_SAYS);

    let said: Vec<String> = (warnings(&got).into_iter())
        .filter(|line| line.contains("anchor") && names_revision(line, anchor))
        .collect();
    assert!(
        !said.is_empty(),
        "no warning that the run starts from the anchor, revision {anchor}: {got:?}"
    );
    bob_reads_the_agent(&scene);
}

/// A device of Bob's account, while Bob is a member and no admin, signs the revision after the
/// latest; the relay gives it with the others, and a second `client` device of Ada's account
/// anchors it. The revision fails all the same: the agent's run warns that it has no effect,
/// naming it and its signer, keeps the revision before it and sends, and Bob's device opens what
/// it sent.
#[test]
fn an_anchored_revision_whose_signer_is_no_admin_s_device_is_refused() {
    let scene = Scene::new(
        [3_135_776_001, 3_135_776_002],
        [
            "ada-stateless-nonadmin-standin",
            "bob-stateless-nonadmin-standin",
        ],
    );
    let (cloud, ada, bob) = (scene.cloud(), scene.ada(), scene.bob());
    let laptop = other_device(cloud, bob, "bob-lost-laptop");
    let last = latest(cloud, &ada.token, scene.space());
    let forged = next(&last, (laptop.id, &laptop.signing), |_| {});
    old_laptop_anchors(&scene, &forged);
    let number = forged.body().revision;
    serve_with(&scene, forged);

    let sent = scene.agent.sent(scene.channel(), AGENT_SAYS);

    assert_stops_at(&sent, number, laptop.id);
    bob_reads_the_agent(&scene);
}
