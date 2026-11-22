//! Each prompt is recorded once, whichever paths tell: the `PermissionRequest` hook at once, the
//! `Notification` hook of type `permission_prompt` about 6 s later or alone. Another notification
//! records nothing, and the hooks print nothing even while messages wait for the agent.

use std::thread;

use crate::hooks::assert_own_line;
use crate::prompts::{Call, Prompted, QUIET, assert_line_of, person_sees, person_sees_none};
use crate::registered::Ada;
use crate::solo::WEB;
use crate::standin::{Standin, Take};

/// What api says to web while web's session waits on its prompt; it is nowhere else.
const WHILE_PROMPTED: &str = "prompt test: api asks web while its session waits on a prompt";

/// web's session starts and answers what its socket gets with `refused`, so api's message to web
/// waits for the hooks. web's session stops on its prompt: `PermissionRequest`, then
/// `Notification`, each printing nothing; the person's `skald recv` shows one line. In the next
/// turn it stops on a prompt again: `PermissionRequest`, then the person's `skald recv` shows one
/// line, then `Notification`, after which it shows none. web's `PostToolUse` then gives api's
/// message, as its line from api at `own`.
#[test]
fn a_prompt_both_hooks_tell_is_recorded_once() {
    let ada = Ada::new(4_919_703_001, "ada-waiting-once");
    let session = Standin::new(Take::Receipt("refused"));
    ada.starts(WEB, &session);
    let web = Prompted {
        ada: &ada,
        agent: WEB,
        session: &session,
    };
    let msg_id = ada.web.own(WHILE_PROMPTED);
    session.wait_lines(1);
    session.wait_receipts(1);

    let first = Call::new("once-first");
    web.requests("turn-1", &first);
    web.notifies("turn-1");
    let lines = person_sees(ada.machine(), 1);
    assert_line_of(&lines[0], WEB, &first);

    let second = Call::new("once-second");
    web.requests("turn-2", &second);
    let lines = person_sees(ada.machine(), 1);
    assert_line_of(&lines[0], WEB, &second);
    web.notifies("turn-2");
    thread::sleep(QUIET);
    person_sees_none(ada.machine(), "after the notification of a prompt shown");

    let context = ada.hook_gives(WEB, &session);
    assert_own_line(&context, WHILE_PROMPTED, msg_id, &ada.web.api());
}

/// web's session stops on its prompt, and the person allows the call, which runs: `PostToolUse`.
/// In the same turn it stops on a prompt for another call: `PermissionRequest` and `Notification`.
/// The person's `skald recv` shows two lines, one per prompt.
#[test]
fn each_prompt_of_a_turn_is_recorded() {
    let ada = Ada::new(4_919_704_001, "ada-waiting-each");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);
    let web = Prompted {
        ada: &ada,
        agent: WEB,
        session: &session,
    };
    let (first, second) = (Call::new("each-first"), Call::new("each-second"));

    web.requests("turn-1", &first);
    web.runs("turn-1", &first);
    web.requests("turn-1", &second);
    web.notifies("turn-1");

    let lines = person_sees(ada.machine(), 2);
    for line in &lines {
        assert_line_of(line, WEB, &first);
        assert_line_of(line, WEB, &second);
    }
}

/// web's session tells, by `Notification`, that it waits for the person's prompt, `idle_prompt`:
/// nothing printed, and the person's `skald recv` shows no prompt. Its next prompt is told by
/// `Notification` of type `permission_prompt` alone, with no `PermissionRequest` hook: nothing
/// printed, and the person's `skald recv` shows one line.
#[test]
fn a_permission_notification_alone_is_recorded_and_no_other_is() {
    let ada = Ada::new(4_919_705_001, "ada-waiting-notice");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);
    let web = Prompted {
        ada: &ada,
        agent: WEB,
        session: &session,
    };

    web.idles("turn-1");
    thread::sleep(QUIET);
    person_sees_none(ada.machine(), "after idle_prompt");

    web.notifies("turn-2");
    let lines = person_sees(ada.machine(), 1);
    assert_line_of(&lines[0], WEB, &Call::new("notice"));
}
