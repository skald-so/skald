//! A removed method reaches no account: its next sign-in signs up as a new account, or is refused
//! while sign-up is closed, and another account may link it.

use crate::common::sign_in::github_rows;
use crate::{BOTH, Provider, Stand, Via, assert_removed, link_github, listed, remove, sorted};

/// With sign-up open, Ada removes her GitHub method; its next sign-in creates a new account with
/// GitHub alone, and Ada's account keeps Google alone.
#[test]
fn removed_method_signs_up_as_a_new_account() {
    let stand = Stand::launch(&[]);
    let api = stand.api();
    let ada = stand.with_both(301, "ada-anew");
    let cookie = &ada.session.cookie;

    assert_removed(&remove(&api, cookie, &ada.id(Provider::GitHub)));

    let new = stand.signed_in(Via::GitHub(&ada.user));
    assert_ne!(new.who.account, ada.session.who.account, "a new account");
    let login = new.who.github_login.as_deref();
    assert_eq!(login, Some(ada.user.login.as_str()));
    assert_eq!(sorted(&listed(&api, &new.cookie)), [Provider::GitHub]);
    assert_eq!(sorted(&listed(&api, cookie)), [Provider::Google]);
    let again = stand.signed_in(Via::Google(&ada.person));
    assert_eq!(again.who.account, ada.session.who.account);
}

/// With sign-up closed, Ada and Carol, listed by their Google emails, have accounts, and Ada links
/// her GitHub user, which no list has, then removes it. Its next sign-in is refused and creates
/// nothing; Carol then links it to her account, and its sign-in reaches hers.
#[test]
fn removed_method_is_refused_with_sign_up_closed_and_another_account_links_it() {
    let emails = "ada-closed@example.test,carol-closed@example.test";
    let stand = Stand::launch(&["--sign-up", "closed", "--sign-up-google-emails", emails]);
    let api = stand.api();
    let ada = stand.with_both(311, "ada-closed");
    let (carol, _) = stand.person(312, "carol-closed");

    assert_removed(&remove(
        &api,
        &ada.session.cookie,
        &ada.id(Provider::GitHub),
    ));

    let before = github_rows(&stand.google.db, &ada.user);
    let refused = stand.browser.page();
    stand.google.github_sign_in(&refused, &ada.user);
    stand.google.assert_signed_out(&refused);
    let after = github_rows(&stand.google.db, &ada.user);
    assert_eq!(after, before, "creates nothing");

    let carol = stand.signed_in(Via::Google(&carol));
    link_github(&stand.google, &carol.page, &ada.user);
    assert_eq!(sorted(&listed(&api, &carol.cookie)), BOTH);
    let by_github = stand.signed_in(Via::GitHub(&ada.user));
    assert_eq!(by_github.who.account, carol.who.account);
    let adas = listed(&api, &ada.session.cookie);
    assert_eq!(sorted(&adas), [Provider::Google]);
}
