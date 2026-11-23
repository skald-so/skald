//! A refused session alone deletes nothing on the machine: `skald delete-account` takes the
//! account as deleted only as the server says so, and a run whose answer it did not get, or got
//! from no server, keeps its mark until the next run finds out. A device without its certificate,
//! which cannot ask, erases its keys and state only once its person types the username again.

use skald_github_stand_in::User;

use crate::cloud::Cloud;
use crate::deletion::{
    AtGrant, DELETE_ACCOUNT, DELETE_REQUEST, Deletion, Held, assert_deleted, assert_keeps_nothing,
    assert_kept, assert_says_deleted_once, delete_at_terminal, deleted_lines, drive,
    state_dir_forms,
};
use crate::fixtures::fronted;
use crate::linking::{NewDevice, says};
use crate::machine::{KEY_STORAGE, Machine};
use crate::relay::Lose;
use crate::system::Entry;
use crate::talk::ADA_HOST;
use crate::terminal::{Shown, Terminal};
use crate::world::{Person, World, handle_of, run};

/// What a run whose deletion got no answer of the server's says.
const MAY_BE_DELETED: &str = "may have gone through";
/// The request timeout of a run that waits for an answer the relay withholds, in seconds.
const TIMEOUT: &str = "5";

/// Ends the session of `token`, as a web session ends it.
fn end_session(cloud: &Cloud, token: &str) {
    cloud.execute(&format!(
        "DELETE FROM sessions WHERE token_hash = sha256(convert_to('{token}', 'UTF8'))"
    ));
}

/// The host name of Ada's machine that waits for linking.
const WAITING_HOST: &str = "ada-waiting-laptop";

/// Fails unless `machine` keeps its local state and its session token, and `text`, what its run
/// showed, does not say the account is deleted.
#[track_caller]
fn assert_state_kept(cloud: &Cloud, machine: &Machine, text: &str) {
    assert!(deleted_lines(text).is_empty(), "it says deleted: {text}");
    assert!(
        !machine.sqlite_files().is_empty(),
        "the local state went: {text}"
    );
    assert!(
        !cloud.token_files(machine).is_empty(),
        "the session token went: {text}"
    );
}

/// `skald delete-account` on `machine` at a terminal, `typed` typed as the username, with a request
/// timeout of [`TIMEOUT`], which the relay's withheld answer outlasts; fails unless it fails, saying
/// the deletion may have gone through. Gives what it showed.
#[track_caller]
fn lost_to_the_timeout(world: &World, machine: &Machine, typed: &str) -> Shown {
    let args = [DELETE_ACCOUNT, "--timeout", TIMEOUT];
    let mut run = Terminal::start(machine, &args);
    let person = (typed, AtGrant::Refuse, world.cloud.grants().len());
    drive(world, &mut run, person, Terminal::exited);
    let lost = run.finish();
    assert!(
        !lost.success && says(&lost.text, &[MAY_BE_DELETED]),
        "{}",
        lost.text
    );
    lost
}

/// `skald delete-account` at a terminal on `machine` of `person`'s account, whose session is
/// `token`, typing the username, gets a proxy's answer, not the server's, to its request that
/// deletes the account, which never reaches the server: it fails, saying the deletion may have gone
/// through, and the account stays. The session then ends while the account lives.
#[track_caller]
fn blocked_then_ended(world: &World, person: &Person, (machine, token): (&Machine, &str)) {
    let cloud = &world.cloud;
    cloud.relay.block(DELETE_REQUEST);
    let typed = person.handle.as_str();
    let blocked = delete_at_terminal(world, machine, typed, AtGrant::Refuse);
    cloud.relay.stop_refusing();
    let text = &blocked.shown.text;
    assert!(
        !blocked.shown.success && says(text, &[MAY_BE_DELETED]),
        "{text}"
    );
    assert_kept(cloud, person);
    end_session(cloud, token);
}

