//! An admin sees the token created in the CLI with its metadata, also at 360 px; a member who is no
//! admin sees none.

use crate::kit::DEFAULT_DAYS;
use crate::spa::{answers_without_secrets, assert_no_secrets};
use crate::stage::{Harness, Stage, shows_no_token, token_row};

/// A phone's width, in CSS pixels.
const PHONE: u32 = 360;

/// Whether `row`, in lower case, holds one of `forms`, in any case.
fn holds_one_of(row: &str, forms: &[String]) -> bool {
    forms.iter().any(|form| row.contains(&form.to_lowercase()))
}

/// Ada creates `ci/deploy-bot` with `skald` for `acme`'s default channel, with the default send and
/// receive permissions and expiry of 90 days. Her Access has one row of its token: it names `ci`
/// and `deploy-bot`, says send and receive and not queues, names the channel, says when the token
/// expires, and has Revoke. No answer the web app got holds the token, the key secret or another
/// secret. In a browser 360 px wide with her session, `/access` shows the row and does not scroll
/// sideways. Bob, a member of `acme` who is no admin, sees no token in his Access, and no failure.
#[test]
fn an_admin_sees_the_token_created_in_the_cli_with_its_metadata_and_a_member_none() {
    let harness = Harness::start();
    let stage = Stage::set(&harness, 3_359_500_001);
    let page = &stage.ada_page;

    harness.open_access(page, &stage.ada);
    let row = token_row(page).to_lowercase();
    let words: Vec<&str> = row
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .collect();
    assert!(words.contains(&"ci"), "the owner: {row:?}");
    for part in ["send", "receive", "revoke"] {
        assert!(row.contains(part), "{part}: {row:?}");
    }
    assert!(!row.contains("queue"), "queues: {row:?}");
    let channels = stage.channel_forms();
    assert!(holds_one_of(&row, &channels), "{channels:?}: {row:?}");
    let expiry = stage.expiry_forms(DEFAULT_DAYS);
    assert!(holds_one_of(&row, &expiry), "{expiry:?}: {row:?}");
    let answers = answers_without_secrets(page);
    assert_no_secrets(
        &answers,
        &[stage.secrets.token.clone(), stage.secrets.key.clone()],
    );

    let phone = harness.world.browser.page_of_width(PHONE);
    phone.add_cookies(&page.cookies());
    phone.goto(&harness.spa.at("/access"));
    token_row(&phone);
    let sideways = phone
        .evaluate("document.documentElement.scrollWidth - document.documentElement.clientWidth");
    assert_eq!(sideways.as_i64(), Some(0), "{:?}", phone.text());

    harness.open_access(&stage.bob_page, &stage.bob);
    assert!(
        shows_no_token(&stage.bob_page),
        "Bob's Access: {:?}",
        stage.bob_page.text()
    );
}
