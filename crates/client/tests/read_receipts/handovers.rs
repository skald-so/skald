//! `read` follows the handover to the `to` agent, by its `skald recv`, `skald recv --wait` or
//! `inbox`, and not the person's run.

use crate::agents::CLAUDE;
use crate::client::{MCP, Session};
use crate::kit::received;
use crate::receipts::{Roles, assert_shows_once, shows_no_read, shows_read_once, waiting};
use crate::world::AGENT;

/// The name the MCP client gives itself at initialization.
const CLIENT: &str = "claude-code";

/// How B's agent takes the message.
#[derive(Debug, Clone, Copy)]
enum Handover {
    Recv,
    Wait,
    Inbox,
}

/// A's agent sends a message `--to` claude. A's `skald recv` shows no `read` of it; nor after B's
/// person's `skald recv`, which shows the message, alone. Then claude takes it as `handover` says
/// and gets it; A's runs show `read` of it once, and no more after that. An MCP session stays open
/// until then.
fn read_once_claude_has_it(ids: [u64; 2], logins: [&str; 2], handover: Handover, text: &str) {
    let roles = Roles::new(ids, logins);
    let msg_id = roles.a_sends_to_claude(text);
    shows_no_read(roles.a(), msg_id, "before B has the message");

    let person = received(roles.b());

    assert_shows_once(&person.shown(), text);
    shows_no_read(roles.a(), msg_id, "after B's person's run alone");

    let mut session = None;
    let got = match handover {
        Handover::Recv => roles.claude_reads(false).stdout,
        Handover::Wait => {
            let run = roles.claude_starts(&waiting()).finish();
            assert!(run.success, "{run:?}");
            run.stdout
        }
        Handover::Inbox => {
            let folder = &roles.senders.agents.folder;
            let args = [AGENT, CLAUDE, MCP];
            let mut started = Session::start(&mut roles.b().command_in(folder, &args), CLIENT);
            let shown = started.inbox();
            session = Some(started);
            shown
        }
    };

    assert_shows_once(&got, text);
    shows_read_once(roles.a(), msg_id);
    drop(session);
}

#[test]
fn read_shows_once_the_agent_s_recv_has_the_message_and_not_after_the_person_s_run() {
    read_once_claude_has_it(
        [4_155_001_001, 4_155_001_002],
        ["ada-agent-reads-it", "bob-sees-the-channel"],
        Handover::Recv,
        "receipt test: claude, rerun the nightly suite",
    );
}

#[test]
fn read_shows_once_the_agent_s_waiting_recv_has_the_message() {
    read_once_claude_has_it(
        [4_155_002_001, 4_155_002_002],
        ["ada-agent-waits-for-it", "bob-watches-the-wait"],
        Handover::Wait,
        "receipt test: claude, bump the lockfile",
    );
}

#[test]
fn read_shows_once_the_inbox_tool_has_the_message() {
    read_once_claude_has_it(
        [4_155_003_001, 4_155_003_002],
        ["ada-session-inbox-reads", "bob-near-the-session"],
        Handover::Inbox,
        "receipt test: claude, archive the old builds",
    );
}
