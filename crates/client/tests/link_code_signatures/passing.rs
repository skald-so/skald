//! A new device with a key GitHub lists for Ada's GitHub user links: `skald link` shows its GitHub
//! login before its person confirms, and its certificate carries its signature, which a member of
//! Ada's space checks as passed.

use crate::ssh_kit::{
    ADA_GITHUB, ADA_LOGIN, assert_member_sees_passed, assert_says_signed, assert_signed_by,
    bob_with, elsewhere,
};
use crate::{Scene, link_at_terminal, names};

/// GitHub's API lists the key in Ada's new machine's SSH directory beside another key of her
/// GitHub user. Her new machine signs in at her terminal, says which key of which GitHub user
/// signs, and shows its code; `skald link` on her first machine, typed the code, shows the new
/// device's name, kind and her GitHub login before it asks to confirm. On a yes, both succeed; the
/// new device's certificate names her GitHub user and carries a signature by that key, not by the
/// other; Bob's device, a member of her space, shows the new device with her login and warns of
/// nothing.
#[test]
fn a_listed_key_signs_through_the_link_code_and_the_issuing_device_shows_its_login() {
    let scene = Scene::start(None);
    let (laptop, key) = scene.new_machine(&scene.api.url());
    let other = elsewhere(&laptop, "other");
    scene
        .api
        .list((ADA_GITHUB, ADA_LOGIN), &[&other, &key], &[]);

    let linked = link_at_terminal(&scene, laptop);

    assert!(
        names(&linked.asked, ADA_LOGIN),
        "skald link asked to confirm without the GitHub login {ADA_LOGIN}: {}",
        linked.asked
    );
    assert_says_signed(&linked.shown, &key, ADA_LOGIN);
    assert_eq!(
        linked.cert.body().issuer,
        Some(scene.ada.device),
        "the issuer"
    );
    assert_signed_by(&linked.cert, ADA_GITHUB, &key, &[&other]);
    let bob = bob_with(&scene.world, &scene.api.url(), &scene.ada);
    assert_member_sees_passed(&bob, linked.device, ADA_LOGIN);
}
