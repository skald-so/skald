//! What the tests share: the person's account with a space shared with Bob, `skald delete-account`
//! at a terminal driven as a person drives it, and what the server and a machine keep of an account
//! afterwards.

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_github_stand_in::User;
use skald_proto::SpaceId;

use crate::browser::{APPROVE, DENY};
use crate::cloud::{Cloud, Grant, WAIT};
use crate::keys;
use crate::linking::lines_holding;
use crate::machine::{Machine, RUN_LIMIT};
use crate::roles::{RoleCommand, role_set};
use crate::terminal::{Shown, Terminal};
use crate::world::{ACME, Person, World, create_with_member, members, run};

/// The command that deletes the account.
pub const DELETE_ACCOUNT: &str = "delete-account";
/// What the command's question for the username holds.
pub const USERNAME: &str = "username";
/// What it says of the spaces a deletion deletes and of the account once deleted, and what the
/// account's other devices say of it.
pub const DELETED: &str = "deleted";
/// What the line of the account's personal space holds beside its name.
pub const PERSONAL: &str = "personal";
/// What it says goes 30 days after the deletion.
pub const LATER: &str = "30 days";
/// The command a blocking space's line names.
pub const HAND_OVER: &str = "hand-over-admin";
/// What it says from a pipe.
pub const TERMINAL: &str = "terminal";

/// The requests that delete the account and that start a device authorization, as their lines
/// start.
pub const DELETE_REQUEST: &str = "DELETE /account ";
pub const DEVICE_AUTHORIZATION: &str = "POST /auth/device ";

/// How much older than the server's setting of 300 seconds a sign-in is made.
const OLDER: &str = "10 minutes";

