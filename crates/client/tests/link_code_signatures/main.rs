//! SSH signatures through a link code (`docs/protocol/membership.md`, "Link code" and "Device
//! certificate").
//!
//! - The new device's request carries its SSH signature and GitHub user when it has them; it makes
//!   them as a device that issues its own certificate does (`ssh_signing/main.rs`).
//! - The issuing device checks them against GitHub's API as it checks a certificate's
//!   (`github_checks/main.rs`) and shows the GitHub login with the new device's `name` and kind for
//!   its person to confirm. One that does not verify fails the run as a key change; one it cannot
//!   check it shows as not checked. On a yes, the certificate carries them.
//! - The new device fails the run unless the certificate holds them as it sent them, or neither.
//! - The issuing page shows what `skald link` shows (`link_pages/main.rs`).
//!
//! As in the tests of linking with a link code (`link_codes/main.rs`): Ada's first machine is
//! linked; her new machine signs in at a terminal and shows a code, and `skald link` on her first
//! machine takes it at a terminal, or, played by a test with her first device's key, answers the
//! run itself. GitHub's API is a GitHub stand-in of its own, as in the tests of signing a device
//! key (`ssh_signing/main.rs`), whose address each machine's runs take from the hidden setting; the
//! new machine has `ssh-keygen` and an SSH key in Ada's SSH directory, the first machine neither.
//!
//! What the tests chose, beside what the spec and the tests they build on name:
//! - before it asks to confirm, `skald link`, and its page, show the GitHub login of a signature
//!   that passed as a word; of one it cannot check, a line holding [`NOT_CHECKED`];
//! - a run that fails on a signature that does not verify says so on a line holding
//!   [`KEY_CHANGE`], without asking to confirm, and exits failing.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod failing;
mod granted;
#[cfg(unix)]
mod on_pages;
mod passing;
mod unchecked;
mod without;

// The modules the tests of the channel history share (`history_common/mod.rs`).
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// The helpers of the tests of signing a device key with a GitHub SSH key.
#[cfg(unix)]
#[path = "../ssh_signing/ssh_agent.rs"]
mod ssh_agent;
#[path = "../ssh_signing/ssh_kit.rs"]
mod ssh_kit;

// The helpers of the tests of linking with a link code and of its pages.
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../link_pages/pages.rs"]
mod pages;
#[path = "../link_codes/played_issuer.rs"]
mod played_issuer;

// The anchors objects of the tests of anchors, and the terminal and signing in there of the tests
// of the recovery key.
#[path = "../anchors/objects.rs"]
mod anchors;
#[path = "../recovery_key/at_terminal.rs"]
mod at_terminal;
#[path = "../recovery_key/grants.rs"]
mod grants;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
// The helpers of linking with a link code name the grants so.
use grants as terminal_grants;

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{Cert, DeviceId};

use cloud::WAIT;
use linking::{NewDevice, YES, says, start_link, typed};
use machine::Machine;
use ssh_kit::{
    ADA_GITHUB, ADA_HOST, ADA_LOGIN, Api, Kind, SshKey, machine, new_key, sign_up, ssh_dir,
    with_ssh_keygen,
};
use terminal::Terminal;
use world::{Person, World, words};

/// The host name of Ada's new machine, its certificate's `name`.
pub const LAPTOP: &str = "ada-laptop";

/// What `skald link` asks once it shows the new device, at a terminal and on its page.
pub const CONFIRMATION: &str = pages::CONFIRMATION;
/// What the issuing device shows of a signature it cannot check, and says of one that does not
/// verify.
pub const NOT_CHECKED: &str = "not checked";
pub const KEY_CHANGE: &str = "key change";

/// A server and GitHub's API, and Ada's first machine, without `ssh-keygen`, signed up.
pub struct Scene {
    pub world: World,
    pub api: Api,
    pub ada: Person,
}

impl Scene {
    /// Her first machine's runs reach GitHub's API at `ada_api`, or at the stand-in if none.
    pub fn start(ada_api: Option<&str>) -> Self {
        let world = World::start();
        let api = Api::start();
        let reached = ada_api.map_or_else(|| api.url(), str::to_owned);
        let first = (machine(ADA_HOST, &reached), ADA_HOST);
        let (ada, _) = sign_up(&world, (ADA_GITHUB, ADA_LOGIN), first, &[]);
        Scene { world, api, ada }
    }

