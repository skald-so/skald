//! The socket's receipts `held`, `expired`, `refused`, `dropped` and `denied` confirm nothing: the
//! message waits for the hooks and `inbox`, which hand it over. A held message the session later
//! releases is not handed over again.

use crate::hooks::{Event, assert_own_line};
use crate::lines::agent_lines;
use crate::receipts::shows_read_once;
use crate::registered::{Ada, QUIET, lines_stay};
use crate::solo::WEB;
use crate::standin::{FAILING, RELEASED, Standin, Take};

/// web's session starts. For each failing status in turn, the session answers what comes with a
/// receipt of it; api sends web a message, which the socket gets and answers. web's `PostToolUse`
/// then gives it, as its line from api at `own`, and the person's `skald recv` shows `read` of it
/// once. The socket never gets it again.
#[test]
fn each_failing_receipt_leaves_the_message_to_the_hooks() {
    let ada = Ada::new(4_919_421_001, "ada-socket-failing-receipts");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);

    for (sent, status) in FAILING.into_iter().enumerate() {
        session.take(Take::Receipt(status));
        let text = format!("receipt test: api asks web, which answers {status}");
        let msg_id = ada.web.own(&text);
        session.wait_lines(sent + 1);
        session.wait_receipts(sent + 1);

        let context = ada.hook_gives(WEB, &session);

        assert_own_line(&context, &text, msg_id, &ada.web.api());
        shows_read_once(ada.machine(), msg_id);
        lines_stay(&session, &text, 1, QUIET);
    }
}

/// What api says to web; it is nowhere else.
const HELD_SAYS: &str = "receipt test: api asks web, which holds it, then lets it in";

/// web's session starts and holds what comes. api sends web a message; the socket gets it and
/// answers `held`. The session releases it: it answers `delivered`, takes it into the
/// conversation, and the model replies. The person's `skald recv` shows `read` of it once; web's
/// `PostToolUse` prints nothing, its `inbox` shows no message, and the socket never gets it again.
#[test]
fn a_held_message_the_session_releases_is_not_handed_over_again() {
    let mut ada = Ada::new(4_919_422_001, "ada-socket-held-released");
    let session = Standin::new(Take::Receipt("held"));
    ada.starts(WEB, &session);

    let msg_id = ada.web.own(HELD_SAYS);
    session.wait_lines(1);
    session.wait_receipts(1);
    session.release();
    session.model_replies();

    shows_read_once(ada.machine(), msg_id);
    assert_eq!(
        session.wait_receipts(2)[1].0,
        RELEASED,
        "the release's receipt"
    );
    (ada.hook(WEB, &session, Event::PostToolUse)).assert_silent("PostToolUse after the release");
    let shown = ada.web.session.inbox();
    assert!(agent_lines(&shown).is_empty(), "inbox: {shown}");
    lines_stay(&session, HELD_SAYS, 1, QUIET);
}
