//! Through the pages on both sides (`link_pages/main.rs`): the issuing page shows the new device's
//! GitHub login before its person confirms, and the certificate carries the signature. Unix only,
//! as the tests' ssh-agent (`ssh_signing/ssh_agent.rs`): both commands run as an agent, so the new
//! device signs only with a key ssh-agent holds.

use crate::linking::{OWN_DEVICE, typed};
use crate::pages::{NewOnPage, Opener, YES_BUTTON, link_on_page};
use crate::ssh_agent::Agent;
use crate::ssh_kit::{
    ADA_GITHUB, ADA_LOGIN, Kind, assert_signed_by, keys_dir, machine, new_key, with_ssh_keygen,
};
use crate::{LAPTOP, Scene, names, says};

/// GitHub's API lists for Ada's GitHub user a key only her new machine's ssh-agent holds. Her new
/// machine signs in as an agent with `--browser`; she reads its code on its page. An agent's `skald
/// link --browser` on her first machine opens its page, where she enters the code and sees the new
/// device's name, kind and her GitHub login, then presses Yes. Both commands succeed, and the new
/// device's certificate names her GitHub user and carries a signature by that key.
#[test]
fn the_issuing_page_shows_the_github_login_before_the_person_confirms() {
    let scene = Scene::start(None);
    let (world, ada) = (&scene.world, &scene.ada);
    let agent = Agent::start();
    let laptop = agent.serving(with_ssh_keygen(machine(LAPTOP, &scene.api.url())));
    let held = new_key(&keys_dir(&laptop).join("held"), Kind::Ed25519, "");
    scene.api.list((ADA_GITHUB, ADA_LOGIN), &[], &[&held]);
    agent.hold(&held);
    let mut new = NewOnPage::start_on(world, ada, laptop, LAPTOP);

    let code = new.code();
    new.open_run(&world.cloud, ada);
    let opener = Opener::on(&ada.machine);
    let (mut run, paged) = link_on_page(world, ada, &opener, OWN_DEVICE);
    paged.enter(&typed(&code));
    paged.wait_for(
        &mut run,
        "the new device's name, kind and GitHub login",
        |text| says(text, &[LAPTOP]) && says(text, &["client"]) && names(text, ADA_LOGIN),
    );
    paged.press(YES_BUTTON);
    let linked = run.finish();
    assert!(linked.success, "skald link: {linked:?}");
    let signed_in = new.finish();
    assert!(signed_in.success, "the new device: {signed_in:?}");

    let cloud = &world.cloud;
    let cert = (cloud.cert(&new.token, new.device)).expect("the new device's certificate");
    assert_signed_by(&cert, ADA_GITHUB, &held, &[]);
}
