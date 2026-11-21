//! What the person's `skald recv` shows of a recorded prompt: one line that names the agent, and
//! with `--json` its agent, session and time, never anything of the prompt.

use serde_json::Value;

use crate::prompts::{
    Call, Prompted, RECORD_TYPE, assert_line_of, person_sees, person_sees_json, unix_now,
};
use crate::registered::Ada;
use crate::solo::{API, WEB};
use crate::standin::{Standin, Take};

/// web's session starts, and stops on its permission prompt for a call: its `PermissionRequest`
/// hook prints nothing. Within seconds the person's `skald recv` shows one line that names web and
/// holds nothing of the call; its next run shows none.
#[test]
fn a_permission_request_shows_the_person_one_line_without_the_prompt() {
    let ada = Ada::new(4_919_701_001, "ada-waiting-line");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);
    let web = Prompted {
        ada: &ada,
        agent: WEB,
        session: &session,
    };
    let call = Call::new("line");

    web.requests("turn-1", &call);

    let lines = person_sees(ada.machine(), 1);
    assert_line_of(&lines[0], WEB, &call);
}

/// A session of web and one of api start, and each stops on its permission prompt. Within seconds
/// the person's `skald recv --json` shows two recorded prompts: one names web and its session, the
/// other api and its session, each with the time of its hook, and neither holds anything of its
/// call. Its next run shows none.
#[test]
fn the_person_s_json_names_the_agent_its_session_and_the_time() {
    let ada = Ada::new(4_919_702_001, "ada-waiting-json");
    let sessions = [
        (WEB, Standin::new(Take::Accept), Call::new("json-web")),
        (API, Standin::new(Take::Accept), Call::new("json-api")),
    ];
    let before = unix_now();
    for (agent, session, call) in &sessions {
        ada.starts(agent, session);
        let prompted = Prompted {
            ada: &ada,
            agent,
            session,
        };
        prompted.requests("turn-1", call);
    }
    let after = unix_now();

    let found = person_sees_json(ada.machine(), 2);
    for (agent, session, call) in &sessions {
        let of: Vec<&Value> = (found.iter())
            .filter(|record| record["session"] == session.session_id())
            .collect();
        let [record] = of.as_slice() else {
            panic!("not one record of {agent}'s session: {found:#?}");
        };
        assert_eq!(record["type"], RECORD_TYPE, "{record}");
        let named = record["agent"].as_str().unwrap_or_default();
        assert!(
            named == *agent || named == ada.web.solo.short(agent),
            "the record names {named:?}, not {agent}: {record}"
        );
        let timestamp = record["timestamp"].as_u64();
        assert!(
            timestamp.is_some_and(|at| before <= at && at <= after + 1),
            "the time is not in [{before}, {after}]: {record}"
        );
        call.assert_absent(&record.to_string(), "the record");
    }
}