/// A new machine of `first`'s account that waits for linking: signed in, without its certificate,
/// its run at the terminal ended.
fn waiting(world: &World, first: &Person) -> NewDevice {
    let mut waiting = NewDevice::start(world, first, WAITING_HOST);
    waiting.kill();
    waiting
}

/// The waiting machine of `first`'s account, whose session the server no longer takes after a
/// deletion it sent: its `skald delete-account` at a terminal asks for the username once, and its
/// person types another; the run fails, names the state directory and keeps the machine's local
/// state.
#[track_caller]
fn assert_waiting_keeps_its_state(world: &World, first: &Person, waiting: &NewDevice) {
    let machine = &waiting.machine;
    let other = format!("not-{}", first.handle.as_str());
    let refused = delete_at_terminal(world, machine, &other, AtGrant::Refuse);
    let Deletion { shown, asked, .. } = refused;
    assert!(
        !shown.success && asked == 1,
        "asked {asked}: {}",
        shown.text
    );
    assert!(
        state_dir_forms(machine)
            .iter()
            .any(|form| shown.text.contains(form.as_str())),
        "the state directory is not named: {}",
        shown.text
    );
    assert_state_kept(&world.cloud, machine, &shown.text);
}

/// The waiting machine `laptop` of `first`'s account runs `skald delete-account` at a terminal,
/// typing the username; the relay withholds the server's answer to its request that deletes the
/// account, and the run's request timeout passes, as [`lost_to_the_timeout`]. Its next `skald
/// delete-account` at a terminal, whose session the server no longer takes, asks for the username
/// once; its person types it, and the run exits with success: the machine then keeps no local state
/// and no file with a session token.
#[track_caller]
fn assert_erased_after_a_lost_answer(world: &World, first: &Person, laptop: &NewDevice) {
    let cloud = &world.cloud;
    let typed = first.handle.as_str();
    cloud.relay.lose_answer(DELETE_REQUEST, Lose::Withhold);
    lost_to_the_timeout(world, &laptop.machine, typed);
    assert_deleted(cloud, first);

    let machine = &laptop.machine;
    let erased = delete_at_terminal(world, machine, typed, AtGrant::Refuse);

    let Deletion { shown, asked, .. } = erased;
    assert!(shown.success && asked == 1, "asked {asked}: {}", shown.text);
    assert!(
        machine.sqlite_files().is_empty(),
        "the local state stays: {}",
        shown.text
    );
    let tokens = cloud.token_files(machine);
    assert!(tokens.is_empty(), "{tokens:?} stay: {}", shown.text);
}

/// `machine` signs in to `other`'s account; fails unless it does.
#[track_caller]
pub fn assert_signs_in(world: &World, machine: &Machine, other: &User) {
    let signed_in = world
        .cloud
        .signed_in(&world.browser.page(), machine, other, &[]);
    assert!(signed_in.success, "{signed_in:?}");
}

/// `person`'s `skald delete-account` at a terminal fails, asking nothing and sending no request
/// that deletes the account, and its machine keeps its local state.
#[track_caller]
fn assert_refused_keeping_state(world: &World, person: &Person) {
    let since = world.cloud.relay.mark();
    let refused = delete_at_terminal(
        world,
        &person.machine,
        person.handle.as_str(),
        AtGrant::Refuse,
    );
    let Deletion { shown, asked, .. } = refused;
    assert!(
        !shown.success && asked == 0,
        "asked {asked}: {}",
        shown.text
    );
    let deletions = world.cloud.count_sent(since, DELETE_REQUEST);
    assert_eq!(deletions, 0, "{}", shown.text);
    assert_state_kept(&world.cloud, &person.machine, &shown.text);
}

