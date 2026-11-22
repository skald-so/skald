//! A crash after the write and before the confirmation neither loses the message nor issues `read`
//! twice; the client reads the registered session's file for `msg_id`s only and keeps no line of
//! it; the session's token stays in the local state.

use crate::hooks::Event;
use crate::receipts::shows_read_once;
use crate::registered::{Ada, QUIET, lines_stay};
use crate::solo::WEB;
use crate::standin::{Standin, Take};

/// What api says to web; it is nowhere else.
const BEFORE_CRASH: &str = "crash test: api asks web just before the helper dies";

/// web's session starts and takes what comes into the conversation. api sends web a message; once
/// the socket has it, the helper is killed, and then the model replies. The session's `Stop` hook
/// runs as its turn ends and prints nothing. The person's `skald recv` shows `read` of the message
/// once, and in [`QUIET`] more the socket has still got it once.
#[test]
fn helper_killed_after_the_write_gives_read_once_and_writes_it_once() {
    let ada = Ada::new(4_919_441_001, "ada-socket-helper-crash");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);
    let msg_id = ada.web.own(BEFORE_CRASH);
    session.wait_lines(1);

    ada.helped.kill(ada.helped.one_helper());
    session.model_replies();
    (ada.hook(WEB, &session, Event::Stop)).assert_silent("Stop after the crash");

    shows_read_once(ada.machine(), msg_id);
    lines_stay(&session, BEFORE_CRASH, 1, QUIET);
}

/// What the conversation holds that no message does, and what api says to web.
const PLANTED: &str = "planted-secret-7f3c9a1e5b2d4086";
const AFTER_PLANTED: &str = "privacy test: api asks web after the planted line";

/// web's session starts and takes what comes into the conversation; a tool result in its session
/// file holds [`PLANTED`]. api sends web a message; the model replies, and the person's
/// `skald recv` shows `read` of it once. Then no file of Ada's machine holds [`PLANTED`], and only
/// files in its state directory hold the session's token.
#[test]
fn local_state_keeps_no_line_of_the_session_file_and_the_token_only_in_it() {
    let ada = Ada::new(4_919_442_001, "ada-socket-keeps-no-line");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);
    session.tool_result(PLANTED);
    session.model_replies();

    let msg_id = ada.web.own(AFTER_PLANTED);
    session.wait_lines(1);
    session.model_replies();
    shows_read_once(ada.machine(), msg_id);

    let planted = ada.machine().files_holding(PLANTED);
    assert!(planted.is_empty(), "{PLANTED} kept in {planted:?}");
    let state = ada.machine().state_dir();
    let outside: Vec<_> = (ada.machine().files_holding(session.token()).into_iter())
        .filter(|file| !file.starts_with(&state))
        .collect();
    assert!(
        outside.is_empty(),
        "the token outside the local state: {outside:?}"
    );
}
