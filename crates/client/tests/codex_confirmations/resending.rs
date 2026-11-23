//! A message a path gave waits for its confirmation, and no other call gives it meanwhile;
//! unconfirmed within the timeout, it goes again by the next path, and once confirmed never again.
//! Codex's paths carry only the signal, so the timeout runs from the signal: in a busy session
//! `Stop` follows `PostToolUse`, then `codex queue` once the turn ends; in an idle one the next
//! `UserPromptSubmit` follows `codex queue`; after the last, the message waits for `inbox`.

use std::thread;
use std::time::Duration;

use skald_proto::MessageId;

use crate::ada::{Ada, Agent, QUIET, assert_gives, assert_gives_none, no_read_for};
use crate::codex::{Event, assert_bare};
use crate::receipts::{shows_no_read, shows_read_once};
use crate::solo::WEB;
use crate::standin::{Ran, assert_queued, now};

/// The resend timeout of these tests, in seconds.
const TIMEOUT: u64 = 4;

/// A wait that the timeout passes in.
const PAST: Duration = Duration::from_secs(TIMEOUT + 2);

/// A wait that the timeout does not pass in, from where it starts, with time to spare for the run
/// that starts it.
const SHORT_OF: Duration = Duration::from_secs(TIMEOUT - 2);

/// What api says to web; it is nowhere else.
const AGAIN_SAYS: &str = "codex resend test: api asks web, whose first inbox is lost, to retry";

/// web's session starts in a Codex process with a timeout of [`TIMEOUT`] seconds, and its turn
/// starts. api sends web a message; web's `PostToolUse` gives the signal, and a call of `inbox` that
/// the rollout never shows gives the message. A second `inbox` gives nothing, and `PostToolUse`
/// prints nothing. Past the timeout with no `read`, web's `Stop` gives the signal, and the next
/// `inbox`, as the rollout shows, gives the message again; its result goes to the model, which
/// replies: `read` of it once. Then `inbox` gives nothing; the continued turn's `Stop` prints
/// nothing; past the timeout again, the next turn's `UserPromptSubmit` and `PostToolUse` print
/// nothing, `inbox` gives nothing, no `codex queue` ran, and no `read` shows again.
pub fn unconfirmed_past_the_timeout_it_goes_by_the_next_path_and_once_confirmed_never_again() {
    let mut ada = Ada::new(4_920_511_001, "ada-codex-resends", TIMEOUT);
    let mut web = ada.start(WEB);
    ada.busy(&web);

    let msg_id = ada.codex.web.own(AGAIN_SAYS);
    let signal = ada.gives(&web, Event::PostToolUse);
    assert_bare(&signal, &[AGAIN_SAYS], &[msg_id], &[]);
    let lost = web.mcp.inbox();
    assert_gives(&lost, AGAIN_SAYS, msg_id);
    let second = web.mcp.inbox();
    assert_gives_none(
        &second,
        "a second inbox while it waits for its confirmation",
    );
    ada.silent(
        &web,
        Event::PostToolUse,
        "while it waits for its confirmation",
    );
    no_read_for(ada.machine(), msg_id, PAST, "unconfirmed");

    let again = ada.gives_now(&web, Event::Stop);
    assert_bare(&again, &[AGAIN_SAYS], &[msg_id], &[]);
    let shown = web.inbox_in_rollout();
    assert_gives(&shown, AGAIN_SAYS, msg_id);
    web.rollout.model_replies();
    shows_read_once(ada.machine(), msg_id);

    let after = web.mcp.inbox();
    assert_gives_none(&after, "inbox once it was confirmed");
    ada.silent(&web, Event::StopActive, "the continued turn's end");
    thread::sleep(PAST);
    ada.silent(
        &web,
        Event::UserPromptSubmit,
        "the next turn, once confirmed",
    );
    ada.silent(&web, Event::PostToolUse, "the next turn, once confirmed");
    let last = web.mcp.inbox();
    assert_gives_none(&last, "inbox past the timeout once it was confirmed");
    ada.app.assert_runs(0, "codex queue of a confirmed message");
    shows_no_read(ada.machine(), msg_id, "read again");
}

