//! A signature by a key GitHub does not list for the laptop's GitHub user, or by another user's:
//! each machine warns once of a key change over several runs, still trusts the laptop, as its
//! chain makes it, and shows no login for it.

use crate::{BOB_GITHUB, BOB_LOGIN, LOGIN, Listed, Scene, device_lines, list_user, pair};
use crate::{warnings_naming, words};

/// Three looks of both machines: each warns of the laptop on the first and never again, trusts it
/// on each, and shows none of `logins` on its lines.
fn warns_once(scene: &Scene, logins: &[&str]) {
    let laptop = scene.laptop.id;
    let looks: Vec<_> = (0..3).map(|_| scene.look()).collect();
    for looked in &looks {
        scene.assert_trusted(looked);
        for shown in looked.outputs() {
            for login in logins {
                assert!(
                    !(device_lines(shown, laptop).iter()).any(|line| words(line).contains(login)),
                    "the laptop is shown with {login}: {looked:?}"
                );
            }
        }
    }
    let machines = looks[0].outputs().len();
    for machine in 0..machines {
        let warned: Vec<usize> = (looks.iter())
            .map(|looked| warnings_naming(looked.outputs()[machine], laptop).len())
            .collect();
        assert_eq!(
            warned,
            [1, 0, 0],
            "warnings of the laptop per look on machine {machine}: {looks:?}"
        );
    }
}

#[test]
fn a_signature_by_a_key_github_does_not_list_warns_once_and_the_laptop_stays_valid() {
    let (_api, scene) = pair(Listed::Nowhere);

    warns_once(&scene, &[LOGIN]);
}

#[test]
fn a_signature_by_another_users_key_warns_once_and_the_laptop_stays_valid() {
    let (api, scene) = pair(Listed::Nowhere);
    let key = scene.key.as_str();
    list_user(&api.github, BOB_GITHUB, BOB_LOGIN, &[key], &[key]);

    warns_once(&scene, &[LOGIN, BOB_LOGIN]);
}
