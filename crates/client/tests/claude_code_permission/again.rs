//! A prompt ends with the session's next `PostToolUse`: the `Notification` hook alone then records
//! the turn's next prompt.

use crate::prompts::{Call, Prompted, assert_line_of, person_sees};
use crate::registered::Ada;
use crate::solo::WEB;
use crate::standin::{Standin, Take};

/// web's session stops on its prompt, told by `Notification` alone; the person allows the call,
/// which runs: `PostToolUse`. In the same turn it stops on a prompt for another call, again told by
/// `Notification` alone. The person's `skald recv` shows two lines, one per prompt.
#[test]
fn a_notification_after_the_call_ran_records_the_next_prompt_of_the_turn() {
    let ada = Ada::new(4_919_708_001, "ada-waiting-again");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);
    let web = Prompted {
        ada: &ada,
        agent: WEB,
        session: &session,
    };
    let (first, second) = (Call::new("again-first"), Call::new("again-second"));

    web.notifies("turn-1");
    web.runs("turn-1", &first);
    web.notifies("turn-1");

    let lines = person_sees(ada.machine(), 2);
    for line in &lines {
        assert_line_of(line, WEB, &first);
        assert_line_of(line, WEB, &second);
    }
}
