//! The Account page of the web app, in the browser: linking and removing sign-in methods.

use skald_github_stand_in::User;

use crate::account::{
    ask_to_remove, confirm_removal, location, open_account, remove_count, sign_in_prompts,
};
use crate::common::browser::Page;
use crate::common::sign_in::github_session;
use crate::common::spa::{ACCOUNT_MENU, wait_for_cabinet, wait_for_sign_in_page, wait_until};
use crate::kit::{Providers, Web, assert_answers};
use crate::sessions::{Api, cookie_of, token_of};
use crate::{BOTH, LINK_AGE, LINK_GITHUB, Provider, listed, sorted};

/// Grace's username: her Google email's local part.
const GRACE: &str = "grace-account";

/// Signs Grace, `person`'s Google, up in `page` and links the GitHub user `user` from the Account
/// page; checks that the link comes back to it, which then names her login and offers Remove on
/// both methods. Gives her account's ID.
fn grace_with_both(
    web: &Web,
    page: &Page,
    person: &skald_oidc_stand_in::Person,
    user: &User,
) -> String {
    web.sign_in_with_google(page, person, "/");
    wait_for_cabinet(page, GRACE);
    let account = web.session(page)["account"].clone();
    page.goto(&web.spa.at("/"));
    wait_for_cabinet(page, GRACE);
    open_account(page);
    assert_eq!(remove_count(page), 0, "no Remove on the only method");
    let here = location(page);

    // Signing in to GitHub leaves the Account page: open it again.
    github_session(page, web.stand.github(), user);
    page.goto(&here);
    wait_until(page, "the Account page again", |page| {
        page.count(LINK_GITHUB) == 1
    });
    page.click_settled(LINK_GITHUB);
    wait_until(page, "the Account page with GitHub linked", |page| {
        location(page) == here && page.text().contains(&user.login) && remove_count(page) == 2
    });
    account.as_str().expect("an account ID").to_owned()
}

/// Grace signs up with Google and links GitHub from the Account page, coming back to it. She
/// removes Google, the method of this session: the confirmation says it signs this browser out,
/// and the web app then shows the sign-in page. GitHub signs her in to the same account, which has
/// GitHub alone, and the Account page offers no Remove.
#[test]
fn google_user_links_github_removes_google_and_signs_in_with_github() {
    let web = Web::launch(Providers::Both, &[]);
    let person = web
        .stand
        .person("322300000501", "grace-account@example.test");
    let user = web.github_user(3_223_000_501, "Grace-Account-Standin");
    let page = web.page();
    let account = grace_with_both(&web, &page, &person, &user);
    let text = page.text();
    assert!(text.contains(GRACE), "the username: {text:?}");

    let asked = ask_to_remove(&page, Provider::Google).expect("a confirmation");
    assert!(asked.to_lowercase().contains("this browser"), "{asked:?}");
    confirm_removal(&page);
    wait_for_sign_in_page(&page);
    assert_answers(&page);

    web.sign_in_with_github(&page, &user, "/");
    wait_for_cabinet(&page, GRACE);
    let session = web.session(&page);
    assert_eq!(session["account"], account.as_str(), "{session}");
    assert_eq!(session["sign_in_method"], "github", "{session}");
    let api = Api::of_google(&web.stand);
    assert_eq!(sorted(&listed(&api, &cookie_of(&page))), [Provider::GitHub]);
    page.goto(&web.spa.at("/"));
    wait_for_cabinet(&page, GRACE);
    open_account(&page);
    assert_eq!(remove_count(&page), 0, "no Remove on the last method");
}

/// Grace, with Google and GitHub, has a sign-in older than the setting: Remove on GitHub, the other
/// method, asks her to sign in again and removes nothing, and she stays in the cabinet.
#[test]
fn with_an_older_sign_in_remove_asks_to_sign_in_again_and_removes_nothing() {
    let web = Web::launch(Providers::Both, &[LINK_AGE, "60"]);
    let person = web
        .stand
        .person("322300000511", "grace-account@example.test");
    let user = web.github_user(3_223_000_511, "Grace-Older-Standin");
    let page = web.page();
    grace_with_both(&web, &page, &person, &user);
    let api = Api::of_google(&web.stand);
    let cookie = cookie_of(&page);
    api.move_sign_in_back(token_of(&cookie), "2 minutes");
    page.goto(&web.spa.at("/"));
    wait_for_cabinet(&page, GRACE);
    open_account(&page);
    let asking = sign_in_prompts(&page);

    if let Some(asked) = ask_to_remove(&page, Provider::GitHub) {
        assert!(!asked.to_lowercase().contains("this browser"), "{asked:?}");
        confirm_removal(&page);
    }
    wait_until(&page, "a prompt to sign in again", |page| {
        sign_in_prompts(page) > asking
    });

    assert_eq!(page.count(ACCOUNT_MENU), 1, "still in the cabinet");
    assert_eq!(sorted(&listed(&api, &cookie)), BOTH);
    assert!(api.web_works(&cookie));
    assert_answers(&page);
}
