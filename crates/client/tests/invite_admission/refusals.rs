//! Acceptances Ada's device does not admit, warning with the handle and the check: one a proxy
//! gives signed by another key, one of an invite past its `expires`, one of an invite signed by a
//! device of a member who is no admin, one asking for a handle a member or a service agent owner
//! holds, and one of an account already a member. No revision is stored.

use skald_crypto::SigningKey;
use skald_proto::{DeviceKind, ServiceAgent, Unknown};

use crate::fixtures::{Issuer, add_device_of_kind};
use crate::kit::received;
use crate::world::{ACME, next_with, now, store};
use crate::{Made, Scene, assert_warned, resign_acceptances};

/// The owner of the service agent `ci/deploy-bot`, and its agent.
const OWNER: &str = "ci";
const BOT: &str = "deploy-bot";

/// Ada's `skald recv` stores no revision, and warns holding `handle` and one of `checks`.
#[track_caller]
fn assert_not_admitted(scene: &Scene, handle: &str, checks: &[&str]) {
    let held = scene.rosters().len();

    let ran = received(&scene.ada.machine);

    assert_eq!(
        scene.rosters().len(),
        held,
        "a revision was stored: {ran:?}"
    );
    assert_warned(&ran, handle, checks);
}

/// The scene with Dan, a member who is no admin.
fn with_dan(ids: [u64; 4], logins: [&str; 4]) -> Scene {
    let people: Vec<(u64, &str)> = ids.into_iter().zip(logins).collect();
    Scene::new(ACME, &people)
}

/// The scene without Dan.
fn without_dan(ids: [u64; 3], logins: [&str; 3]) -> Scene {
    let people: Vec<(u64, &str)> = ids.into_iter().zip(logins).collect();
    Scene::new(ACME, &people)
}

/// Bob accepts the invite; the relay gives his acceptance signed by another key than the invite's.
#[test]
fn acceptance_signed_by_another_key_is_not_admitted() {
    let scene = without_dan(
        [4_855_201_001, 4_855_201_002, 4_855_201_003],
        [
            "ada-checks-signatures",
            "carol-signs-too",
            "bob-forged-acceptance",
        ],
    );
    let made = scene.invite();
    scene.bob_accepts(&made);
    resign_acceptances(scene.cloud(), scene.space.id, SigningKey::generate());

    assert_not_admitted(
        &scene,
        scene.bob.person.handle.as_str(),
        &["signature", "key"],
    );
}

/// Bob accepts the invite; the server's database then holds it with its `expires` a minute ago,
/// signed by Ada's device, while the server keeps giving it.
#[test]
fn acceptance_of_an_invite_past_its_expiry_is_not_admitted() {
    let scene = without_dan(
        [4_855_202_001, 4_855_202_002, 4_855_202_003],
        ["ada-checks-expiry", "carol-sees-expiry", "bob-accepts-late"],
    );
    let made = scene.invite();
    scene.bob_accepts(&made);
    let expired = now() - 60;
    made.replaced_in_database(scene.cloud(), &scene.ada.key, |body| {
        body.expires = expired;
    });

    assert_not_admitted(&scene, scene.bob.person.handle.as_str(), &["expired"]);
}

/// Bob accepts the invite; the server's database then holds it signed by Dan's device, a member
/// who is no admin, as its `signer`.
#[test]
fn acceptance_of_an_invite_signed_by_a_member_who_is_no_admin_is_not_admitted() {
    let scene = with_dan(
        [4_855_203_001, 4_855_203_002, 4_855_203_003, 4_855_203_004],
        [
            "ada-checks-signers",
            "carol-the-admin",
            "bob-invited-by-dan",
            "dan-no-admin",
        ],
    );
    let made = scene.invite();
    scene.bob_accepts(&made);
    let dan = &scene.dan.as_ref().expect("Dan").person;
    made.replaced_in_database(scene.cloud(), &dan.key, |body| body.signer = dan.device);

    assert_not_admitted(&scene, scene.bob.person.handle.as_str(), &["admin"]);
}

/// Bob accepts the invite asking for Carol's handle.
#[test]
fn acceptance_asking_for_a_member_s_handle_is_not_admitted() {
    let scene = without_dan(
        [4_855_204_001, 4_855_204_002, 4_855_204_003],
        [
            "ada-checks-handles",
            "carol-holds-her-handle",
            "bob-wants-carol",
        ],
    );
    let made = scene.invite();
    let taken = scene.carol.person.handle.to_string();
    made.accepted_by(scene.cloud(), &scene.bob.person, &taken);

    assert_not_admitted(&scene, &taken, &["handle"]);
}

/// A revision signed by Ada's device adds the service agent `ci/deploy-bot`, a `service` device of
/// Ada's account; Bob accepts the invite asking for the handle `ci`.
#[test]
fn acceptance_asking_for_a_service_agent_owner_s_handle_is_not_admitted() {
    let scene = without_dan(
        [4_855_205_001, 4_855_205_002, 4_855_205_003],
        ["ada-runs-ci", "carol-sees-ci", "bob-wants-ci"],
    );
    let (cloud, ada) = (scene.cloud(), &scene.ada);
    let device = add_device_of_kind(
        cloud,
        (ada.account, DeviceKind::Service),
        "ada-deploy-bot",
        &Issuer::Device(ada),
    )
    .id;
    let with_agent = next_with(&scene.latest(), (ada.device, &ada.key), |body| {
        body.service_agents.push(ServiceAgent {
            device,
            owner: OWNER.parse().expect("a name"),
            agent: BOT.parse().expect("a name"),
            unknown: Unknown::default(),
        });
    });
    store(cloud, &ada.token, &with_agent);
    received(&ada.machine);
    let made: Made = scene.invite();
    made.accepted_by(cloud, &scene.bob.person, OWNER);

    assert_not_admitted(&scene, OWNER, &["handle"]);
}

/// Dan, a member, accepts the invite for his own account, asking for a free handle.
#[test]
fn acceptance_of_an_account_already_a_member_is_not_admitted() {
    let scene = with_dan(
        [4_855_206_001, 4_855_206_002, 4_855_206_003, 4_855_206_004],
        [
            "ada-checks-members",
            "carol-admin-again",
            "bob-stays-out",
            "dan-joins-twice",
        ],
    );
    let made = scene.invite();
    let dan = &scene.dan.as_ref().expect("Dan").person;
    made.accepted_by(scene.cloud(), dan, "dan-second-handle");

    assert_not_admitted(&scene, "dan-second-handle", &["already", "member"]);
}
