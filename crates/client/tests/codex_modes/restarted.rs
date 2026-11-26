//! The delivery settings and the wakes Codex sessions counted are kept across runs: the helper,
//! killed and started again, keeps waking by them.

use crate::codex::{Codex, Event};
use crate::delivery::{
    PULL, api_sends_in, assert_wakes, listed, mode_of, of_channel, other_channel, set_mode, wakes,
    wakes_of,
};
use crate::modes::quiet_turn;
use crate::queued::assert_inbox_gives;
use crate::senders::OWN;
use crate::standin::{assert_queued, assert_quiet, web_in_app};

/// What api says to web; none is anywhere else.
const BEFORE: &str = "codex modes test: api asks web, before the restart, to vacuum the db";
const PULLED: &str = "codex modes test: api asks web, in pull after the restart, to archive";
const AFTER: &str = "codex modes test: api asks web, after the restart, to reindex";

/// The person sets `own` in the other channel to `pull`. web's session starts in a Codex app of its
/// own: it is idle. api sends web a message in the default channel: the app's `codex` runs `codex
/// queue`, and the default channel has one wake; a turn runs that gives nothing. The helper is
/// killed: the listing still gives one wake and `pull`. web's session starts again in the same
/// app, which starts a helper. api sends web a message in the other channel: the app's `codex` does
/// not run again. api sends web one in the default channel: `codex queue` runs once more, and the
/// default channel has two wakes, the other none. web's `inbox` gives the three messages.
pub fn settings_and_wakes_survive_the_helper_s_restart() {
    let mut codex = Codex::new(4_929_005_001, "ada-codex-modes-restart");
    let solo = &codex.web.solo;
    let (default, other) = (solo.default_channel(), other_channel(solo));
    set_mode(solo, other, OWN, PULL, None);
    let (app, web, mut process) = web_in_app(&codex);

    let before = codex.web.own(BEFORE);
    let ran = app.await_runs(1);
    assert_queued(&ran[0], (&web, &app), &[BEFORE], &[before], &[]);
    assert_wakes(&codex.web.solo, default, 1);
    quiet_turn(&mut process, &codex, &web, "the turn before the restart");

    codex.helped.kill(codex.helped.one_helper());
    let objects = listed(&codex.web.solo);
    let (kept, set) = (of_channel(&objects, default), of_channel(&objects, other));
    assert_eq!(wakes_of(kept), 1, "wakes lost: {kept}");
    assert_eq!(mode_of(set, OWN), (PULL.to_owned(), None), "{set}");
    process
        .hook(&codex, &web, Event::SessionStart)
        .assert_silent("SessionStart again");

    let pulled = api_sends_in(&codex.web.solo, other, PULLED);
    assert_quiet(&[(
        &app,
        1,
        "codex queue for a pulled message after the restart",
    )]);
    let after = codex.web.own(AFTER);
    let ran = app.await_runs(2);
    assert_queued(&ran[1], (&web, &app), &[AFTER], &[after], &[]);
    quiet_turn(&mut process, &codex, &web, "the turn after the restart");

    assert_wakes(&codex.web.solo, default, 2);
    assert_eq!(wakes(&codex.web.solo, other), 0, "a pulled message woke it");
    let sent = [(BEFORE, before), (PULLED, pulled), (AFTER, after)];
    assert_inbox_gives(&mut codex, &sent);
}
