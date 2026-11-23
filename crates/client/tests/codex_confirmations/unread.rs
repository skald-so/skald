//! A session whose session file the client does not read, such as a rollout it cannot parse, gets
//! `read` by construction, at the write, as an MCP client without Skald's hooks does; a session
//! whose rollout it reads gets it only once the rollout confirms it.

use crate::ada::{Ada, NEVER, QUIET, assert_gives, no_read_for};
use crate::codex::Event;
use crate::receipts::shows_read_once;
use crate::solo::{API, WEB};

/// What api says to web, and web to api; neither is anywhere else.
const TO_WEB: &str = "codex unread test: api asks web, whose rollout is unreadable, to ship";
const TO_API: &str = "codex unread test: web asks api, whose rollout is read, to review";

/// web's session starts in a Codex process with a rollout Skald cannot parse, and api's with one it
/// can; both turns start. api sends web a message; web's `PostToolUse` gives the signal and web's
/// `inbox` gives the message: the person's `skald recv` shows `read` of it once. web sends api a
/// message with the send tool; api's `PostToolUse` gives the signal and api's `inbox` gives the
/// message, as api's rollout shows: no `read` of it over [`QUIET`]. api's model replies: `read` of
/// it once.
pub fn an_unparsable_rollout_gives_read_at_the_write_and_a_readable_one_holds_it_back() {
    let mut ada = Ada::new(4_920_541_001, "ada-codex-unparsable-rollout", NEVER);
    let mut web = ada.start_unparsable(WEB);
    let mut api = ada.start(API);
    ada.busy(&web);
    ada.busy(&api);

    let to_web = ada.codex.web.own(TO_WEB);
    ada.gives(&web, Event::PostToolUse);
    let shown = web.mcp.inbox();
    assert_gives(&shown, TO_WEB, to_web);
    shows_read_once(ada.machine(), to_web);

    let solo = &ada.codex.web.solo;
    let to_api = solo.sends(&mut web.mcp, &solo.short(API), TO_API);
    ada.gives(&api, Event::PostToolUse);
    let shown = api.inbox_in_rollout();
    assert_gives(&shown, TO_API, to_api);
    no_read_for(
        ada.machine(),
        to_api,
        QUIET,
        "api's rollout shows no reply yet",
    );
    api.rollout.model_replies();
    shows_read_once(ada.machine(), to_api);
}
