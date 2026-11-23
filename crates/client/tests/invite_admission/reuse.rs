//! Invites Ada's device takes for none of this space's admins' devices, and invites it used
//! before: one of another space of Ada's, one signed by a `service` device of Ada's account, and
//! one a removed member joined with, given again while the server leaves out the revisions that
//! listed it. No revision admits Bob.

use serde_json::Value;
use skald_proto::{Cbor as _, DeviceKind, NonEmpty};

use crate::fixtures::{Issuer, add_device_of_kind, hex};
use crate::kit::{create_space, received};
use crate::world::{ACME, next_with, revisions_request, store};
use crate::{Scene, assert_warned, lists};

/// Ada's `skald recv` leaves the space's last revision without Bob, and warns holding Bob's handle
/// and one of `checks`.
#[track_caller]
fn assert_bob_not_admitted(scene: &Scene, checks: &[&str]) {
    let ran = received(&scene.ada.machine);

    assert!(
        !lists(&scene.latest(), &scene.bob.person),
        "Bob is admitted: {ran:?}"
    );
    assert_warned(&ran, scene.bob.person.handle.as_str(), checks);
}

/// Bob accepts the invite; the server's database then holds it as an invite of another space of
/// Ada's, signed by Ada's device, with the same ID.
#[test]
fn acceptance_of_an_invite_of_another_space_is_not_admitted() {
    let scene = Scene::new(
        ACME,
        &[
            (4_855_207_001, "ada-has-two-spaces"),
            (4_855_207_002, "carol-in-one"),
            (4_855_207_003, "bob-invited-elsewhere"),
        ],
    );
    let other = create_space(scene.cloud(), &scene.ada, "elsewhere");
    let made = scene.invite();
    scene.bob_accepts(&made);
    made.replaced_in_database(scene.cloud(), &scene.ada.key, |body| body.space = other.id);

    assert_bob_not_admitted(&scene, &["another space"]);
}

/// Bob accepts the invite; the server's database then holds it signed by a `service` device of
/// Ada's account, an admin, as its `signer`.
#[test]
fn acceptance_of_an_invite_signed_by_an_admin_s_service_device_is_not_admitted() {
    let scene = Scene::new(
        ACME,
        &[
            (4_855_208_001, "ada-runs-a-bot"),
            (4_855_208_002, "carol-sees-the-bot"),
            (4_855_208_003, "bob-invited-by-a-bot"),
        ],
    );
    let ada = &scene.ada;
    let bot = add_device_of_kind(
        scene.cloud(),
        (ada.account, DeviceKind::Service),
        "ada-invite-bot",
        &Issuer::Device(ada),
    );
    let made = scene.invite();
    scene.bob_accepts(&made);
    made.replaced_in_database(scene.cloud(), &bot.signing, |body| body.signer = bot.id);

    assert_bob_not_admitted(&scene, &["admin"]);
}

/// Ada's device admits Bob, and a revision signed by it then removes him. The server's database
/// then holds the invite and Bob's acceptance again, and the relay gives the space's last revision
/// alone, as a compromised server would.
#[test]
fn acceptance_of_an_invite_a_removed_member_joined_with_is_not_admitted_again() {
    let scene = Scene::new(
        ACME,
        &[
            (4_855_209_001, "ada-removes-bob"),
            (4_855_209_002, "carol-sees-him-go"),
            (4_855_209_003, "bob-comes-back"),
        ],
    );
    let (cloud, ada, bob) = (scene.cloud(), &scene.ada, &scene.bob.person);
    let made = scene.invite();
    scene.bob_accepts(&made);
    received(&ada.machine);
    assert!(lists(&scene.latest(), bob), "Bob is not admitted");
    let removed = next_with(&scene.latest(), (ada.device, &ada.key), |body| {
        let mut members = body.members.clone().into_vec();
        members.retain(|it| it.account != bob.account);
        body.members = NonEmpty::from_vec(members).expect("a member");
        for channel in &mut body.channels {
            channel.members.retain(|it| it.account != bob.account);
        }
    });
    store(cloud, &ada.token, &removed);
    received(&ada.machine);
    let acceptance = made.acceptance(bob, bob.handle.as_str(), &made.secret.invite_key());
    cloud.execute(&format!(
        "INSERT INTO invites (id, space_id, invite, expires_at)
         VALUES ('{}'::uuid, '{}'::uuid, '\\x{}'::bytea, now() + interval '1 day')",
        made.id(),
        scene.space.id,
        hex(&made.invite.to_cbor()),
    ));
    cloud.execute(&format!(
        "INSERT INTO acceptances (invite_id, account_id, acceptance)
         VALUES ('{}'::uuid, '{}'::uuid, '\\x{}'::bytea)",
        made.id(),
        bob.account,
        hex(&acceptance.to_cbor()),
    ));
    cloud
        .relay
        .rewrite(&revisions_request(scene.space.id), |answer| {
            let mut held: Value = serde_json::from_str(answer).expect("the server's revisions");
            let list = held["revisions"]
                .as_array_mut()
                .expect("a list of revisions");
            list.drain(..list.len() - 1);
            held.to_string()
        });

    assert_bob_not_admitted(&scene, &["revision"]);
}
