//! `SessionStart` registers the session with the helper, starting one if none runs; the helper
//! keeps running while a session is registered. `SessionEnd` ends the registration: messages then
//! wait for the hooks and `inbox`.

use std::thread;
use std::time::Duration;

use crate::helped::{EXIT_LIMIT, IDLE_SETTING};
use crate::hooks::{Event, assert_own_line};
use crate::receipts::shows_read_once;
use crate::registered::{Ada, QUIET, lines_stay};
use crate::solo::WEB;
use crate::standin::{Standin, Take};

/// The helper's idle time in this test, in seconds.
const SHORT_IDLE: &str = "2";

/// How long the test watches the helper of a registered session: several of its idle times.
const REGISTERED_FOR: Duration = Duration::from_secs(8);

/// No helper runs on Ada's machine. web's session starts, its `SessionStart` run with a helper
/// idle time of [`SHORT_IDLE`] seconds: one helper runs, and it still runs after
/// [`REGISTERED_FOR`] with no command. The session's `SessionEnd` prints nothing, and the helper
/// exits.
#[test]
fn helper_runs_while_a_session_is_registered_and_exits_after_session_end() {
    let ada = Ada::new(4_919_431_001, "ada-socket-helper-lifetime");
    let session = Standin::new(Take::Accept);
    ada.helped.assert_none();

    ada.starts_with(WEB, &session, &[(IDLE_SETTING, SHORT_IDLE)]);
    let helper = ada.helped.one_helper();
    thread::sleep(REGISTERED_FOR);

    assert!(
        ada.helped.helpers().contains(&helper),
        "the helper exited while the session was registered"
    );
    ada.ends(WEB, &session);
    ada.helped.await_exit(helper, EXIT_LIMIT);
}

/// What api says to web once its sessions ended; neither is anywhere else.
const AFTER_END: &str = "lifetime test: api asks web after its newer session ended";
const AFTER_BOTH: &str = "lifetime test: api asks web after both its sessions ended";

/// Two sessions of web start, the older one first; the newer one ends: its `SessionEnd` prints
/// nothing. api sends web a message: the older session's socket gets it, as its line from api at
/// `own`, and the newer one's gets nothing. The older session ends too, and api sends web another
/// message: neither socket gets it in [`QUIET`]; web's `PostToolUse`, in a later session of its,
/// gives it, and the person's `skald recv` shows `read` of it once.
#[test]
fn session_end_ends_the_registration_and_messages_then_wait_for_the_hooks() {
    let ada = Ada::new(4_919_432_001, "ada-socket-after-the-end");
    let (older, newer) = (Standin::new(Take::Queue), Standin::new(Take::Queue));
    let later = Standin::new(Take::Accept);
    ada.starts(WEB, &older);
    ada.starts(WEB, &newer);
    ada.ends(WEB, &newer);

    let first = ada.web.own(AFTER_END);

    let line = older.wait_lines(1).remove(0);
    assert_own_line(&line.content(), AFTER_END, first, &ada.web.api());
    assert_eq!(newer.lines().len(), 0, "{:#?}", newer.lines());
    ada.ends(WEB, &older);
    let second = ada.web.own(AFTER_BOTH);
    lines_stay(&older, AFTER_BOTH, 0, QUIET);
    assert_eq!(newer.lines().len(), 0, "{:#?}", newer.lines());
    let context = ada
        .hook(WEB, &later, Event::PostToolUse)
        .context(Event::PostToolUse);
    assert_own_line(&context, AFTER_BOTH, second, &ada.web.api());
    shows_read_once(ada.machine(), second);
}
