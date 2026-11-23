//! A new device signed by a key GitHub does not list for Ada's GitHub user, as the issuing device
//! reaches GitHub: the issuing device fails the run as a key change before its person confirms, and
//! the new device gets no certificate.

use crate::ssh_kit::{ADA_GITHUB, ADA_LOGIN, Api, elsewhere};
use crate::{Asking, KEY_CHANGE, Scene, assert_no_certificate, link_until_asked, says};

/// Ada's new machine reaches a GitHub API of its own that lists the key in her SSH directory for
/// her GitHub user, so its request carries a signature by it; the GitHub API her first machine
/// reaches lists only another key for that user. `skald link` on her first machine, typed the code,
/// exits failing without asking to confirm, saying it is a key change; the new device says its run
/// failed, and neither sends nor holds a certificate.
#[test]
fn a_signature_by_a_key_github_does_not_list_fails_the_issuing_run_as_a_key_change() {
    let scene = Scene::start(None);
    let elsewhere_api = Api::start();
    let (laptop, key) = scene.new_machine(&elsewhere_api.url());
    let ada = (ADA_GITHUB, ADA_LOGIN);
    elsewhere_api.list(ada, &[&key], &[]);
    scene
        .api
        .list(ada, &[&elsewhere(&scene.ada.machine, "other")], &[]);

    let (mut new, Asking { run, shown, asked }) = link_until_asked(&scene, laptop);

    assert!(
        !asked,
        "skald link asked to confirm a device whose signature fails: {shown}"
    );
    let linked = run.finish();
    assert!(!linked.success, "skald link succeeded: {}", linked.text);
    assert!(
        says(&linked.text, &[KEY_CHANGE]),
        "skald link did not say it is a key change: {}",
        linked.text
    );
    new.wait_for_failures(1);
    assert_no_certificate(&scene, &new);
    new.kill();
}