    /// Ada's new machine, with the host name [`LAPTOP`] and `ssh-keygen`, its runs reaching
    /// GitHub's API at `api`, with an unencrypted Ed25519 key in `id_ed25519` in her SSH
    /// directory; gives the machine and the key.
    pub fn new_machine(&self, api: &str) -> (Machine, SshKey) {
        let laptop = with_ssh_keygen(machine(LAPTOP, api));
        let key = new_key(&ssh_dir(&laptop).join("id_ed25519"), Kind::Ed25519, "");
        (laptop, key)
    }

    /// `laptop` signs in to Ada's account at a terminal, as [`NewDevice::start_on`] does.
    pub fn new_device(&self, laptop: Machine) -> NewDevice {
        NewDevice::start_on(&self.world, &self.ada, laptop, LAPTOP)
    }
}

/// What `skald link` showed so far, at a terminal.
pub struct Asking {
    pub run: Terminal,
    pub shown: String,
    /// Whether it asked to confirm the new device; otherwise it exited without asking.
    pub asked: bool,
}

/// `laptop` signs in to Ada's account at a terminal and shows its code; `skald link` on her first
/// machine at a terminal, the person typing that code, runs until it asks to confirm the new
/// device, or exits without asking. Gives the new device and the run of `skald link`.
#[track_caller]
pub fn link_until_asked(scene: &Scene, laptop: Machine) -> (NewDevice, Asking) {
    let mut new = scene.new_device(laptop);
    new.open_run(&scene.world.cloud, &scene.ada);
    let mut run = start_link(&scene.ada, &[]);
    run.type_line(&typed(&new.code()));
    let deadline = Instant::now() + WAIT;
    loop {
        let shown = run.shown();
        let asked = says(&shown, &[CONFIRMATION]);
        if asked || run.exited() {
            return (new, Asking { run, shown, asked });
        }
        assert!(
            Instant::now() < deadline,
            "skald link neither asked nor exited: {shown}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// A new device linked at the person's terminal on both sides.
pub struct Linked {
    pub device: DeviceId,
    /// What `skald link` showed when it asked to confirm the new device.
    pub asked: String,
    /// What the new device's `skald sign-in` showed.
    pub shown: String,
    /// The new device's certificate, as the server holds it.
    pub cert: Cert,
}

/// `laptop` signs in to Ada's account at a terminal and shows its code, which `skald link` on her
/// first machine at a terminal takes. Fails unless `skald link` shows the new device's name and
/// kind and asks to confirm it; on a yes, unless both runs succeed and the server holds the new
/// device's certificate.
#[track_caller]
pub fn link_at_terminal(scene: &Scene, laptop: Machine) -> Linked {
    let (mut new, Asking { run, shown, asked }) = link_until_asked(scene, laptop);
    assert!(
        asked,
        "skald link exited without asking to confirm: {shown}"
    );
    let asked = shown;
    assert!(
        says(&asked, &[LAPTOP]) && says(&asked, &["client"]),
        "the new device's name and kind: {asked}"
    );
    run.type_line(YES);
    let linked = run.finish();
    assert!(linked.success, "skald link failed: {}", linked.text);
    let shown = new.finish();
    assert!(shown.success, "the new device failed: {}", shown.text);
    let cloud = &scene.world.cloud;
    let cert = (cloud.cert(&new.token, new.device)).expect("the new device's certificate");
    Linked {
        device: new.device,
        asked,
        shown: shown.text,
        cert,
    }
}

/// Whether a line of `text` holds `login` as a word.
pub fn names(text: &str, login: &str) -> bool {
    text.lines().any(|line| words(line).contains(&login))
}

/// Fails unless the new device `new` of `ada`'s account holds no certificate: it sent none, and
/// the server holds none.
#[track_caller]
pub fn assert_no_certificate(scene: &Scene, new: &NewDevice) {
    let cloud = &scene.world.cloud;
    let sent = cloud.count_sent(new.started, linking::STORE_CERT);
    assert_eq!(sent, 0, "the new device sent a certificate");
    assert!(
        cloud.cert(&new.token, new.device).is_none(),
        "the server holds a certificate of the new device"
    );
}
