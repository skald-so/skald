//! A signature by a key GitHub lists for the laptop's GitHub user, among its authentication keys
//! or its signing keys: both machines show the laptop with the login GitHub gives, each checks it
//! once across runs, and the requests to GitHub carry nothing of the Skald session.

use crate::{LOGIN, Listed, assert_carry_nothing_of_the_session, pair, shows_login};

#[test]
fn a_signature_by_an_authentication_key_github_lists_shows_the_login_on_both_machines() {
    let (api, scene) = pair(Listed::Authentication);

    let looked = scene.look();
    scene.assert_passed(&looked, LOGIN);
    let requests = api.github.requests();
    assert!(!requests.is_empty(), "no request to GitHub's API");
    assert_carry_nothing_of_the_session(&requests, &scene.ada);
    assert_carry_nothing_of_the_session(&requests, scene.bob());
}

#[test]
fn a_signature_by_a_signing_key_github_lists_shows_the_login_on_both_machines() {
    let (_api, scene) = pair(Listed::Signing);

    let looked = scene.look();
    scene.assert_passed(&looked, LOGIN);
}

/// Each machine checks the laptop on its first look, and no later look asks GitHub again; the
/// login still shows.
#[test]
fn each_machine_checks_a_certificate_once_across_runs() {
    let (api, scene) = pair(Listed::Authentication);
    let laptop = scene.laptop.id;

    let own = scene.own();
    assert!(shows_login(&own, laptop, LOGIN), "{own:?}");
    let after_own = api.github.requests().len();
    assert!(after_own > 0, "Ada's machine did not ask GitHub");
    let member = scene.member();
    assert!(shows_login(&member, laptop, LOGIN), "{member:?}");
    let after_member = api.github.requests().len();
    assert!(after_member > after_own, "Bob's machine did not ask GitHub");

    for _ in 0..2 {
        let looked = scene.look();
        scene.assert_passed(&looked, LOGIN);
    }
    assert_eq!(
        api.github.requests().len(),
        after_member,
        "a later run asked GitHub again: {:?}",
        api.github.requests()
    );
}
