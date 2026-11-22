//! An agent's output never shows a recorded prompt: neither its `skald recv` nor `inbox`, nor its
//! session's hooks.

use crate::prompts::{Call, Prompted, agent_sees_nothing, assert_line_of, person_sees};
use crate::registered::Ada;
use crate::solo::{API, WEB};
use crate::standin::{Standin, Take};

/// web's session stops on its prompt: `PermissionRequest` and `Notification`. web's `skald recv`,
/// with `--json` too, and its `inbox`, and api's `skald recv`, show nothing of it. The person
/// allows the call: web's `PostToolUse` prints nothing. The person's `skald recv` then shows its
/// line.
#[test]
fn an_agent_sees_nothing_of_a_permission_prompt() {
    let mut ada = Ada::new(4_919_706_001, "ada-waiting-agents");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);
    let call = Call::new("agents");
    let web = Prompted {
        ada: &ada,
        agent: WEB,
        session: &session,
    };
    web.requests("turn-1", &call);
    web.notifies("turn-1");

    agent_sees_nothing(&mut ada, WEB, &session, &call);
    agent_sees_nothing(&mut ada, API, &session, &call);
    let web = Prompted {
        ada: &ada,
        agent: WEB,
        session: &session,
    };
    web.runs("turn-1", &call);

    let lines = person_sees(ada.machine(), 1);
    assert_line_of(&lines[0], WEB, &call);
}
