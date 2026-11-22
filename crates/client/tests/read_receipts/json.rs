//! An agent sees `read` only in `--json`: its short lines show none, so a receipt costs it no
//! token.

use crate::receipts::{READ, Roles, read_objects};
use crate::senders::holds_word;

/// What A's agent sends; none is anywhere else.
const FIRST: &str = "receipt json test: claude, tag the release candidate";
const SECOND: &str = "receipt json test: claude, publish the changelog";

/// A's agent sends a message `--to` claude, which claude's `skald recv` gets; once C has read B's
/// receipt of it, so the server holds it for A too, A's agent's `skald recv` shows no line with the
/// word `read`. A's agent sends a second message, which claude gets; once C has read its receipt,
/// A's agent's `skald recv --json` gives one `read` status of it, with the channel and B's device.
#[test]
fn agent_s_short_lines_show_no_read_and_its_json_has_it() {
    let mut roles = Roles::new(
        [4_155_004_001, 4_155_004_002],
        ["ada-agent-skips-receipts", "bob-holds-the-receipts"],
    );
    let first = roles.a_sends_to_claude(FIRST);
    roles.claude_reads(false);
    roles.c_gets_receipt_of(first);

    let short = roles.a_agent_reads(&[]);

    let shown = short.shown();
    let with_read: Vec<&str> = (shown.lines())
        .filter(|line| holds_word(line, READ))
        .collect();
    assert!(with_read.is_empty(), "{READ} in short lines: {shown}");

    let second = roles.a_sends_to_claude(SECOND);
    roles.claude_reads(false);
    roles.c_gets_receipt_of(second);

    let json = roles.a_agent_reads_json();

    let objects = read_objects(&json.stdout, second);
    let [object] = objects.as_slice() else {
        panic!("not one {READ} of {second}: {json:?}");
    };
    assert_eq!(
        object["channel"].as_str(),
        Some(roles.channel().to_string().as_str()),
        "{object}"
    );
    assert_eq!(
        object["device"].as_str(),
        Some(roles.b_device().to_string().as_str()),
        "{object}"
    );
}
