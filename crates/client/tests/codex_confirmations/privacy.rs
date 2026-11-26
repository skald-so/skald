//! The client reads only the registered session's rollout, only the references in the entries
//! that gave Skald's messages, and keeps no line of it.

use crate::ada::{Ada, NEVER, QUIET, assert_gives, no_read_for};
use crate::codex::Event;
use crate::receipts::shows_read_once;
use crate::solo::WEB;

/// What the session's rollout holds that no message does.
const PLANTED: &str = "planted-rollout-marker-4c1e9a7f02d85b36";

/// What api says to web; it is nowhere else.
const AFTER_PLANTED: &str = "codex privacy test: api asks web after the planted lines";

/// web's session starts in a Codex process, its turn starts with a prompt that holds [`PLANTED`],
/// and a command's output that holds it goes to the model, which replies with it. api sends web a
/// message; web's `PostToolUse` gives the signal, and the model calls `inbox`, which gives the
/// message: no `read` over [`QUIET`]. `inbox`'s result goes into the rollout and the model replies,
/// naming [`PLANTED`]: the person's `skald recv` shows `read` of the message once. Then no file in
/// the machine's state directory holds [`PLANTED`].
pub fn the_state_directory_keeps_nothing_of_the_rollout() {
    let mut ada = Ada::new(4_920_531_001, "ada-codex-keeps-no-line", NEVER);
    let mut web = ada.start(WEB);
    web.rollout
        .turn_starts(&format!("Check the deploy key {PLANTED}, then reply."));
    ada.busy(&web);
    let read = web.rollout.calls_command("cat deploy/key.txt");
    web.rollout.command_output(&read, PLANTED);
    web.rollout.model_says(&format!("The key is {PLANTED}."));

    let msg_id = ada.codex.web.own(AFTER_PLANTED);
    ada.gives(&web, Event::PostToolUse);
    let (call, shown) = web.inbox();
    assert_gives(&shown, AFTER_PLANTED, msg_id);
    no_read_for(
        ada.machine(),
        msg_id,
        QUIET,
        "the rollout shows nothing of it yet",
    );
    web.rollout.mcp_result(&call, &shown);
    web.rollout.model_says(&format!("Done; {PLANTED} stays."));
    shows_read_once(ada.machine(), msg_id);

    let state = ada.machine().state_dir();
    let kept: Vec<_> = (ada.machine().files_holding(PLANTED).into_iter())
        .filter(|file| file.starts_with(&state))
        .collect();
    assert!(kept.is_empty(), "{PLANTED} kept in {kept:?}");
}
