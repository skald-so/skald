//! A key change the issuing device finds in the new device's SSH signature waits in its local
//! state until its person sees it, as every key change does (`membership.md`, "Device checks").

use crate::machine::DEVICES;
use crate::ssh_kit::{ADA_GITHUB, ADA_LOGIN, Api};
use crate::terminal::Terminal;
use crate::world::run;
use crate::{Asking, Scene, link_until_asked};

/// The command that shows the key changes that wait, and what its line and its question hold.
const KEY_CHANGES: &str = "key-changes";
const SHOWN_BY: &str = "skald key-changes";
const ENTER: &str = "Enter";

/// Ada's new machine signs its request with a key a GitHub API of its own lists for her GitHub
/// user; the GitHub API her first machine reaches has no such user. `skald link` on her first
/// machine fails the run. A later `skald devices` there says a key change waits, and her `skald
/// key-changes` at a terminal shows it, naming the new device.
#[test]
fn a_key_change_in_the_new_device_s_signature_waits_until_its_person_sees_it() {
    let scene = Scene::start(None);
    let own_api = Api::start();
    let (laptop, key) = scene.new_machine(&own_api.url());
    own_api.list((ADA_GITHUB, ADA_LOGIN), &[&key], &[]);

    let (mut new, Asking { run: link, .. }) = link_until_asked(&scene, laptop);
    let linked = link.finish();
    assert!(!linked.success, "skald link succeeded: {}", linked.text);
    new.kill();

    let machine = &scene.ada.machine;
    let later = run(machine, &[DEVICES]);
    assert!(later.success, "skald devices: {later:?}");
    assert!(
        later.shown().lines().any(|line| line.contains(SHOWN_BY)),
        "no key change waits: {}",
        later.shown()
    );
    let mut terminal = Terminal::start(machine, &[KEY_CHANGES]);
    let shown = terminal.wait_to_show(ENTER);
    assert!(
        shown.contains(&new.device.to_string()),
        "the key change does not name the new device {}: {shown}",
        new.device
    );
    terminal.enter();
    assert!(terminal.finish().success);
}
