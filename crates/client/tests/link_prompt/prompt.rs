//! What the tests share: Ada's new machine waiting for linking, and the line that names it.

use std::thread;
use std::time::{Duration, Instant};

use crate::cloud::{Cloud, WAIT};
use crate::lines::assert_no_uuid;
use crate::linking::{NewDevice, waiting};
use crate::world::{Person, World};

/// The host name of Ada's new machine, the `name` it registers.
pub const LAPTOP: &str = "ada-laptop";

/// What the line holds besides the waiting device's `name`, case aside: its kind, that it is the
/// server's unverified report, and the command that links it.
pub const SAYS: [&str; 4] = ["client", "server", "unverified", "skald link"];

/// What marks the line, case aside; no other output of the CLI says it.
pub const UNVERIFIED: &str = "unverified";

/// Ada's new machine, on the host `host`, signs in to her account at a terminal and shows its code;
/// gives it once the server has its run open.
#[track_caller]
pub fn waiting_device(world: &World, ada: &Person, host: &'static str) -> NewDevice {
    let mut new = NewDevice::start(world, ada, host);
    new.open_run(&world.cloud, ada);
    new
}

/// The lines of `shown` that hold `name`.
pub fn naming(shown: &str, name: &str) -> Vec<String> {
    (shown.lines())
        .filter(|line| line.contains(name))
        .map(str::to_owned)
        .collect()
}

/// Fails unless one line of `shown` holds `name`, and that line holds each of [`SAYS`]. Gives it.
#[track_caller]
pub fn assert_prompted(shown: &str, name: &str) -> String {
    let lines = naming(shown, name);
    let [line] = lines.as_slice() else {
        panic!("not one line naming {name:?}: {shown}");
    };
    let lower = line.to_lowercase();
    for part in SAYS {
        assert!(lower.contains(part), "no {part:?} in {line:?}: {shown}");
    }
    line.clone()
}

/// As [`assert_prompted`], for the output of a run as an agent: the line holds no ID either.
#[track_caller]
pub fn assert_agent_prompted(shown: &str, name: &str) -> String {
    let line = assert_prompted(shown, name);
    assert_no_uuid(&line);
    line
}

/// Fails if a line of `shown` holds [`UNVERIFIED`], or holds `name` and `skald link`.
#[track_caller]
pub fn assert_not_prompted(shown: &str, name: &str) {
    let lower = shown.to_lowercase();
    assert!(!lower.contains(UNVERIFIED), "a prompt: {shown}");
    let linking: Vec<String> = (naming(shown, name).into_iter())
        .filter(|line| line.to_lowercase().contains("skald link"))
        .collect();
    assert!(linking.is_empty(), "{name:?} to link: {shown}");
}

/// Waits for up to [`WAIT`] until the server holds no open run of `new`'s device for `ada`'s
/// account.
#[track_caller]
pub fn until_not_waiting(cloud: &Cloud, ada: &Person, new: &NewDevice) {
    let deadline = Instant::now() + WAIT;
    while waiting(cloud, ada, new.device) {
        assert!(Instant::now() < deadline, "the run of {} is open", new.host);
        thread::sleep(Duration::from_millis(100));
    }
}
