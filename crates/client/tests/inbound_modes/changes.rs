//! A message taken in keeps the decision of the mode it was taken in under: a mode changed later
//! does not change it.

use crate::agents::assert_not_handed;
use crate::inbound::{
    ACCEPT, REFUSE, b_shows_it_marked, claude_set_to, shows_no_refused, shows_refused_once,
};
use crate::kit::received;
use crate::receipts::{Roles, assert_shows_once, shows_read_once};

/// B's person sets claude to `refuse`. A's agent sends a message `--to` claude, which B's person's
/// `skald recv` takes in and shows marked refused for claude. B's person then sets claude to
/// `accept`: claude's `skald recv` gets nothing, and A's runs show `refused` once.
#[test]
fn message_refused_when_taken_in_stays_refused_after_the_mode_turns_to_accept() {
    let roles = Roles::new(
        [4_731_011_001, 4_731_011_002],
        ["ada-agent-turned-away-first", "bob-sees-the-mode-turn"],
    );
    claude_set_to(&roles, REFUSE);
    let text = "inbound mode change test: claude, shut the old region down";
    let msg_id = roles.a_sends_to_claude(text);
    b_shows_it_marked(&roles, msg_id, text);

    claude_set_to(&roles, ACCEPT);

    assert_not_handed(&roles.claude_reads(false).shown(), text);
    shows_refused_once(roles.a(), msg_id);
}

/// A's agent sends a message `--to` claude, in `accept`, which B's person's `skald recv` takes in
/// and shows. B's person then sets claude to `refuse`: claude's `skald recv` gets the message, and
/// A's runs show `read` of it once and no `refused`.
#[test]
fn message_kept_for_the_agent_when_taken_in_is_handed_over_after_the_mode_turns_to_refuse() {
    let roles = Roles::new(
        [4_731_012_001, 4_731_012_002],
        ["ada-agent-accepted-before", "bob-sees-the-mode-close"],
    );
    let text = "inbound mode change test: claude, open the new region";
    let msg_id = roles.a_sends_to_claude(text);
    assert_shows_once(&received(roles.b()).shown(), text);

    claude_set_to(&roles, REFUSE);

    assert_shows_once(&roles.claude_reads(false).shown(), text);
    shows_read_once(roles.a(), msg_id);
    shows_no_refused(roles.a(), msg_id, "a message kept before the change");
}
