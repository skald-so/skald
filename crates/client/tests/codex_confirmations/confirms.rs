//! `read` goes once the session's rollout shows that the model got the message: the entry of the
//! `inbox` result that gave it, followed by the model's own entry, not by an API error. Nothing
//! else in the rollout confirms it, and the local state keeps nothing of the rollout.

use crate::ada::{Ada, NEVER, QUIET, assert_gives, assert_gives_none, no_read_for};
use crate::codex::{Event, assert_bare};
use crate::receipts::shows_read_once;
use crate::solo::WEB;

/// What api says to web; it is nowhere else.
const CONFIRMED_SAYS: &str = "codex confirm test: api asks web to bump the version";

/// web's session starts in a Codex process, and its turn starts. api sends web a message; web's
/// `PostToolUse` gives the signal, and the model calls `inbox`, which gives the message. Over
/// [`QUIET`], the person's `skald recv` shows no `read` of it. `inbox`'s result goes into the
/// rollout: still none over [`QUIET`]. The call to the model fails with an API error: still none
/// over [`QUIET`]. Then the model replies: `read` of it once, with no hook run meanwhile.
pub fn read_follows_once_the_model_s_own_entry_follows_inbox_s_result_in_the_rollout() {
    let mut ada = Ada::new(4_920_501_001, "ada-codex-confirms", NEVER);
    let mut web = ada.start(WEB);
    ada.busy(&web);

    let msg_id = ada.codex.web.own(CONFIRMED_SAYS);
    let signal = ada.gives(&web, Event::PostToolUse);
    assert_bare(&signal, &[CONFIRMED_SAYS], &[msg_id], &[]);
    let (call, shown) = web.inbox();
    assert_gives(&shown, CONFIRMED_SAYS, msg_id);

    let machine = ada.machine();
    no_read_for(
        machine,
        msg_id,
        QUIET,
        "inbox gave it; the rollout shows only its call",
    );
    web.rollout.mcp_result(&call, &shown);
    no_read_for(
        machine,
        msg_id,
        QUIET,
        "inbox's result, nothing of the model's after it",
    );
    web.rollout.api_error();
    no_read_for(machine, msg_id, QUIET, "an API error after inbox's result");
    web.rollout.model_replies();
    shows_read_once(machine, msg_id);
}

/// What api says to web; it is nowhere else.
const READ_ELSEWHERE: &str = "codex confirm test: api asks web, its line read from a file";

/// web's session starts in a Codex process, and its turn starts. api sends web a message; once
/// `PostToolUse` gave the signal, a call of `inbox` that the rollout does not show gives it. The
/// model runs a command whose output holds the message's `msg_id` and the text `inbox` gave, as the
/// agent reading a file would, and replies: no `read` over [`QUIET`]. The model calls `inbox` again,
/// which gives nothing; that result goes into the rollout, and the model replies: still no `read`
/// over [`QUIET`].
pub fn a_reference_in_another_tool_s_result_confirms_nothing() {
    let mut ada = Ada::new(4_920_502_001, "ada-codex-confirms-not-elsewhere", NEVER);
    let mut web = ada.start(WEB);
    ada.busy(&web);

    let msg_id = ada.codex.web.own(READ_ELSEWHERE);
    ada.gives(&web, Event::PostToolUse);
    let shown = web.mcp.inbox();
    assert_gives(&shown, READ_ELSEWHERE, msg_id);
    let read = web.rollout.calls_command("cat notes/inbox.txt");
    web.rollout
        .command_output(&read, &format!("{msg_id}\n{shown}"));
    web.rollout.model_replies();

    let machine = ada.machine();
    no_read_for(
        machine,
        msg_id,
        QUIET,
        "its reference in a command's output",
    );
    let empty = web.inbox_in_rollout();
    assert_gives_none(&empty, "inbox while the message waits for its confirmation");
    web.rollout.model_replies();
    no_read_for(
        machine,
        msg_id,
        QUIET,
        "an inbox result that did not give it",
    );
}
