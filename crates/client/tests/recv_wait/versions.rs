//! Waits beside a helper of another version: a helper that exits for a newer command ends its
//! sessions, and an older one-shot wait gives way to a newer helper that starts meanwhile, even with
//! nothing come.

use crate::agents::CLAUDE;
use crate::helped::{EXIT_LIMIT, ONE_SHOT, VERSION, VERSION_SETTING, older, times};
use crate::kit::{RECV, received};
use crate::talk::{BOB_HOST, Line, assert_shows_once};
use crate::waits::{
    WAIT_FLAG, Waits, assert_got, assert_keeps_waiting, await_connections, await_helper_beside,
    helpers_beside,
};
use crate::world::AGENT;

/// What Bob's device says; none is anywhere else.
const TO_ALL: &str = "version test: the nightly job moved to midnight";
const TO_ADA: &str = "version test: ada, please approve the budget yourself";

/// What an older command says when the helper is newer.
const AGAIN: &str = "again";

/// Through a helper of an older version than the copy's, which a run as claude in `~/work/acme`
/// started, claude waits, acting as that version too, and keeps waiting while nothing comes. The
/// person's `skald recv` of the copy's version succeeds; the older helper exits, and the waiting
/// session fails. Bob's device sends a message to the channel: claude's next `skald recv --wait`
/// shows it once, by its reference, through a new helper.
#[test]
fn session_on_a_helper_that_exits_for_a_newer_command_fails_and_the_next_gets_messages() {
    let mut waits = Waits::new(
        [4_148_041_001, 4_148_041_002],
        ["ada-upgrades-mid-wait", "bob-writes-after-upgrade"],
    );
    let older = older();
    let as_older = [(VERSION_SETTING, older.as_str())];
    let args = [AGENT, CLAUDE, RECV];
    let ran = (waits
        .helped
        .start_in(&waits.agents.folder, &as_older, &args))
    .finish();
    assert!(ran.success, "{ran:?}");
    let old_helper = waits.helped.one_helper();
    let mut waiting = waits.session_with(&as_older, CLAUDE);
    assert_keeps_waiting(&mut waiting);

    waits.helped.received();

    waits.helped.await_exit(old_helper, EXIT_LIMIT);
    let ended = waiting.finish();
    assert!(!ended.success, "it outlived its helper: {ended:?}");
    let bob = waits.agents.bob();
    let id = waits.agents.bob_sends(&Line::new(&bob, 1, TO_ALL));
    let next = waits.session(CLAUDE).finish();
    assert_got(&next, TO_ALL, id);
    assert_ne!(
        waits.helped.one_helper(),
        old_helper,
        "the older helper still serves"
    );
}

/// The device has run as claude in `~/work/acme`, in one-shot mode, and the person's `skald recv`
/// has shown what came so far. The person's `skald recv --wait` in one-shot mode, acting as an
/// older version than the copy's, waits on its own connection. claude's `skald recv --wait` of the
/// copy's version then starts a helper and waits through it, on the helper's connection. Bob's
/// device sends a message to Ada as a person: the older wait ends, failing, saying to run it again
/// and naming both versions, while claude's session keeps waiting and the helper keeps running.
/// Bob's device then sends a message to the channel, which claude's session shows once, by its
/// reference. The older wait and the person's next `skald recv`, through that helper, showed each
/// of Bob's messages once in all.
#[test]
fn older_one_shot_wait_ends_when_a_newer_helper_starts_and_the_helper_and_its_sessions_go_on() {
    let mut waits = Waits::new(
        [4_148_042_001, 4_148_042_002],
        ["ada-waits-on-a-stale-binary", "bob-writes-past-stale"],
    );
    waits.agents.run_as(CLAUDE);
    received(&waits.agents.talk.ada.machine);
    let older = older();
    let since = waits.agents.talk.cloud().relay.mark();
    let as_older = [(VERSION_SETTING, older.as_str())];
    let mut stale = waits.helped.start(&as_older, &[ONE_SHOT, RECV, WAIT_FLAG]);
    await_connections(&waits.agents.talk, &mut stale, (since, 1));
    let mut session = waits.session(CLAUDE);
    let helper = await_helper_beside(&waits.helped, &[&stale, &session]);
    // The helper connects once it holds the lock and listens: a message that came before would be
    // the older wait's own.
    await_connections(&waits.agents.talk, &mut session, (since, 2));
    let bob = waits.agents.bob();
    let mut to_ada = Line::new(&bob, 1, TO_ADA);
    to_ada.to = Some(waits.agents.talk.address(&waits.agents.talk.ada));

    waits.agents.bob_sends(&to_ada);

    let ended = stale.finish();
    assert!(!ended.success, "{ended:?}");
    let said = ended.shown();
    assert!(said.to_lowercase().contains(AGAIN), "{ended:?}");
    for version in [older.as_str(), VERSION] {
        assert!(times(&said, version) > 0, "{version} not named: {ended:?}");
    }
    session.assert_running();
    let beside = helpers_beside(&waits.helped, &[&session]);
    assert_eq!(beside, [helper], "the helper did not keep running");
    let id = waits.agents.bob_sends(&Line::new(&bob, 2, TO_ALL));
    let got = session.finish();
    assert_got(&got, TO_ALL, id);
    assert!(!got.shown().contains(TO_ADA), "handed over: {got:?}");
    let person = waits.helped.received();
    let channel = waits.agents.talk.channel;
    let shown = format!("{said}\n{}", person.shown());
    for text in [TO_ADA, TO_ALL] {
        assert_shows_once(&shown, text, (channel, &bob, BOB_HOST));
    }
    assert_eq!(waits.helped.helpers(), [helper], "not the same helper");
}

/// The person's `skald recv` has shown what came so far. The person's `skald recv --wait` in
/// one-shot mode, acting as an older version than the copy's, keeps waiting. The person's
/// `skald recv` of the copy's version then starts a helper. With nothing come, the older
/// wait ends, failing, saying to run it again, and the helper keeps running.
#[test]
fn older_one_shot_wait_ends_once_a_newer_helper_starts_with_nothing_come() {
    let waits = Waits::new(
        [4_148_043_001, 4_148_043_002],
        ["ada-updates-mid-wait", "bob-has-nothing-to-say"],
    );
    let older = older();
    received(&waits.agents.talk.ada.machine);
    let args = [ONE_SHOT, RECV, WAIT_FLAG];
    let mut stale = (waits.helped).start(&[(VERSION_SETTING, &older)], &args);
    assert_keeps_waiting(&mut stale);

    waits.helped.received();

    let ended = stale.finish();
    assert!(!ended.success, "{ended:?}");
    assert!(ended.shown().to_lowercase().contains(AGAIN), "{ended:?}");
    waits.helped.one_helper();
}
