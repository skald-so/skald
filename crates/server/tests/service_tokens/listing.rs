//! Any admin of the space lists its tokens and revokes any, a web session only from a page of the
//! server; a member who is no admin and another space's admin do neither. A token's last use
//! follows its requests.

use crate::api::assert_refused;
use crate::common::device_flow::text;
use crate::tokens::{ELSEWHERE, Stage, bearer, find, seconds, tokens_of};
use crate::wire::now;

/// GitHub user IDs of the tests' people: the fixture's alice has the first, bob the next.
const LISTING: u64 = 3_535_400_001;
const WEB: u64 = 3_535_400_011;

/// alice's session creates a token of the bot; bob's, the other admin's, lists it and revokes it,
/// after which it is refused.
#[test]
fn another_admin_lists_and_revokes_a_token() {
    let stage = Stage::new(LISTING);
    let created = stage.create();
    let bob = &stage.f.team.bob;
    let listed = stage.list(bob);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(
        find(&listed, &created.id)["device"],
        stage.bot.id.to_string()
    );

    let response = stage.revoke_as(bob, &created.id);
    assert_eq!(response.status, 204, "{}", text(&response));
    assert!(!stage.works(&created.token), "revoked by bob");
    assert!(stage.list(bob).is_empty(), "still listed");
}

/// carol, a member who is no admin, and eve, the admin of another space, neither list `acme`'s
/// tokens nor revoke the bot's: each request is refused and the token still works.
#[test]
fn a_member_who_is_no_admin_and_another_spaces_admin_neither_list_nor_revoke() {
    let stage = Stage::new(3_535_400_021);
    stage.other_channel();
    let created = stage.create();
    let t = &stage.f.team;
    for (what, person) in [
        ("carol, a member", &t.carol),
        ("eve, globex's admin", &stage.f.eve),
    ] {
        let bearer = bearer(person.session.token());
        let headers = [("authorization", bearer.as_str())];
        assert_refused(&stage.list_response(&headers), &format!("{what} lists"));
        assert_refused(
            &stage.revoke_response(&created.id, &headers),
            &format!("{what} revokes"),
        );
        assert!(stage.works(&created.token), "{what}: the token ended");
    }
    assert_eq!(stage.listed(&t.alice), vec![created.id]);
}

/// bob's web session lists the bot's token from a page of the server. Its revocation from another
/// site and without `Origin` ends nothing; from a page of the server it ends the token.
#[test]
fn a_web_session_revokes_only_from_a_page_of_the_server() {
    let stage = Stage::new(WEB);
    let created = stage.create();
    let cookie = stage.cookie(&stage.f.team.bob, WEB + 1);

    let own = [("cookie", cookie.as_str()), ("origin", stage.public())];
    let listed = tokens_of(&stage.list_response(&own));
    find(&listed, &created.id);

    let elsewhere = [("cookie", cookie.as_str()), ("origin", ELSEWHERE)];
    for (what, headers) in [("another site", &elsewhere[..]), ("no origin", &own[..1])] {
        assert_refused(&stage.revoke_response(&created.id, headers), what);
        assert!(stage.works(&created.token), "{what}: the token ended");
    }

    let response = stage.revoke_response(&created.id, &own);
    assert_eq!(response.status, 204, "{}", text(&response));
    assert!(
        !stage.works(&created.token),
        "revoked from a page of the server"
    );
}

/// A new token is listed without a last use; after the token's request, its last use is a time of
/// that request's minute.
#[test]
fn a_tokens_last_use_follows_its_request() {
    let stage = Stage::new(3_535_400_031);
    let created = stage.create();
    let alice = &stage.f.team.alice;
    let unused = stage.list(alice);
    let last = &find(&unused, &created.id)["last_used_at"];
    assert!(last.is_null(), "a last use before any: {last}");

    let before = i64::try_from(now()).expect("a time");
    assert!(stage.works(&created.token));
    let after = i64::try_from(now()).expect("a time");
    let used = stage.list(alice);
    let at = seconds(stage.db(), &find(&used, &created.id)["last_used_at"]);
    assert!(
        (before - 60..=after).contains(&at),
        "last used at {at}, not in the minute of a request between {before} and {after}"
    );
}