/// What the person does with each device authorization a run shows.
#[derive(Clone, Copy)]
pub enum AtGrant<'a> {
    /// Approves it on the verification page, signed in as this GitHub user.
    Approve(&'a User),
    /// Denies it there, signed in as this GitHub user.
    Deny(&'a User),
    /// Lets it expire: the test moves its expiry to the past in the database.
    Expire,
    /// None is expected: the test fails if the run shows one.
    Refuse,
}

/// A run of `skald delete-account` at a terminal, once it exited.
#[derive(Debug)]
pub struct Deletion {
    pub shown: Shown,
    /// How many device authorizations it showed.
    pub grants: usize,
    /// How many times it asked for the username.
    pub asked: usize,
}

/// Runs `skald delete-account` on `machine` at a terminal as a person does: each time it asks for
/// the username, the person types `typed`, three times at most, then ends the input; each device
/// authorization it shows the person treats as `at_grant` says. Gives the run once it exits.
pub fn delete_at_terminal(
    world: &World,
    machine: &Machine,
    typed: &str,
    at_grant: AtGrant,
) -> Deletion {
    let before = world.cloud.grants().len();
    let mut run = Terminal::start(machine, &[DELETE_ACCOUNT]);
    let (asked, grants) = drive(world, &mut run, (typed, at_grant, before), Terminal::exited);
    Deletion {
        shown: run.finish(),
        grants,
        asked,
    }
}

/// Deletes `person`'s account by `skald delete-account` at a terminal on its machine, as
/// [`delete_at_terminal`] runs it, the person typing the username and approving a sign-in again if
/// asked; fails unless the run succeeds. Gives the run.
#[track_caller]
pub fn deleted_at_terminal(world: &World, person: &Person) -> Deletion {
    let deletion = delete_at_terminal(
        world,
        &person.machine,
        person.handle.as_str(),
        AtGrant::Approve(&person.user),
    );
    assert!(deletion.shown.success, "{}", deletion.shown.text);
    deletion
}

/// Plays the person at `run`, a `skald delete-account` at a terminal, as [`delete_at_terminal`]
/// does, `(typed, at_grant, before)` saying what to type, what to do with a device authorization
/// and how many the server gave before the run, until `done` holds. Gives how many times the run
/// asked for the username and how many device authorizations it showed.
pub fn drive(
    world: &World,
    run: &mut Terminal,
    (typed, at_grant, before): (&str, AtGrant, usize),
    mut done: impl FnMut(&mut Terminal) -> bool,
) -> (usize, usize) {
    let cloud = &world.cloud;
    let started = Instant::now();
    let (mut asked, mut grants) = (0, 0);
    while !done(run) {
        let shown = run.shown();
        let questions = lines_holding(&shown, USERNAME).len();
        if questions > asked {
            if asked < 3 {
                run.type_line(typed);
            } else {
                run.end_input();
            }
            asked = questions;
        }
        let given = cloud.grants();
        if let Some(grant) = given.get(before + grants)
            && shown.contains(&grant.user_code)
        {
            treat(world, grant, at_grant, &shown);
            grants += 1;
        }
        assert!(
            started.elapsed() < RUN_LIMIT,
            "skald {DELETE_ACCOUNT} ran for more than {RUN_LIMIT:?}: {shown}"
        );
        thread::sleep(Duration::from_millis(50));
    }
    (asked, grants)
}

/// The person treats `grant` as `at_grant` says; `shown` is what the run showed so far.
fn treat(world: &World, grant: &Grant, at_grant: AtGrant, shown: &str) {
    let cloud = &world.cloud;
    match at_grant {
        AtGrant::Approve(user) => {
            cloud.decide(&world.browser.page(), user, grant, APPROVE);
        }
        AtGrant::Deny(user) => {
            cloud.decide(&world.browser.page(), user, grant, DENY);
        }
        AtGrant::Expire => cloud.execute(
            "UPDATE device_codes SET expires_at = now() - interval '1 second'
             WHERE state = 'pending'",
        ),
        AtGrant::Refuse => panic!("it asked to sign in again: {shown}"),
    }
}

/// Moves the sign-in of the session `token` back past the server's setting.
pub fn make_sign_in_old(cloud: &Cloud, token: &str) {
    cloud.execute(&format!(
        "UPDATE sessions SET created_at = created_at - interval '{OLDER}'
         WHERE token_hash = sha256(convert_to('{token}', 'UTF8'))"
    ));
}

/// `ada`'s machine creates `acme`, and the revision its device signs adds `bob` as a member, which
/// its machine accepts: Ada is the space's only admin. Gives the space.
pub fn shared_with(cloud: &Cloud, ada: &Person, bob: &Person) -> SpaceId {
    let third = create_with_member(cloud, ada, ACME, bob);
    members(&ada.machine, ACME);
    third.body().space
}

/// `ada`'s machine hands its account's admin rights in `acme` to `bob`.
pub fn hand_over(ada: &Person, bob: &Person) {
    role_set(&ada.machine, RoleCommand::HandOver, ACME, bob);
}

/// Whether `viewer`'s session gets `member` marked deleted among the members of `space`.
pub fn marked_deleted(cloud: &Cloud, viewer: &Person, space: SpaceId, member: &Person) -> bool {
    let path = format!("/spaces/{space}/members");
    let (status, body) = cloud.get_as(&path, &viewer.token);
    assert_eq!(status, 200, "GET {path}: {body}");
    let listed = body["members"]
        .as_array()
        .unwrap_or_else(|| panic!("GET {path}: {body}"))
        .iter()
        .find(|entry| entry["handle"] == member.handle.as_str())
        .unwrap_or_else(|| panic!("{} is not listed: {body}", member.handle))
        .clone();
    listed["deleted"] == Value::Bool(true)
}

/// Fails unless `person`'s account is deleted: its session is refused.
#[track_caller]
pub fn assert_deleted(cloud: &Cloud, person: &Person) {
    let (status, body) = cloud.session_of(&person.token);
    assert_ne!(
        status, 200,
        "{}'s session still works: {body}",
        person.handle
    );
}

/// Fails unless `person`'s account is kept: its session works, and its machine still runs as its
/// device.
#[track_caller]
pub fn assert_kept(cloud: &Cloud, person: &Person) {
    let (status, body) = cloud.session_of(&person.token);
    assert_eq!(status, 200, "{}'s session: {body}", person.handle);
    let listed = run(&person.machine, &["devices"]);
    assert!(listed.success, "its machine: {listed:?}");
}

/// Fails unless no request since `since` deleted the account or started a device authorization.
#[track_caller]
pub fn assert_sent_nothing(cloud: &Cloud, since: Instant, what: &str) {
    assert_eq!(
        cloud.count_sent(since, DELETE_REQUEST),
        0,
        "{what}: a request to delete the account"
    );
    assert_eq!(
        cloud.count_sent(since, DEVICE_AUTHORIZATION),
        0,
        "{what}: a device authorization"
    );
}

/// The device's certificate and the keys `person`'s machine holds: what its machine must not keep
/// once the account is deleted.
pub struct Held {
    pub signing: [u8; 32],
    pub hpke: [u8; 32],
}

impl Held {
    /// What `person`'s machine holds now, its certificate as the server holds it.
    pub fn of(cloud: &Cloud, person: &Person) -> Self {
        let cert = (cloud.cert(&person.token, person.device)).expect("the device's certificate");
        Held {
            signing: cert.body().signing,
            hpke: cert.body().hpke,
        }
    }
}

/// Fails unless the state directory of `person`'s machine keeps nothing of its account: no SQLite
/// database, and no file holding a session token the server gave, the device's signing or HPKE
/// private key, the account's or the device's ID, or the account's username.
#[track_caller]
pub fn assert_keeps_nothing(cloud: &Cloud, person: &Person, held: &Held) {
    let machine = &person.machine;
    let state = machine.state_dir();
    let databases = machine.sqlite_files();
    assert!(
        databases.is_empty(),
        "the local state is kept: {databases:?}"
    );
    let tokens: Vec<_> = (cloud.token_files(machine).into_iter())
        .filter(|(file, _)| file.starts_with(&state))
        .collect();
    assert!(tokens.is_empty(), "a session token is kept: {tokens:?}");
    let (account, device) = (person.account.to_string(), person.device.to_string());
    for file in machine.files_holding(&account) {
        assert!(!file.starts_with(&state), "{file:?} holds the account ID");
    }
    for file in machine.files_holding(&device) {
        assert!(!file.starts_with(&state), "{file:?} holds the device ID");
    }
    for file in machine.files_holding(person.handle.as_str()) {
        assert!(!file.starts_with(&state), "{file:?} holds the username");
    }
    for file in machine.files() {
        if !file.starts_with(&state) {
            continue;
        }
        let bytes = fs::read(&file).unwrap_or_default();
        assert!(
            keys::private_key(&bytes, &held.signing, keys::ed25519).is_none(),
            "{file:?} holds the device's signing key"
        );
        assert!(
            keys::private_key(&bytes, &held.hpke, keys::x25519).is_none(),
            "{file:?} holds the device's HPKE key"
        );
        assert!(
            !keys::holds(&bytes, person.account.as_bytes()),
            "{file:?} holds the account ID's bytes"
        );
        assert!(
            !keys::holds(&bytes, person.device.as_bytes()),
            "{file:?} holds the device ID's bytes"
        );
    }
}

/// The forms in which a run may name `machine`'s state directory: as the machine sets it, and its
/// canonical path, with and without Windows' verbatim prefix.
pub fn state_dir_forms(machine: &Machine) -> Vec<String> {
    let set = machine.state_dir();
    let mut forms = vec![set.display().to_string()];
    if let Ok(canonical) = set.canonicalize() {
        let canonical = canonical.display().to_string();
        forms.push(canonical.trim_start_matches(r"\\?\").to_owned());
        forms.push(canonical);
    }
    forms
}

/// The lines of `text` that say the account is deleted: each holds `account` and [`DELETED`].
pub fn deleted_lines(text: &str) -> Vec<String> {
    (lines_holding(text, DELETED).into_iter())
        .filter(|line| line.to_lowercase().contains("account"))
        .collect()
}

/// Fails unless `text`, what a run on another device of the deleted account showed, says once that
/// the server reports the account deleted and names `machine`'s state directory.
#[track_caller]
pub fn assert_says_deleted_once(text: &str, machine: &Machine, what: &str) {
    let lines = deleted_lines(text);
    assert_eq!(
        lines.len(),
        1,
        "{what}: not one line saying the account is deleted: {text}"
    );
    assert!(
        state_dir_forms(machine)
            .iter()
            .any(|form| text.contains(form.as_str())),
        "{what}: the state directory {:?} is not named: {text}",
        machine.state_dir()
    );
}

/// Waits for up to [`WAIT`] until `done` holds; `shown` gives what to show if it never does.
#[track_caller]
pub fn until(what: &str, mut done: impl FnMut() -> bool, shown: impl Fn() -> String) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(
            Instant::now() < deadline,
            "{what} after {WAIT:?}: {}",
            shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
}
