//! What the helper writes into a session's socket at once stays within Claude Code's hook output
//! limit, as what the hooks give: an `own` message longer than the limit is cut and ends with how
//! to read it whole.

use crate::hooks::{LIMIT, assert_cut};
use crate::registered::Ada;
use crate::solo::WEB;
use crate::standin::{Standin, Take};

/// What starts api's long message; it is nowhere else.
const LONG_HEAD: &str = "cut test: api's export for web, 12000 characters:";

/// web's session starts. api sends web a message of 12,000 characters. The session's socket gets
/// one line of at most 10,000 characters: it starts with the message's head, does not hold its
/// whole text, and its last line names `inbox` and the message's `#<ref>`.
#[test]
fn own_message_over_the_limit_goes_into_the_socket_cut_with_how_to_read_it_whole() {
    let ada = Ada::new(4_919_464_001, "ada-socket-cut-export");
    let session = Standin::new(Take::Queue);
    ada.starts(WEB, &session);
    let mut long = format!("{LONG_HEAD} ");
    while long.len() < 12_000 {
        long.push_str("row of the export, ");
    }
    long.truncate(12_000);

    let msg_id = ada.web.own(&long);

    let content = session.wait_lines(1).remove(0).content();
    assert!(
        content.chars().count() <= LIMIT,
        "{} characters",
        content.len()
    );
    assert!(
        content.contains(LONG_HEAD),
        "not the long message: {content}"
    );
    assert_cut(&content, &long, msg_id);
}
