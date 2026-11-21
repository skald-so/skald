//! `inline` for `peer` by the hooks of a Claude Code session that refuses cross-session messages:
//! a peer's message reaches the model as its text, labelled as data from its sender after the
//! warning line, with no `inbox` call, and `read` follows; `own` messages as before. Over the
//! socket: `modes.rs`.

use serde_json::json;

use crate::delivery::{INLINE, assert_inline, set_text};
use crate::hooks::Bob;
use crate::receipts::shows_read_once;
use crate::senders::PEER;
use crate::socket_claude::Options;
use crate::socket_fallbacks::idle_web;
use crate::socket_model::{Model, Reply, conversation, last_user_text};
use crate::socket_sessions::{after_a_tool, assert_own_by_hook};
use crate::solo::{API, Solo, WEB};

/// What review and api say to web, and the person's prompt; none is anywhere else.
const BY_HOOK: &str = "e2e inline test: review asks web, inline, to raise the timeout";
const OWN_BY_HOOK: &str = "e2e inline test: api asks web, beside inline, to lower the retries";
const AGAIN: &str = "e2e inline test: carry on";

/// Bob joins `clienta`; a session of web that refuses cross-session messages, `crossSessionInbound`
/// `refuse`, runs as the apps run it, idle after the prompt; the person sets `peer`'s text in the
/// default channel to `inline`. review and api each send web a message; then the person gives a
/// prompt, on which the stub runs `sleep 1` until a turn holds both: after a tool's result, that
/// turn holds review's line at `peer`, after a line that names data, and api's at `own`. Bob's
/// machine shows `read` of review's once.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn inline_peer_text_reaches_a_busy_session_by_its_hooks_without_inbox() {
    let solo = Solo::new(5_207_502_001, "ada-inline-e2e-hooks");
    let bob = Bob::join(&solo, 5_207_502_002, "bob-inline-e2e-hooks");
    let model = Model::start(|body| {
        let all = conversation(body);
        if all.contains(BY_HOOK) && all.contains(OWN_BY_HOOK) {
            Reply::Done
        } else if last_user_text(body).contains(AGAIN) || after_a_tool(body) {
            Reply::Bash("sleep 1")
        } else {
            Reply::Done
        }
    });
    let refusing = [("crossSessionInbound".to_owned(), json!("refuse"))];
    let options = Options {
        settings: refusing.into_iter().collect(),
        ..Options::default()
    };
    let Some(mut session) = idle_web(&solo, &model, options) else {
        return;
    };
    set_text(&solo, solo.default_channel(), PEER, INLINE);

    let asked = bob.sends(&solo, WEB, BY_HOOK);
    let own = solo.api_sends_web(OWN_BY_HOOK);
    session.prompt(AGAIN);

    let turn = model.wait_turn_holding(BY_HOOK, "review's message by a hook");
    assert!(after_a_tool(&turn), "not after a tool: {turn}");
    assert_inline(&last_user_text(&turn), BY_HOOK, asked, &bob);
    let turn = model.wait_turn_holding(OWN_BY_HOOK, "api's message by a hook");
    assert_own_by_hook(&turn, OWN_BY_HOOK, own, &solo.short(API));
    shows_read_once(&bob.person.machine, asked);
    session.end();
    model.assert_no_foreign_credential();
}
