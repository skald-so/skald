//! A message from Ada's second machine gets its `read` there once web's session confirms it: the
//! helper takes in for the receipt, whether its own reading of the rollout or the `Stop` hook's
//! confirmed it.

use crate::ada::{Ada, NEVER, QUIET, assert_gives, no_read_for};
use crate::codex::Event;
use crate::receipts::shows_read_once;
use crate::solo::WEB;

/// What the second machine's agent review says to web; neither is anywhere else.
const BY_HELPER: &str = "codex confirm test: the laptop asks web, which the helper sees read";
const BY_STOP: &str = "codex confirm test: the laptop asks web, which Stop sees read";

/// Ada's second machine joins `clienta`; web's session starts in a Codex process, and its turn
/// starts. The second machine's agent review sends web a message; web's `PostToolUse` gives the
/// signal, and the model calls `inbox`, which gives the message: the second machine's `skald recv`
/// shows no `read` over [`QUIET`]. `inbox`'s result goes into the rollout and the model replies:
/// with no hook run, the second machine's `skald recv` shows `read` of the message once.
pub fn the_helper_s_confirmation_sends_read_to_the_second_machine_with_no_hook_run() {
    let mut ada = Ada::new(4_920_551_001, "ada-codex-confirms-to-laptop", NEVER);
    let laptop = ada.laptop();
    let mut web = ada.start(WEB);
    ada.busy(&web);

    let msg_id = laptop.sends_web(&ada, BY_HELPER);
    ada.gives(&web, Event::PostToolUse);
    let (call, shown) = web.inbox();
    assert_gives(&shown, BY_HELPER, msg_id);
    no_read_for(
        laptop.machine(),
        msg_id,
        QUIET,
        "the rollout shows nothing of it yet",
    );

    web.rollout.mcp_result(&call, &shown);
    web.rollout.model_replies();
    shows_read_once(laptop.machine(), msg_id);
}

/// As [`the_helper_s_confirmation_sends_read_to_the_second_machine_with_no_hook_run`], but the
/// helper pauses before `inbox`'s result goes into the rollout and the model replies, and web's
/// `Stop` prints nothing as the turn ends. The helper runs on: with no hook run, the second
/// machine's `skald recv` shows `read` of the message once.
pub fn a_confirmation_by_stop_sends_read_to_the_second_machine_once_the_helper_runs_on() {
    let mut ada = Ada::new(4_920_552_001, "ada-codex-stop-confirms-to-laptop", NEVER);
    let laptop = ada.laptop();
    let mut web = ada.start(WEB);
    ada.busy(&web);

    let msg_id = laptop.sends_web(&ada, BY_STOP);
    ada.gives(&web, Event::PostToolUse);
    let (call, shown) = web.inbox();
    assert_gives(&shown, BY_STOP, msg_id);
    no_read_for(
        laptop.machine(),
        msg_id,
        QUIET,
        "the rollout shows nothing of it yet",
    );

    let paused = ada.pause_helper();
    web.rollout.mcp_result(&call, &shown);
    web.rollout.model_replies();
    ada.silent(&web, Event::Stop, "the turn's end, with the helper paused");
    drop(paused);
    shows_read_once(laptop.machine(), msg_id);
}
