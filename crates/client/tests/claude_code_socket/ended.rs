//! A session that ends before its file confirms what its socket took, by `SessionEnd` or killed,
//! leaves it to the hooks and `inbox`, unless its `Stop` hook read the confirmation first. A killed
//! session's socket file stays behind, but the socket takes no connection: the registration ends
//! as for a socket that is gone.

use serde_json::{Value, json};

use crate::helped::{EXIT_LIMIT, IDLE_SETTING, ONE_SHOT_SETTING};
use crate::hooks::{Event, assert_own_line};
use crate::receipts::shows_read_once;
use crate::registered::Ada;
use crate::solo::WEB;
use crate::standin::{Standin, Take};

/// The helper's idle time in these tests, in seconds.
const SHORT_IDLE: &str = "2";

/// What api says to web; none is anywhere else.
const AFTER_KILL: &str = "ended test: api asks web after its newer session was killed";
const BEFORE_END: &str = "ended test: api asks web before its session ends";
const BEFORE_KILL: &str = "ended test: api asks web before its session is killed";
const BEFORE_STOP: &str = "ended test: api asks web before its Stop hook";

/// Two sessions of web start, the older one first, its `SessionStart` with a helper idle time of
/// [`SHORT_IDLE`] seconds. The newer one is killed: its socket's file stays. api sends web a
/// message: the older session's socket gets it, as its line from api at `own`, and the newer one's
/// gets nothing. The older session is killed too: the helper exits.
#[test]
fn a_killed_session_s_socket_left_behind_ends_its_registration() {
    let ada = Ada::new(4_919_461_001, "ada-socket-killed-session");
    let (older, newer) = (Standin::new(Take::Queue), Standin::new(Take::Queue));
    ada.starts_with(WEB, &older, &[(IDLE_SETTING, SHORT_IDLE)]);
    let helper = ada.helped.one_helper();
    ada.starts(WEB, &newer);
    newer.dies();
    assert!(newer.socket().exists(), "the killed session's socket went");

    let msg_id = ada.web.own(AFTER_KILL);

    let line = older.wait_lines(1).remove(0);
    assert_own_line(&line.content(), AFTER_KILL, msg_id, &ada.web.api());
    assert_eq!(newer.lines().len(), 0, "{:#?}", newer.lines());
    older.dies();
    ada.helped.await_exit(helper, EXIT_LIMIT);
}

/// For each way a session ends, by `SessionEnd` and killed: a session of web starts and queues what
/// comes. api sends web a message, which the socket gets; the session ends with it still queued.
/// web's `PostToolUse`, in a later session of its, gives the message as its line from api at `own`,
/// and the person's `skald recv` shows `read` of it once.
#[test]
fn what_the_socket_took_unconfirmed_waits_for_the_hooks_once_the_session_ends() {
    let ada = Ada::new(4_919_462_001, "ada-socket-ends-unconfirmed");
    let later = Standin::new(Take::Accept);
    for (says, killed) in [(BEFORE_END, false), (BEFORE_KILL, true)] {
        let session = Standin::new(Take::Queue);
        ada.starts(WEB, &session);
        let msg_id = ada.web.own(says);
        session.wait_lines(1);

        match killed {
            true => session.dies(),
            false => ada.ends(WEB, &session),
        }

        let context = ada.hook_gives(WEB, &later);
        assert_own_line(&context, says, msg_id, &ada.web.api());
        shows_read_once(ada.machine(), msg_id);
    }
}

/// web's session starts and takes what comes into the conversation. api sends web a message; once
/// the socket has it, the helper is killed, and the model replies. The session's `Stop` hook runs in
/// one-shot mode, so through no helper, and prints nothing; then the session is killed. The
/// person's `skald recv` shows `read` of the message once, and web's `PostToolUse`, in a later
/// session of its, prints nothing.
#[test]
fn the_stop_hook_confirms_from_the_session_file_without_the_helper() {
    let ada = Ada::new(4_919_463_001, "ada-socket-stop-confirms");
    let (session, later) = (Standin::new(Take::Accept), Standin::new(Take::Accept));
    ada.starts(WEB, &session);
    let msg_id = ada.web.own(BEFORE_STOP);
    session.wait_lines(1);
    ada.helped.kill(ada.helped.one_helper());
    session.model_replies();

    let dir = ada.web.solo.dir(WEB);
    let mut input: Value = serde_json::from_str(&Event::Stop.input(&dir)).expect("JSON");
    input["session_id"] = json!(session.session_id());
    input["transcript_path"] = json!(session.file().to_string_lossy());
    (ada.hook_of(WEB, &session, &input, &[(ONE_SHOT_SETTING, "1")]))
        .assert_silent("the one-shot Stop");
    session.dies();

    shows_read_once(ada.machine(), msg_id);
    (ada.hook(WEB, &later, Event::PostToolUse)).assert_silent("PostToolUse after the confirmation");
}
