//! GitHub unreachable at the issuing device: it shows the new device's signature as not checked,
//! and a yes issues the certificate with it.

use crate::ssh_kit::{ADA_GITHUB, ADA_LOGIN, assert_signed_by, nowhere};
use crate::{NOT_CHECKED, Scene, link_at_terminal, says};

/// Ada's first machine reaches no GitHub API; her new machine reaches one that lists the key in
/// her SSH directory for her GitHub user. `skald link` on her first machine, typed the code, shows
/// the signature as not checked before it asks to confirm; on a yes, both succeed, and the new
/// device's certificate names her GitHub user and carries a signature by that key.
#[test]
fn with_github_unreachable_the_issuing_device_shows_not_checked_and_a_yes_keeps_the_signature() {
    let scene = Scene::start(Some(&nowhere()));
    let (laptop, key) = scene.new_machine(&scene.api.url());
    scene.api.list((ADA_GITHUB, ADA_LOGIN), &[&key], &[]);

    let linked = link_at_terminal(&scene, laptop);

    assert!(
        says(&linked.asked, &[NOT_CHECKED]),
        "skald link asked to confirm without saying the signature is not checked: {}",
        linked.asked
    );
    assert_signed_by(&linked.cert, ADA_GITHUB, &key, &[]);
}
