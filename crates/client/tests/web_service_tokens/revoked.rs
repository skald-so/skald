//! Revoke on a token's row asks first; once confirmed, the server refuses the token at its next
//! request.

use crate::access::{press, revoke, rows_holding};
use crate::kit::{AGENT_NAME, own_record};
use crate::spa::wait_until;
use crate::stage::{Harness, Stage, token_row};

/// The dialogs a confirmation opens.
const DIALOG: &str = "[role=dialog], [role=alertdialog]";

/// Ada clicks Revoke on the row of `ci/deploy-bot`'s token: a dialog asks first, and the token
/// still reads its record; Cancel closes it and keeps the token. She clicks Revoke again and
/// confirms: the row goes, and the server refuses the token at its next request.
#[test]
fn revoke_asks_first_and_then_the_token_is_refused_at_its_next_request() {
    let harness = Harness::start();
    let stage = Stage::set(&harness, 3_359_600_001);
    let cloud = &harness.world.cloud;
    let page = &stage.ada_page;
    let token = &stage.secrets.token;
    harness.open_access(page, &stage.ada);
    token_row(page);

    revoke(page, AGENT_NAME);
    wait_until(page, "the question before revoking", |page| {
        page.count(DIALOG) == 1
    });
    assert!(own_record(cloud, token).is_some(), "revoked before a yes");
    press(page, Some(DIALOG), "button", "^cancel$");
    wait_until(page, "the question closed", |page| page.count(DIALOG) == 0);
    assert_eq!(rows_holding(page, AGENT_NAME).len(), 1, "the token's row");
    assert!(own_record(cloud, token).is_some(), "revoked on cancel");

    revoke(page, AGENT_NAME);
    wait_until(page, "the question before revoking", |page| {
        page.count(DIALOG) == 1
    });
    press(page, Some(DIALOG), "button", "revoke");
    wait_until(page, "the token's row gone", |page| {
        rows_holding(page, AGENT_NAME).is_empty()
    });
    assert_eq!(
        own_record(cloud, token),
        None,
        "the token still works after its revocation"
    );
}
