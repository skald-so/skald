//! A run that syncs the spaces it follows, as `skald spaces` does, checks the certificates it takes of its own
//! account and of the space's members: a key change warns there, once, and a passed signature is
//! kept, so `skald devices` and `skald members` ask GitHub nothing more.

use crate::world::SPACES;
use crate::{LOGIN, Listed, Scene, pair, run_with, shows_login, warnings_naming};

/// Runs `skald spaces` on each machine of `scene` as the person; gives how many warnings of
/// the laptop each showed, Ada's machine first.
fn sync_both(scene: &Scene) -> Vec<usize> {
    [&scene.ada, scene.bob()]
        .iter()
        .map(|person| {
            let ran = run_with(&person.machine, &scene.env, &[SPACES]);
            assert!(ran.success, "{ran:?}");
            warnings_naming(&ran, scene.laptop.id).len()
        })
        .collect()
}

#[test]
fn a_run_that_syncs_a_space_warns_once_of_a_key_change() {
    let (_api, scene) = pair(Listed::Nowhere);

    assert_eq!(
        sync_both(&scene),
        [1, 1],
        "warnings of the laptop per machine"
    );
    assert_eq!(
        sync_both(&scene),
        [0, 0],
        "warnings of the laptop per machine"
    );
    let looked = scene.look();
    scene.assert_trusted(&looked);
    for shown in looked.outputs() {
        assert_eq!(
            warnings_naming(shown, scene.laptop.id).len(),
            0,
            "{looked:?}"
        );
    }
}

#[test]
fn a_run_that_syncs_a_space_keeps_a_passed_signature() {
    let (api, scene) = pair(Listed::Authentication);

    assert_eq!(
        sync_both(&scene),
        [0, 0],
        "warnings of the laptop per machine"
    );
    let asked = api.github.requests().len();
    assert!(asked > 0, "no request to GitHub's API");
    let looked = scene.look();
    for shown in looked.outputs() {
        assert!(shows_login(shown, scene.laptop.id, LOGIN), "{looked:?}");
    }
    assert_eq!(
        api.github.requests().len(),
        asked,
        "a later run asked GitHub again"
    );
}
