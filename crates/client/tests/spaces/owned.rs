//! The limit on the spaces an account owns, in the CLI: when the server refuses `skald
//! create-space <name>` with the problem `https://skald.so/problems/owned-spaces-limit`, the run
//! fails, says the account `owns as many spaces as it may`, by the problem's type whatever its
//! `detail`, and creates nothing, then or on a later run. The relay answers the creation itself
//! with that refusal; the server's own limit is tested in `crates/server/tests/spaces/owned.rs`.

use super::{CREATE, STORE, account, create_space, line_of, setup, spaces, team_spaces};

/// The server's refusal of a creation beyond the limit; its `detail` says otherwise than the CLI.
const OVER_LIMIT: &str = r#"{"type":"https://skald.so/problems/owned-spaces-limit","title":"Space limit reached","status":403,"detail":"Refused: 2 of 2."}"#;

/// What the CLI says on that refusal.
const SAID: &str = "owns as many spaces as it may";

/// A creation the server refuses as beyond the limit fails the run, which says the account owns as
/// many spaces as it may and sends no revision; the space is not listed. Once the server would
/// take it, a later run creates nothing either.
#[test]
fn creation_beyond_the_limit_creates_nothing_and_says_the_account_owns_as_many_spaces_as_it_may() {
    let (cloud, browser) = setup();
    let ada = account(
        &cloud,
        &browser,
        4_120_711_001,
        "ada-limit-standin",
        "ada-workstation",
    );
    cloud.relay.refuse_with(CREATE, "403 Forbidden", OVER_LIMIT);
    let since = cloud.relay.mark();

    let refused = create_space(&ada.machine, "acme");

    assert!(!refused.success, "{refused:?}");
    assert_eq!(cloud.count_sent(since, CREATE), 1, "{refused:?}");
    assert!(
        refused.shown().to_lowercase().contains(SAID),
        "it does not say the account owns as many spaces as it may: {refused:?}"
    );
    assert_eq!(cloud.count_sent(since, STORE), 0, "a revision was sent");
    assert_eq!(line_of(&spaces(&ada.machine), "acme"), None);

    cloud.relay.stop_refusing();
    let since = cloud.relay.mark();
    assert_eq!(line_of(&spaces(&ada.machine), "acme"), None);
    assert_eq!(
        cloud.count_sent(since, CREATE),
        0,
        "the creation was sent again"
    );
    assert_eq!(
        team_spaces(&cloud, &ada),
        Vec::new(),
        "a later run created it"
    );
}
