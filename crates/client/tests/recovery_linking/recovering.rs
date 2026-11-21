//! Linking a new device with the recovery key at the person's terminal, as the tests of linking
//! with it and of the history through it do: the person's first machine, which shows the key, and
//! the new machine's sign-in with it, the person approving its code and typing the key.

use std::thread;
use std::time::{Duration, Instant};

use skald_github_stand_in::User;
use skald_proto::DeviceId;

use crate::at_terminal::{approve, first_at_terminal, shown_key};
use crate::cloud::Cloud;
use crate::fixtures::strs;
use crate::machine::{Machine, RUN_LIMIT, SERVER, SIGN_IN};
use crate::terminal::{Shown, Terminal};
use crate::world::{Person, World};

/// The option that links with the recovery key, and those that name devices.
pub const WITH_RECOVERY_KEY: &str = "--with-recovery-key";
pub const STOLEN: &str = "--stolen";
pub const LOST: &str = "--lost";
pub const KEEP: &str = "--keep";
pub const KEEP_REVOKED: &str = "--keep-revoked";

/// What the line that asks for the recovery key holds.
pub const ASKS: &str = "recovery key";
/// What the new device says when the account has no recovery key, and over the devices limit.
pub const NO_RECOVERY_KEY: &str = "no recovery key";
pub const AT_THE_LIMIT: &str = "has as many devices as it may";

/// The request that stores a device's certificate, as its line starts.
pub const STORE_CERT: &str = "PUT /devices/";

/// The host names of Ada's first machine, of her new one and of Bob's machine.
pub const ADA_HOST: &str = "ada-workstation";
pub const NEW_HOST: &str = "ada-recovered";
pub const BOB_HOST: &str = "bob-desk";

/// How long the person waits, once asked for the key, before typing it: the run turns echo off
/// after it shows the question.
const ECHO_OFF: Duration = Duration::from_millis(300);

/// Ada's account, made by her first machine, and its recovery key in its shown form.
pub struct Ada {
    pub person: Person,
    pub key: String,
}

impl Ada {
    pub fn user(&self) -> &User {
        &self.person.user
    }
}

/// Ada, GitHub user `id` with `login`, signs up on her first machine with the host name `host` at a
/// terminal: she approves its code and presses Enter once it shows the recovery key. Gives her and
/// the key.
#[track_caller]
pub fn sign_up(world: &World, (id, login): (u64, &str), host: &'static str) -> Ada {
    let user = world.cloud.user(id, login);
    let machine = Machine::new().with_host_name(host);
    let page = world.browser.page();
    let mut run = first_at_terminal(&world.cloud, &page, &machine, &user, &[]);
    let key = shown_key(&mut run);
    run.enter();
    let signed_up = run.finish();
    assert!(signed_up.success, "{signed_up:?}");
    Ada {
        person: world.person(user, machine, host),
        key,
    }
}

/// Whether the last line `text` shows asks for the recovery key.
fn asks_for_the_key(text: &str) -> bool {
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| line.to_lowercase().contains(ASKS))
}

/// Plays the person at `run` until it exits or `stop` holds: approves, as `user`, the code device
/// authorization gives it after the `before` grants it gave earlier, if it gives one, and types
/// `key`, if any, once it asks for the recovery key.
#[track_caller]
pub fn play(
    world: &World,
    user: &User,
    run: &mut Terminal,
    (before, key): (usize, Option<&str>),
    stop: impl Fn() -> bool,
) {
    let cloud = &world.cloud;
    let started = Instant::now();
    let (mut approved, mut typed) = (false, false);
    while !run.exited() && !stop() {
        if !approved && cloud.grants().len() > before {
            approve(cloud, &world.browser.page(), user, run, before);
            approved = true;
        }
        if let Some(key) = key
            && !typed
            && asks_for_the_key(&run.shown())
        {
            thread::sleep(ECHO_OFF);
            run.type_line(key);
            typed = true;
        }
        assert!(
            started.elapsed() < RUN_LIMIT,
            "skald did not finish: {}",
            run.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
}

/// The arguments of `skald <before> sign-in --server <cloud> --with-recovery-key`, then each of `named`'s options with its device.
pub fn recovery_args(cloud: &Cloud, before: &[&str], named: &[(&str, DeviceId)]) -> Vec<String> {
    let mut args: Vec<String> = before.iter().map(|arg| (*arg).to_owned()).collect();
    args.extend([SIGN_IN, SERVER, &cloud.url, WITH_RECOVERY_KEY].map(str::to_owned));
    for (option, device) in named {
        args.extend([(*option).to_owned(), device.to_string()]);
    }
    args
}

/// Starts the sign-in of [`recovery_args`] on `machine` at a terminal; gives the run and how many
/// grants device authorization gave before it.
pub fn start_recovery(
    world: &World,
    machine: &Machine,
    before: &[&str],
    named: &[(&str, DeviceId)],
) -> (Terminal, usize) {
    let grants = world.cloud.grants().len();
    let args = recovery_args(&world.cloud, before, named);
    (Terminal::start(machine, &strs(&args)), grants)
}

/// GitHub user `user` starts linking `machine` with the recovery key at a terminal, naming
/// `named`, and plays the person, typing `key`, if any, as [`play`] does until `stop` holds; gives
/// the run.
#[track_caller]
pub fn played(
    world: &World,
    (user, key): (&User, Option<&str>),
    machine: &Machine,
    named: &[(&str, DeviceId)],
    stop: impl Fn() -> bool,
) -> Terminal {
    let (mut run, grants) = start_recovery(world, machine, &[], named);
    play(world, user, &mut run, (grants, key), stop);
    run
}

/// GitHub user `user` links `machine` with the recovery key `key`, if any, at a terminal, naming
/// `named`, and the run ends; gives what it showed.
#[track_caller]
pub fn recover(
    world: &World,
    user: &User,
    machine: &Machine,
    named: &[(&str, DeviceId)],
    key: Option<&str>,
) -> Shown {
    played(world, (user, key), machine, named, || false).finish()
}

/// As [`recover`], as Ada with her recovery key; fails unless the run succeeds. Gives what it
/// showed.
#[track_caller]
pub fn recovered(world: &World, ada: &Ada, machine: &Machine, named: &[(&str, DeviceId)]) -> Shown {
    let shown = recover(world, ada.user(), machine, named, Some(&ada.key));
    assert!(
        shown.success,
        "linking with the recovery key failed: {shown:?}"
    );
    shown
}

/// The one device of `ada`'s account that is not among `known`.
#[track_caller]
pub fn new_device(cloud: &Cloud, ada: &Person, known: &[DeviceId]) -> DeviceId {
    cloud.new_device(&ada.token, known)
}
