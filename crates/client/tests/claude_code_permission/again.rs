//! A prompt ends with the session's next `PostToolUse`: the `Notification` hook alone then records
//! the turn's next prompt.

use crate::prompts::{Call, Prompted, assert_line_of, person_sees};
use crate::registered::Ada;
use crate::solo::WEB;
use crate::standin::{Standin, Take};

/// Twice in one turn, web's session stops on its prompt, told by `Notification` alone, and the
/// person allows the call, which runs: `PostToolUse`. The person's `skald recv` shows two lines,
/// one per prompt.
#[test]
fn a_notification_after_the_call_ran_records_the_next_prompt_of_the_turn() {
    let ada = Ada::new(4_919_708_001, "ada-waiting-again");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);
    let calls = [Call::new("again-first"), Call::new("again-second")];
    for call in &calls {
        let prompted = Prompted {
            ada: &ada,
            agent: WEB,
            session: &session,
        };
        prompted.notifies("turn-1");
        prompted.runs("turn-1", call);
    }

    let lines = person_sees(ada.machine(), calls.len());
    for (line, call) in lines
        .iter()
        .flat_map(|line| calls.iter().map(move |call| (line, call)))
    {
        assert_line_of(line, WEB, call);
    }
}
