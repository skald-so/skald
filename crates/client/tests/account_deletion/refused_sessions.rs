//! A refused session alone deletes nothing on the machine: `skald delete-account` takes the
//! account as deleted only as the server says so, and a run whose answer it did not get, or got
//! from no server, keeps its mark until the next run finds out.

use crate::cloud::Cloud;
use crate::deletion::{
    AtGrant, DELETE_ACCOUNT, DELETE_REQUEST, Deletion, Held, assert_deleted, assert_keeps_nothing,
    assert_kept, delete_at_terminal, deleted_lines, drive,
};
use crate::fixtures::fronted;
use crate::linking::says;
use crate::relay::Lose;
use crate::talk::ADA_HOST;
use crate::terminal::Terminal;
use crate::world::{Person, World};

/// What a run whose deletion got no answer of the server's says.
const MAY_BE_DELETED: &str = "may have gone through";
/// The request timeout of a run that waits for an answer the relay withholds, in seconds.
const TIMEOUT: &str = "5";

/// Ends `person`'s session, as a web session ends it.
fn end_session(cloud: &Cloud, person: &Person) {
    cloud.execute(&format!(
        "DELETE FROM sessions WHERE token_hash = sha256(convert_to('{}', 'UTF8'))",
        person.token
    ));
}

/// Fails unless `person`'s machine keeps its local state and its session token, and `text`, what
/// its run showed, does not say the account is deleted.
#[track_caller]
fn assert_state_kept(cloud: &Cloud, person: &Person, text: &str) {
    assert!(deleted_lines(text).is_empty(), "it says deleted: {text}");
    assert!(
        !person.machine.sqlite_files().is_empty(),
        "the local state went: {text}"
    );
    assert!(
        !cloud.token_files(&person.machine).is_empty(),
        "the session token went: {text}"
    );
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
    assert_state_kept(&world.cloud, person, &shown.text);
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
    end_session(&world.cloud, &ada);

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
    let cloud = &world.cloud;
    let ada = world.sign_up(6_120_009_002, "ada-is-blocked-then-ends", ADA_HOST);
    cloud.relay.block(DELETE_REQUEST);
    let blocked = delete_at_terminal(&world, &ada.machine, ada.handle.as_str(), AtGrant::Refuse);
    cloud.relay.stop_refusing();
    let text = &blocked.shown.text;
    assert!(
        !blocked.shown.success && says(text, &[MAY_BE_DELETED]),
        "{text}"
    );
    assert_kept(cloud, &ada);
    end_session(cloud, &ada);

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

    let args = [DELETE_ACCOUNT, "--timeout", TIMEOUT];
    let mut waiting = Terminal::start(&ada.machine, &args);
    let person = (ada.handle.as_str(), AtGrant::Refuse, cloud.grants().len());
    drive(&world, &mut waiting, person, Terminal::exited);
    let lost = waiting.finish();
    assert!(
        !lost.success && says(&lost.text, &[MAY_BE_DELETED]),
        "{}",
        lost.text
    );
    assert_deleted(cloud, &ada);
    assert_state_kept(cloud, &ada, &lost.text);

    assert_finishes(&world, &ada, &held);
}