/// `person`'s `skald delete-account` at a terminal, once the server deleted its account, asks
/// nothing, signs in to nothing, says the account is deleted once and exits with success; its
/// machine then keeps nothing of the account, of which it held `held`.
#[track_caller]
pub fn assert_finishes(world: &World, person: &Person, held: &Held) {
    let finished = delete_at_terminal(
        world,
        &person.machine,
        person.handle.as_str(),
        AtGrant::Refuse,
    );
    let Deletion { shown, asked, .. } = finished;
    assert!(shown.success && asked == 0, "asked {asked}: {}", shown.text);
    assert_eq!(deleted_lines(&shown.text).len(), 1, "{}", shown.text);
    assert_keeps_nothing(&world.cloud, person, held);
}

/// Ada's session ends while her account lives. Her `skald delete-account` at a terminal fails,
/// asks nothing, sends no request that deletes the account, and her machine keeps its local state.
#[test]
fn a_session_ended_while_the_account_lives_keeps_the_machines_state() {
    let (world, _front) = fronted();
    let ada = world.sign_up(6_120_009_001, "ada-ends-a-session", ADA_HOST);
    end_session(&world.cloud, &ada.token);

    assert_refused_keeping_state(&world, &ada);
}

/// An earlier `skald delete-account` of Ada's at a terminal, typing her username, got a proxy's
/// answer, not the server's, to its request to delete the account, which never reached the server:
/// it failed, saying the deletion may have gone through, and her account stays. Her session then
/// ends while her account lives: her next `skald delete-account` at a terminal fails, asks nothing,
/// and her machine keeps its local state.
#[test]
fn a_session_ended_after_a_deletion_that_never_reached_the_server_keeps_the_machines_state() {
    let (world, _front) = fronted();
    let ada = world.sign_up(6_120_009_002, "ada-is-blocked-then-ends", ADA_HOST);
    blocked_then_ended(&world, &ada, (&ada.machine, &ada.token));

    assert_refused_keeping_state(&world, &ada);
}

/// The relay withholds the server's answer to the request of Ada's `skald delete-account` at a
/// terminal that deletes her account, and the run's request timeout passes: the run fails, saying
/// the deletion may have gone through, as it has, and her machine keeps its local state. Her next
/// `skald delete-account` at a terminal finishes it.
#[test]
fn an_answer_lost_to_the_timeout_says_the_account_may_be_deleted_and_the_next_run_finishes() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_009_003, "ada-waits-out-the-answer", ADA_HOST);
    let held = Held::of(cloud, &ada);
    cloud.relay.lose_answer(DELETE_REQUEST, Lose::Withhold);

    let lost = lost_to_the_timeout(&world, &ada.machine, ada.handle.as_str());
    assert_deleted(cloud, &ada);
    assert_state_kept(cloud, &ada.machine, &lost.text);

    assert_finishes(&world, &ada, &held);
}

/// Ada's machine that waits for linking, without its certificate, loses the server's answer to its
/// deletion of her account; its next `skald delete-account` at a terminal erases its keys and state
/// once she types her username again ([`assert_erased_after_a_lost_answer`]). The machine then
/// signs in to Bob's account.
#[test]
fn on_a_device_waiting_for_linking_after_a_lost_answer_the_typed_username_erases_the_machine() {
    let (world, _front) = fronted();
    let ada = world.sign_up(6_120_009_004, "ada-waits-while-linking", ADA_HOST);
    let bob = world.cloud.user(6_120_009_007, "bob-takes-the-laptop");
    let laptop = waiting(&world, &ada);

    assert_erased_after_a_lost_answer(&world, &ada, &laptop);

    assert_signs_in(&world, &laptop.machine, &bob);
}

/// Ada's machine that waits for linking, without its certificate, runs `skald delete-account` at a
/// terminal, typing her username; a proxy's answer, not the server's, takes its request that
/// deletes her account, which never reaches the server, and her account stays. The machine's
/// session then ends while her account lives: its next `skald delete-account` at a terminal asks
/// for the username once, and Ada types another: the run fails, names the state directory and
/// keeps the machine's local state.
#[test]
fn on_a_device_waiting_for_linking_a_session_ended_after_a_blocked_deletion_keeps_its_state() {
    let (world, _front) = fronted();
    let ada = world.sign_up(6_120_009_005, "ada-links-blocked-then-ends", ADA_HOST);
    let laptop = waiting(&world, &ada);
    blocked_then_ended(&world, &ada, (&laptop.machine, &laptop.token));

    assert_waiting_keeps_its_state(&world, &ada, &laptop);
}