/// What follows a `codex queue` of the signal of `msg_id`, whose text is `text`, that `ran` is,
/// with no `inbox`: it is for web's thread and holds only the signal, and no `read` shows. The
/// turn it starts runs: its `UserPromptSubmit` and `Stop` print nothing. Past the timeout, the
/// person's next prompt: `UserPromptSubmit` gives the signal. Past the timeout again, the turn's
/// `PostToolUse` and `Stop` and the next prompt's `UserPromptSubmit` print nothing; over [`QUIET`]
/// more, `codex queue` ran once in all, no `read` shows, and `inbox` gives the message.
#[track_caller]
fn queued_then_user_prompt_submit_then_waits(
    (ada, web): (&mut Ada, &mut Agent),
    ran: &Ran,
    (text, msg_id): (&str, MessageId),
) {
    assert_queued(ran, (&web.thread, &ada.app), &[text], &[msg_id], &[]);
    shows_no_read(ada.machine(), msg_id, "codex queue ran");
    ada.silent(web, Event::UserPromptSubmit, "the woken turn's start");
    ada.silent(web, Event::Stop, "the woken turn's end");

    thread::sleep(PAST);
    let prompt = ada.gives_now(web, Event::UserPromptSubmit);
    assert_bare(&prompt, &[text], &[msg_id], &[]);
    thread::sleep(PAST);
    ada.silent(web, Event::PostToolUse, "after the last path");
    ada.silent(web, Event::Stop, "after the last path");
    let next = "the next prompt, after the last path";
    ada.silent(web, Event::UserPromptSubmit, next);
    thread::sleep(QUIET);
    ada.app.assert_runs(1, "codex queue after the last path");
    shows_no_read(ada.machine(), msg_id, "never confirmed");
    let shown = web.mcp.inbox();
    assert_gives(&shown, text, msg_id);
}

/// What api says to web, whose agent never calls `inbox`; it is nowhere else.
const BUSY_SAYS: &str = "codex resend test: api asks web, busy and never reading, to tag it";

/// web's session starts in a Codex process with a timeout of [`TIMEOUT`] seconds, and its turn
/// starts. api sends web a message; web's `PostToolUse` gives the signal, and no `inbox` follows.
/// Past the timeout, web's `Stop` gives the signal and continues the turn; the continued turn's
/// `Stop` prints nothing and ends it. The app's `codex` does not run until the timeout nearly
/// passed again, then runs `codex queue` once; then as
/// [`queued_then_user_prompt_submit_then_waits`].
pub fn busy_the_signal_goes_by_stop_then_codex_queue_then_user_prompt_submit_then_waits() {
    let mut ada = Ada::new(4_920_512_001, "ada-codex-resends-busy", TIMEOUT);
    let mut web = ada.start(WEB);
    ada.busy(&web);

    let msg_id = ada.codex.web.own(BUSY_SAYS);
    let signal = ada.gives(&web, Event::PostToolUse);
    assert_bare(&signal, &[BUSY_SAYS], &[msg_id], &[]);
    thread::sleep(PAST);
    let stop = ada.gives_now(&web, Event::Stop);
    assert_bare(&stop, &[BUSY_SAYS], &[msg_id], &[]);
    let stopped = now();
    let end = "the continued turn's end, with no inbox";
    ada.silent(&web, Event::StopActive, end);

    thread::sleep(SHORT_OF.saturating_sub(stopped.elapsed().unwrap_or_default()));
    let early = "codex queue before the timeout from Stop's signal";
    ada.app.assert_runs(0, early);
    let ran = ada.app.await_runs(1);
    queued_then_user_prompt_submit_then_waits((&mut ada, &mut web), &ran[0], (BUSY_SAYS, msg_id));
}

/// What api says to web, idle, whose agent never calls `inbox`; it is nowhere else.
const IDLE_SAYS: &str = "codex resend test: api asks web, idle and never reading, to prune it";

/// web's session starts in a Codex process with a timeout of [`TIMEOUT`] seconds: it is idle. api
/// sends web a message: the app's `codex` runs `codex queue` once; then as
/// [`queued_then_user_prompt_submit_then_waits`].
pub fn idle_the_signal_goes_by_codex_queue_then_user_prompt_submit_then_waits() {
    let mut ada = Ada::new(4_920_513_001, "ada-codex-resends-idle", TIMEOUT);
    let mut web = ada.start(WEB);

    let msg_id = ada.codex.web.own(IDLE_SAYS);
    let ran = ada.app.await_runs(1);
    queued_then_user_prompt_submit_then_waits((&mut ada, &mut web), &ran[0], (IDLE_SAYS, msg_id));
}
