//! The helper reads the rollout while the session runs, and its `Stop` hook reads it as the
//! fallback. A helper that dies after `inbox` gave a message leaves its confirmation to the next
//! run: a restarted helper or the `Stop` hook marks it once, and `read` goes once.

use crate::ada::{Ada, NEVER, QUIET, assert_gives, assert_gives_none, no_read_for};
use crate::codex::Event;
use crate::receipts::shows_read_once;
use crate::solo::WEB;

/// What api says to web; it is nowhere else.
const BEFORE_STOP: &str = "codex fallback test: api asks web just before the helper dies";

/// web's session starts in a Codex process, and its turn starts. api sends web a message; web's
/// `PostToolUse` gives the signal, and the model calls `inbox`, which gives the message. The
/// helper is killed: no `read` over [`QUIET`]. With no helper running, `inbox`'s result goes into
/// the rollout and the model replies. web's `Stop` prints nothing as the turn ends: the person's
/// `skald recv` shows `read` of the message once. A helper starts again: no `read` again over
/// [`QUIET`], and `inbox` gives nothing.
pub fn with_the_helper_killed_stop_confirms_it_and_read_goes_once() {
    let mut ada = Ada::new(4_920_521_001, "ada-codex-confirms-by-stop", NEVER);
    let mut web = ada.start(WEB);
    ada.busy(&web);

    let msg_id = ada.codex.web.own(BEFORE_STOP);
    ada.gives(&web, Event::PostToolUse);
    let (call, shown) = web.inbox();
    assert_gives(&shown, BEFORE_STOP, msg_id);
    ada.kill_helper();
    no_read_for(
        ada.machine(),
        msg_id,
        QUIET,
        "the rollout shows nothing of it yet",
    );

    web.rollout.mcp_result(&call, &shown);
    web.rollout.model_replies();
    ada.silent(&web, Event::Stop, "the turn's end, with no helper");
    shows_read_once(ada.machine(), msg_id);

    let _helper = ada.restart_helper();
    no_read_for(
        ada.machine(),
        msg_id,
        QUIET,
        "with the helper started again",
    );
    let after = web.mcp.inbox();
    assert_gives_none(&after, "inbox once it was confirmed");
}

/// What api says to web; it is nowhere else.
const BEFORE_RESTART: &str = "codex fallback test: api asks web while the helper restarts";

/// web's session starts in a Codex process, and its turn starts. api sends web a message; web's
/// `PostToolUse` gives the signal, and the model calls `inbox`, which gives the message. The
/// helper is killed: no `read` over [`QUIET`]. A helper starts again; then `inbox`'s result goes
/// into the rollout and the model replies: with no hook run, the person's `skald recv` shows
/// `read` of the message once.
pub fn a_restarted_helper_confirms_it_from_the_rollout_while_the_session_runs() {
    let mut ada = Ada::new(4_920_522_001, "ada-codex-confirms-after-restart", NEVER);
    let mut web = ada.start(WEB);
    ada.busy(&web);

    let msg_id = ada.codex.web.own(BEFORE_RESTART);
    ada.gives(&web, Event::PostToolUse);
    let (call, shown) = web.inbox();
    assert_gives(&shown, BEFORE_RESTART, msg_id);
    ada.kill_helper();
    no_read_for(
        ada.machine(),
        msg_id,
        QUIET,
        "the rollout shows nothing of it yet",
    );

    let _helper = ada.restart_helper();
    web.rollout.mcp_result(&call, &shown);
    web.rollout.model_replies();
    shows_read_once(ada.machine(), msg_id);
}
