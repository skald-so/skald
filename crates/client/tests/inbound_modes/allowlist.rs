//! A sender on an agent's allowlist reaches it whatever the mode: by its address in either form
//! `--to` takes, or by its member's handle, which admits the member's agents and person alike. A
//! listed agent admits no other agent of its member.

use crate::agents::assert_not_handed;
use crate::inbound::{REFUSE, REFUSED, a_sees_refused, claude_allows, claude_set_to, shows_not};
use crate::receipts::{Roles, assert_shows_once, shows_read_once};
use crate::senders::LAPTOP_AGENT;

/// An agent of Ada's that sends nothing.
const ANOTHER_AGENT: &str = "codex-x4z9qp";

/// What A's person says to every agent in the channel; it is nowhere else.
const FROM_A_S_PERSON: &str = "inbound allowlist test: everyone, the release is cut";

/// How the allowlist names A's agent.
#[derive(Debug, Clone, Copy)]
enum Named {
    /// `<ada>/<agent>`.
    Short,
    /// `acme/<ada>/<agent>@<host>`.
    Full,
    /// `<ada>`, Ada's handle.
    Handle,
}

impl Named {
    fn sender(self, roles: &Roles) -> String {
        match self {
            Named::Short => format!("{}/{LAPTOP_AGENT}", roles.senders.ada()),
            Named::Full => roles.full(LAPTOP_AGENT),
            Named::Handle => roles.senders.ada(),
        }
    }
}

/// B's person sets claude to `refuse` and puts A's agent, named as `named` says, on its allowlist.
/// A's agent sends a message `--to` claude; claude's `skald recv` gets it, and A's runs show `read`
/// of it once and no `refused`.
fn allowed_sender_is_handed_over(roles: &Roles, named: Named, text: &str) {
    claude_set_to(roles, REFUSE);
    claude_allows(roles, &named.sender(roles));
    let msg_id = roles.a_sends_to_claude(text);

    let got = roles.claude_reads(false).shown();

    assert_shows_once(&got, text);
    shows_read_once(roles.a(), msg_id);
    shows_not(roles.a(), msg_id, REFUSED, "a sender on the allowlist");
}

#[test]
fn agent_on_the_allowlist_by_its_short_address_reaches_an_agent_in_refuse() {
    let roles = Roles::new(
        [4_731_004_001, 4_731_004_002],
        ["ada-agent-allowed-short", "bob-sees-it-allowed"],
    );
    allowed_sender_is_handed_over(
        &roles,
        Named::Short,
        "inbound allowlist test: claude, merge the hotfix",
    );
}

#[test]
fn agent_on_the_allowlist_by_its_full_address_reaches_an_agent_in_refuse() {
    let roles = Roles::new(
        [4_731_005_001, 4_731_005_002],
        ["ada-agent-allowed-full", "bob-sees-it-allowed-full"],
    );
    allowed_sender_is_handed_over(
        &roles,
        Named::Full,
        "inbound allowlist test: claude, tag the hotfix",
    );
}

/// As above, with Ada's handle on claude's allowlist; then A's person says a text to every agent in
/// the channel, which claude's next `skald recv` gets too.
#[test]
fn member_s_handle_on_the_allowlist_admits_its_agents_and_its_person() {
    let roles = Roles::new(
        [4_731_006_001, 4_731_006_002],
        ["ada-handle-allowed", "bob-sees-the-handle-allowed"],
    );
    allowed_sender_is_handed_over(
        &roles,
        Named::Handle,
        "inbound allowlist test: claude, deploy the hotfix",
    );

    roles.senders.laptop_person_says(FROM_A_S_PERSON);

    let got = roles.claude_reads(false).shown();
    assert_shows_once(&got, FROM_A_S_PERSON);
}

/// B's person sets claude to `refuse` and puts another agent of Ada's on its allowlist. A's agent
/// sends a message `--to` claude; claude's `skald recv` gets nothing, and A sees `refused`.
#[test]
fn only_another_agent_of_the_member_on_the_allowlist_leaves_the_message_refused() {
    let mut roles = Roles::new(
        [4_731_007_001, 4_731_007_002],
        ["ada-other-agent-allowed", "bob-sees-the-wrong-one"],
    );
    claude_set_to(&roles, REFUSE);
    claude_allows(&roles, &format!("{}/{ANOTHER_AGENT}", roles.senders.ada()));
    let text = "inbound allowlist test: claude, revert the hotfix";
    let msg_id = roles.a_sends_to_claude(text);

    let got = roles.claude_reads(false).shown();

    assert_not_handed(&got, text);
    a_sees_refused(&mut roles, msg_id);
    assert_not_handed(&roles.claude_reads(false).shown(), text);
}
