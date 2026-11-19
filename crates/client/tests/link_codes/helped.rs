//! `skald link` beside a running helper: the steps after its person's answers go through the
//! helper, which holds the local state's lock and keeps the run between them. In one-shot mode it
//! starts none.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use crate::cloud::WAIT;
use crate::linking::{LINK, NewDevice, YES, link_with, says, start_link, typed};
use crate::machine::{Machine, Run};
use crate::world::World;

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";
/// What `skald link` asks once it shows the new device's request.
const CONFIRM: &str = "Link it to this account?";

/// The socket of the helper of `machine`, there while one listens.
fn socket(machine: &Machine) -> PathBuf {
    machine.state_dir().join("helper").join("sock")
}

/// `skald helper` on `machine`, once it listens; killed once dropped.
fn helper(machine: &Machine) -> Run {
    let run = machine.start(&["helper"]);
    let socket = socket(machine);
    let deadline = Instant::now() + WAIT;
    while !socket.exists() {
        assert!(
            Instant::now() < deadline,
            "no helper listens after {WAIT:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
    run
}

/// With `skald helper` running on Ada's first machine, `skald link` there shows her new device's
/// request; the helper is killed before she confirms. The run went with it: `skald link` fails,
/// saying so, and the server holds no certificate of the new device. With a helper running again,
/// her new machine's fresh code links it there, and the helper keeps running.
#[test]
fn skald_link_runs_its_steps_in_the_helper() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(
        4_175_801_021,
        "ada-links-through-a-helper",
        "ada-workstation",
    );
    let mut new = NewDevice::start(&world, &ada, LAPTOP);

    let first = helper(&ada.machine);
    new.open_run(cloud, &ada);
    let mut linking = start_link(&ada, &[]);
    linking.type_line(&typed(&new.code()));
    // Once the helper has run the step that shows the request.
    linking.wait_to_show(CONFIRM);
    first.kill();
    linking.type_line(YES);
    let failed = linking.finish();
    assert!(!failed.success, "{}", failed.text);
    assert!(
        says(&failed.text, &["helper", "nothing is linked"]),
        "{}",
        failed.text
    );
    let device = new.device;
    let certified = || {
        (cloud.devices(&ada.token).into_iter()).any(|held| held.id == device && held.cert.is_some())
    };
    assert!(!certified(), "a certificate stored");

    let mut second = helper(&ada.machine);
    new.fresh_code();
    new.open_run(cloud, &ada);
    let linked = link_with(&ada, &typed(&new.code()), LAPTOP, YES);
    assert!(linked.success, "{LINK}: {}", linked.text);
    assert!(certified(), "no certificate stored");
    second.assert_running();
    let shown = new.finish();
    assert!(shown.success, "the new device failed: {}", shown.text);
}

/// `skald --one-shot link` on Ada's first machine links her new machine and leaves no helper
/// running there.
#[test]
fn skald_link_in_one_shot_mode_starts_no_helper() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(
        4_175_801_031,
        "ada-links-in-one-shot-mode",
        "ada-workstation",
    );
    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    new.open_run(cloud, &ada);
    let mut linking = start_link(&ada, &["--one-shot"]);
    linking.type_line(&typed(&new.code()));
    linking.wait_to_show(LAPTOP);
    linking.type_line(YES);
    let linked = linking.finish();
    assert!(linked.success, "{LINK}: {}", linked.text);
    assert!(!socket(&ada.machine).exists(), "a helper listens");
    let shown = new.finish();
    assert!(shown.success, "the new device failed: {}", shown.text);
}
