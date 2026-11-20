//! A message for an agent in `hold` reaches no `skald recv`, `skald recv --wait` or `inbox` of its
//! running MCP session until the person accepts it; its sender sees `held`, then `read`, or
//! `refused` with the person's reason, and a refused one never reaches the agent.

use skald_proto::{MessageId, ReceiptStatus};

use crate::agents::{CLAUDE, assert_not_handed};
use crate::client::{CLIENT, session_in};
use crate::held::{
    Decision, HELD, a_agent_says_once, accepted_then_read, c_reads, decide, holding, reason_of,
    says_none_waiting, says_waiting, shows_not, shows_once,
};
use crate::inbound::REFUSED;
use crate::lines::lines_referring_to;
use crate::receipts::{READ, Roles, assert_shows_once, shows_read_once, waiting};
use crate::senders::{LAPTOP, LAPTOP_AGENT, holds_word};
use crate::waits::assert_keeps_waiting;
use crate::world::AGENT;

/// What the person types when refusing.
const REASON: &str = "not before the release review";

/// Once A's agent's message `msg_id` to claude is held: B's person's `skald recv` says one message
/// waits; C reads B's `held` receipt of it; A's person's runs show `held` of it once, and A's agent
/// says it once.
#[track_caller]
fn a_sees_held(roles: &mut Roles, msg_id: MessageId) {
    says_waiting(roles.b(), 1);
    let to = roles.full(LAPTOP_AGENT);
    c_reads(roles, msg_id, ReceiptStatus::Held, &to);
    shows_once(roles.a(), msg_id, HELD);
    a_agent_says_once(roles, msg_id, HELD);
}

/// Fails unless `shown`, what the person's `skald held` showed, lists A's agent's message `msg_id`
/// with `text` once: with claude, the channel's ID, A's agent, A's device and its trust level
/// `own`, and its `#<ref>`.
#[track_caller]
fn assert_listed(roles: &Roles, shown: &str, msg_id: MessageId, text: &str) {
    assert_eq!(shown.matches(text).count(), 1, "{text:?} not once: {shown}");
    let channel = roles.channel().to_string();
    for part in [CLAUDE, channel.as_str(), LAPTOP_AGENT, LAPTOP] {
        assert!(shown.contains(part), "{text:?} without {part}: {shown}");
    }
    assert!(
        shown.lines().any(|line| holds_word(line, "own")),
        "no trust level own: {shown}"
    );
    let referring = lines_referring_to(shown, msg_id);
    assert!(!referring.is_empty(), "no #<ref> of {msg_id}: {shown}");
}

/// claude's MCP session runs; B's person sets claude to `hold`. A's agent sends a message `--to`
/// claude: neither the session's `inbox` nor claude's `skald recv` gets it, and A sees `held`. The
/// person's `skald held` lists it with its sender and accepts it: the session's next `inbox`
/// returns it, A's runs show `read` of it once, and C reads B's `read` receipt. No message waits
/// any more.
#[test]
fn held_message_reaches_the_running_session_only_once_the_person_accepts_it() {
    let mut roles = holding(
        [4_739_001_001, 4_739_001_002],
        ["ada-session-kept-back", "bob-sees-it-kept"],
    );
    let folder = roles.senders.agents.folder.clone();
    let mut session = session_in(roles.b(), &folder, &[AGENT, CLAUDE], CLIENT);
    let text = "hold test: claude, rotate the signing keys";
    let msg_id = roles.a_sends_to_claude(text);

    let mut got = session.inbox();
    got += &roles.claude_reads(false).shown();
    assert_not_handed(&got, text);
    a_sees_held(&mut roles, msg_id);
    assert_not_handed(&session.inbox(), text);

    let shown = decide(roles.b(), &folder, &[(text, Decision::Accept)]);

    assert_listed(&roles, &shown, msg_id, text);
    assert_shows_once(&session.inbox(), text);
    shows_read_once(roles.a(), msg_id);
    let to = roles.full(LAPTOP_AGENT);
    c_reads(&mut roles, msg_id, ReceiptStatus::Read, &to);
    says_none_waiting(roles.b(), "once accepted");
}

/// claude's `skald recv --wait` waits; B's person sets claude to `hold`. A's agent sends a message
/// `--to` claude: the waiting run keeps waiting and shows nothing of it, and A sees `held`. Once
/// the person accepts it, claude's next `skald recv` gets it and A shows `read` once.
#[test]
fn held_message_reaches_no_waiting_recv_until_accepted() {
    let mut roles = holding(
        [4_739_002_001, 4_739_002_002],
        ["ada-wait-kept-back", "bob-watches-the-wait"],
    );
    let mut wait = roles.claude_starts(&waiting());
    assert_keeps_waiting(&mut wait);
    let text = "hold test: claude, drain the eu queue";
    let msg_id = roles.a_sends_to_claude(text);

    assert_keeps_waiting(&mut wait);
    a_sees_held(&mut roles, msg_id);
    wait.assert_running();
    assert_not_handed(&wait.kill().shown(), text);

    accepted_then_read(&roles, msg_id, text);
}

/// B's person sets claude to `hold`. A's agent sends a message `--to` claude, which claude's
/// `skald recv` does not get, and A sees `held`. The person's `skald held` refuses it with a
/// reason: C reads B's `refused` receipt with that reason; A's person's runs show `refused` of it
/// once with the reason, and A's agent says it once with the reason; A shows no `read`. claude's
/// next `skald recv` gets nothing, and no message waits any more.
#[test]
fn held_message_the_person_refuses_never_reaches_the_agent_and_the_sender_sees_the_reason() {
    let mut roles = holding(
        [4_739_003_001, 4_739_003_002],
        ["ada-agent-turned-down-by-hand", "bob-gets-the-reason"],
    );
    let text = "hold test: claude, force-push the main branch";
    let msg_id = roles.a_sends_to_claude(text);
    assert_not_handed(&roles.claude_reads(false).shown(), text);
    a_sees_held(&mut roles, msg_id);

    let folder = roles.senders.agents.folder.clone();
    decide(roles.b(), &folder, &[(text, Decision::Refuse(REASON))]);

    let to = roles.full(LAPTOP_AGENT);
    let refused = c_reads(&mut roles, msg_id, ReceiptStatus::Refused, &to);
    assert!(reason_of(&refused).contains(REASON), "{refused:#?}");
    let line = shows_once(roles.a(), msg_id, REFUSED);
    assert!(line.contains(REASON), "no reason {REASON:?}: {line}");
    let said = a_agent_says_once(&roles, msg_id, REFUSED);
    assert!(said.contains(REASON), "no reason {REASON:?}: {said}");
    shows_not(roles.a(), msg_id, READ, "refused by the person");
    assert_not_handed(&roles.claude_reads(false).shown(), text);
    says_none_waiting(roles.b(), "once refused");
}
