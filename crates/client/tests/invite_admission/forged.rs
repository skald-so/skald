//! A revision a proxy adds that lists the newcomer without an admin's signature has no effect: the
//! admin's device admits the newcomer by a revision of its own, from which the newcomer's device
//! gets into the channels.

use serde_json::{Value, json};
use skald_proto::Cbor as _;

use crate::kit::{b64, received};
use crate::world::{ACME, revisions_request, warnings};
use crate::{Newcomer, Scene, admitting};

/// Bob accepts the invite. While the server holds `acme`'s revisions up to the latest, the relay
/// gives after them a next revision that admits Bob with the invite, signed by Bob's own device,
/// which is no admin's. Ada's `skald recv` warns, and stores a revision of its own, signed by her
/// device, that admits Bob; Bob's device joins both channels from Welcomes of Ada's device.
#[test]
fn revision_a_proxy_adds_without_an_admin_s_signature_has_no_effect() {
    let mut scene = Scene::new(
        ACME,
        &[
            (4_855_401_001, "ada-ignores-forgeries"),
            (4_855_401_002, "carol-ignores-forgeries"),
            (4_855_401_003, "bob-forges-his-way-in"),
        ],
    );
    let made = scene.invite();
    scene.bob_accepts(&made);
    let previous = scene.latest();
    let bob = &scene.bob.person;
    let newcomer = Newcomer {
        person: bob,
        handle: bob.handle.as_str(),
    };
    let forged = admitting(&previous, bob, &made, &newcomer);
    let held = previous.body().revision as usize;
    let added = b64(&forged.to_cbor());
    (scene.cloud().relay).rewrite(&revisions_request(scene.space.id), move |answer| {
        let mut given: Value = serde_json::from_str(answer).unwrap_or(Value::Null);
        if let Some(list) = given["revisions"].as_array_mut()
            && list.len() == held
        {
            list.push(json!(added));
        }
        given.to_string()
    });

    let ran = received(&scene.ada.machine);

    assert!(
        !warnings(&ran).is_empty(),
        "no warning of the forged revision: {ran:?}"
    );
    scene.assert_bob_admitted(&previous, scene.ada.device, &made);
    assert_ne!(
        scene.latest().hash(),
        forged.hash(),
        "the forged revision is stored"
    );
    scene.assert_bob_welcomed();
}
