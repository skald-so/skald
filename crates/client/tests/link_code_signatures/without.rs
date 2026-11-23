//! A new device without a key GitHub lists links without an SSH signature.

use crate::ssh_kit::{
    ADA_GITHUB, ADA_LOGIN, assert_says_unsigned_naming, assert_unsigned, elsewhere,
};
use crate::{Scene, link_at_terminal, names};

/// Ada's new machine has `ssh-keygen` but no key in her SSH directory; GitHub's API lists a key of
/// her GitHub user that is elsewhere. Her new machine signs in at her terminal and says it found no
/// key of her GitHub user; `skald link` on her first machine shows no GitHub login before it asks
/// to confirm; on a yes, both succeed, and the new device's certificate carries neither an SSH
/// signature nor a GitHub user.
#[test]
fn a_new_device_without_a_listed_key_links_without_a_signature() {
    let scene = Scene::start(None);
    let (laptop, key) = scene.new_machine(&scene.api.url());
    key.remove_files();
    let listed = elsewhere(&laptop, "elsewhere");
    scene.api.list((ADA_GITHUB, ADA_LOGIN), &[&listed], &[]);

    let linked = link_at_terminal(&scene, laptop);

    assert!(
        !names(&linked.asked, ADA_LOGIN),
        "skald link showed a GitHub login of a device without a signature: {}",
        linked.asked
    );
    assert_says_unsigned_naming(&linked.shown, ADA_LOGIN);
    assert_unsigned(&linked.cert);
}