/// With the system's key storage, only with `SKALD_TEST_REAL_KEY_STORAGE=1`: Ada's machine keeps
/// its secrets in the key storage's entry of the machine. Its `skald delete-account` at a terminal
/// loses the server's answer to the timeout, as the relay withholds it, after the server deleted
/// her account; its `skald recv` then learns from the server that the account is deleted. Its next
/// `skald delete-account` at a terminal, from that mark, exits with success, and the entry is gone.
/// The test deletes the entry.
#[test]
fn with_the_systems_key_storage_a_run_from_the_deleted_mark_deletes_the_entry() {
    let real = std::env::var_os("SKALD_TEST_REAL_KEY_STORAGE");
    if real.is_none_or(|real| real != "1") {
        return;
    }
    let machine = Machine::new()
        .with_host_name(ADA_HOST)
        .with_env(KEY_STORAGE, "auto");
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = cloud.user(6_120_009_006, "ada-marks-her-keyring");
    let entry = Entry::of(&machine);
    let signed_in = cloud.signed_in(&world.browser.page(), &machine, &ada, &[]);
    assert!(signed_in.success, "{signed_in:?}");
    let handle = handle_of(cloud, &cloud.last_token());
    assert!(entry.value().is_some(), "the entry holds no secrets");
    cloud.relay.lose_answer(DELETE_REQUEST, Lose::Withhold);
    lost_to_the_timeout(&world, &machine, handle.as_str());
    let learned = run(&machine, &["recv"]).shown();
    assert_says_deleted_once(&learned, &machine, "skald recv");

    let finished = delete_at_terminal(&world, &machine, handle.as_str(), AtGrant::Refuse);

    assert!(finished.shown.success, "{}", finished.shown.text);
    assert_eq!(entry.value(), None, "the entry is kept");
    assert_signs_in(
        &world,
        &machine,
        &cloud.user(6_120_009_008, "bob-takes-the-keyring"),
    );
}

/// With the system's key storage, only with `SKALD_TEST_REAL_KEY_STORAGE=1`: Ada's machine that
/// waits for linking keeps its secrets in the key storage's entry of the machine. It loses the
/// server's answer to its deletion of her account, and its next `skald delete-account` at a
/// terminal erases its keys and state once she types her username again
/// ([`assert_erased_after_a_lost_answer`]): the entry is gone, and the machine then signs in to
/// Bob's account. The test deletes the entry.
#[test]
fn with_the_systems_key_storage_a_device_waiting_for_linking_erases_the_entry_and_signs_in_again() {
    let real = std::env::var_os("SKALD_TEST_REAL_KEY_STORAGE");
    if real.is_none_or(|real| real != "1") {
        return;
    }
    let machine = Machine::new()
        .with_host_name(WAITING_HOST)
        .with_env(KEY_STORAGE, "auto");
    let entry = Entry::of(&machine);
    let (world, _front) = fronted();
    let ada = world.sign_up(6_120_009_009, "ada-waits-with-a-keyring", ADA_HOST);
    let bob = world
        .cloud
        .user(6_120_009_010, "bob-takes-the-waiting-keyring");
    let mut laptop = NewDevice::start_on(&world, &ada, machine, WAITING_HOST);
    laptop.kill();
    assert!(entry.value().is_some(), "the entry holds no secrets");

    assert_erased_after_a_lost_answer(&world, &ada, &laptop);
    assert_eq!(entry.value(), None, "the entry is kept");

    assert_signs_in(&world, &laptop.machine, &bob);
}
