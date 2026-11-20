//! What `skald reply` and the MCP reply tool say of a reply the server refuses over its device's
//! send rate: the problem `rate-limited` and what it means, as `skald send` says it
//! (`sending/rate.rs`). The front gives the refusal in place of the server's `stored`.

use serde_json::json;

use crate::agents::{Agents, CLAUDE};
use crate::client::{CLIENT, MESSAGE_FIELD, REPLY_TOOL, TEXT_FIELD, session_in};
use crate::front::Answers;
use crate::lines::agent_line_of;
use crate::rate_said::{RATE_LIMITED, assert_says_rate_limited};
use crate::replying::{BOB_CLAUDE, Writes, agent_of, bob_writes, handles, replies};
use crate::world::{ACME, AGENT};

/// What Bob's claude asks Ada's claude, and what Ada's claude answers; the answers are refused.
const ASKED: [&str; 2] = [
    "reply rate test: bob's claude asks for the build number",
    "reply rate test: bob's claude asks for the release date",
];
const ANSWERS: [&str; 2] = [
    "reply rate test: the build is number forty-two",
    "reply rate test: the release is on friday",
];

/// Bob's claude asks Ada's claude two things, which claude's `skald recv` shows. The front refuses
/// every envelope with `rate-limited`. claude's `skald reply` to the first, by its `#<ref>`, fails
/// and says the problem and what it means; so does the reply tool's result for the second, by its
/// `msg_id`, which is not `sent`.
#[test]
fn a_reply_over_the_send_rate_says_what_the_refusal_means() {
    let mut agents = Agents::new(
        [4_216_062_001, 4_216_062_002],
        ["ada-replies-over-send-rate", "bob-asks-past-the-rate"],
    );
    let ada_claude = agents.address(CLAUDE);
    let bob_claude = agent_of(&handles(&agents.talk).0, ACME, BOB_CLAUDE);
    let mut asked = Vec::new();
    for (counter, text) in (1..).zip(ASKED) {
        let writes = Writes::new(&bob_claude, Some(&ada_claude), counter, text).hops(1);
        let at = agents.talk.at();
        asked.push(bob_writes(&mut agents.talk, at, &writes));
    }
    let shown = agents.run_as(CLAUDE).shown();
    let first = format!("#{}", agent_line_of(&shown, ASKED[0]).reference);
    agents
        .talk
        .front
        .answer(Answers::Refuse(RATE_LIMITED.to_owned()));

    let replied = replies(&agents, Some(CLAUDE), &first, ANSWERS[0]);

    assert!(!replied.success, "{replied:?}");
    assert_says_rate_limited(&replied.shown(), "skald reply");

    let machine = &agents.talk.ada.machine;
    let mut session = session_in(machine, &agents.folder, &[AGENT, CLAUDE], CLIENT);
    let called = session.call(
        REPLY_TOOL,
        json!({ MESSAGE_FIELD: asked[1].to_string(), TEXT_FIELD: ANSWERS[1] }),
    );

    assert_says_rate_limited(&called.text, "the reply tool");
    assert!(
        !called.text.trim().to_lowercase().starts_with("sent"),
        "the reply tool says sent: {called:?}"
    );
}
