//! The command that decides held messages runs only as the person at a terminal: as an agent, or
//! with its input or output not a terminal, it decides nothing and says why.

use std::process::Stdio;

use crate::agents::{CLAUDE, assert_not_handed};
use crate::held::{ACCEPTS, DECIDE, accepted_then_read, holding, says_waiting, shows_not};
use crate::inbound::REFUSED;
use crate::machine::{Finished, Run};
use crate::receipts::READ;
use crate::terminal::Terminal;
use crate::world::AGENT;

/// Fails unless `what` failed and said something.
#[track_caller]
fn refused_to_decide(what: &str, success: bool, shown: &str) {
    assert!(!success, "{what} decided: {shown}");
    assert!(!shown.trim().is_empty(), "{what} says nothing");
}

/// B's person sets claude to `hold`, and A's agent sends a message `--to` claude, which waits for a
/// decision. `skald held` with its input closed, with `accept` piped into its input, and as claude
/// at a terminal where `accept` is typed, each fails and says why. claude's `skald recv` gets
/// nothing, A shows neither `read` nor `refused`, and the message still waits. The person's
/// `skald held` at a terminal then accepts it: claude gets it, and A shows `read`.
#[test]
fn without_a_terminal_or_as_an_agent_the_command_decides_nothing() {
    let roles = holding(
        [4_739_004_001, 4_739_004_002],
        ["ada-agent-cannot-decide", "bob-sees-it-undecided"],
    );
    let folder = roles.senders.agents.folder.clone();
    let text = "hold test: claude, approve your own access request";
    let msg_id = roles.a_sends_to_claude(text);
    says_waiting(roles.b(), 1);

    let closed: Finished = roles.b().start_in(&folder, &[DECIDE]).finish();
    refused_to_decide("input closed", closed.success, &closed.shown());
    let mut piped = roles.b().command_in(&folder, &[DECIDE]);
    piped.stdin(Stdio::piped());
    let mut fed = Run::spawn(&mut piped);
    fed.answer(&format!("{ACCEPTS}\n"));
    let fed = fed.finish();
    refused_to_decide("input piped", fed.success, &fed.shown());
    let agent = Terminal::start_in(roles.b(), &folder, &[AGENT, CLAUDE, DECIDE]);
    agent.type_line(ACCEPTS);
    let agent = agent.finish();
    refused_to_decide("as an agent", agent.success, &agent.text);

    assert_not_handed(&roles.claude_reads(false).shown(), text);
    for status in [READ, REFUSED] {
        shows_not(roles.a(), msg_id, status, "decided by no person");
    }
    says_waiting(roles.b(), 1);

    accepted_then_read(&roles, msg_id, text);
}
