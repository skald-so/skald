//! Linking a method: either method then signs in to the same account; a Google link lets the
//! person choose the Google account; a method the account has already changes nothing; accounts
//! never merge by email.

use ring::digest::{SHA256, digest};
use skald_oidc_stand_in::{Endpoint, base64url};

use super::common::browser::Browser;
use super::common::google::{CALLBACK, CLIENT_ID};
use super::{
    Method, assert_on_server, both, github_and_google, github_only, github_page, google_page,
    link_github, link_google, linked_at, listed, methods,
};

#[test]
fn google_user_links_github_and_signs_in_with_either() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988000", "ada.links@example.com");
    let bob = google.github_user(5_310_448_001, "bob-links-standin", "bob.links@example.org");

    let (page, who) = google_page(&google, &browser, &ada);
    assert_eq!(methods(&google, &page), vec![Method::Google]);

    let visit = link_github(&google, &page, &bob);
    assert_on_server(&google, &visit);
    let after = google.account(&page);
    assert_eq!(after.account, who.account, "the session keeps its account");
    assert_eq!(after.username, who.username, "a link keeps the username");
    assert_eq!(methods(&google, &page), github_and_google(&bob));
    let body = listed(&google, &page);
    let at = |provider: &str| {
        let items = body["methods"].as_array().expect("methods");
        let item = items.iter().find(|item| item["provider"] == provider);
        linked_at(item.unwrap_or_else(|| panic!("a {provider} method in {body}")))
    };
    assert!(at("github") >= at("google"), "linked after sign-up: {body}");

    // A GitHub sign-in now reaches the account, and so does a Google one.
    let (_, by_github) = github_page(&google, &browser, &bob);
    assert_eq!(by_github.account, who.account);
    assert_eq!(by_github.github_login.as_deref(), Some(bob.login.as_str()));
    let (_, by_google) = google_page(&google, &browser, &ada);
    assert_eq!(by_google.account, who.account);
    assert_eq!(google.account(&page).account, who.account);
}

#[test]
fn github_user_links_google_and_signs_in_with_either() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988010", "ada.reverse@example.com");
    let bob = google.github_user(5_310_448_011, "bob-reverse-standin", "bob.rev@example.org");

    let (page, who) = github_page(&google, &browser, &bob);
    assert_eq!(methods(&google, &page), github_only(&bob));

    let visit = link_google(&google, &page, &ada);
    assert_on_server(&google, &visit);
    assert_eq!(google.account(&page), who, "the session keeps its account");
    assert_eq!(methods(&google, &page), github_and_google(&bob));

    let (_, by_google) = google_page(&google, &browser, &ada);
    assert_eq!(by_google, who);
    let (_, by_github) = github_page(&google, &browser, &bob);
    assert_eq!(by_github, who);
}

#[test]
fn google_link_asks_to_choose_the_account() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988020", "ada.choose@example.com");
    let bob = google.github_user(
        5_310_448_021,
        "bob-choose-standin",
        "bob.choose@example.org",
    );

    let (page, _) = github_page(&google, &browser, &bob);
    link_google(&google, &page, &ada);
    assert_eq!(methods(&google, &page), github_and_google(&bob));

    let asked = google.provider.requests_to(Endpoint::Authorize);
    let [link] = asked.as_slice() else {
        panic!("one authorization request, the link's: {asked:?}");
    };
    let param = |name: &str| link.params.get(name).map(String::as_str);
    assert_eq!(param("prompt"), Some("select_account"), "{link:?}");
    let callback = format!("{}{CALLBACK}", google.public);
    assert_eq!(param("redirect_uri"), Some(callback.as_str()), "{link:?}");
    assert_eq!(param("client_id"), Some(CLIENT_ID), "{link:?}");
    assert_eq!(param("code_challenge_method"), Some("S256"), "{link:?}");
    let mut scopes: Vec<&str> = param("scope").unwrap_or_default().split(' ').collect();
    scopes.sort_unstable();
    assert_eq!(scopes, ["email", "openid"], "{link:?}");
    for name in ["state", "nonce"] {
        assert!(
            param(name).is_some_and(|value| value.len() >= 16),
            "{name}: {link:?}"
        );
    }
    let exchanged = google.provider.requests_to(Endpoint::Token);
    let [exchange] = exchanged.as_slice() else {
        panic!("one token request, the link's: {exchanged:?}");
    };
    let verifier = exchange
        .params
        .get("code_verifier")
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        Some(base64url(digest(&SHA256, verifier.as_bytes())).as_str()),
        param("code_challenge"),
        "the verifier of the link's challenge: {exchange:?}"
    );
}

#[test]
fn linking_a_method_twice_keeps_one() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988030", "ada.twice@example.com");
    let bob = google.github_user(5_310_448_031, "bob-twice-standin", "bob.twice@example.org");

    let (page, who) = google_page(&google, &browser, &ada);
    link_github(&google, &page, &bob);
    assert_eq!(methods(&google, &page), github_and_google(&bob));

    // The same GitHub user again, and the account's own Google method.
    link_github(&google, &page, &bob);
    assert_eq!(methods(&google, &page), github_and_google(&bob));
    link_google(&google, &page, &ada);
    assert_eq!(methods(&google, &page), github_and_google(&bob));
    assert_eq!(google.account(&page).account, who.account);

    let (_, by_github) = github_page(&google, &browser, &bob);
    assert_eq!(by_github.account, who.account);
}

#[test]
fn google_and_github_users_with_one_email_stay_two_accounts() {
    let browser = Browser::launch();
    let google = both(&[]);
    let email = "same.person@example.com";
    let ada = google.person("110248495921238988040", email);
    let bob = google.github_user(5_310_448_041, "bob-email-standin", email);
    let carol = google.github_user(5_310_448_042, "carol-email-standin", "carol@example.org");

    let (ada_page, by_google) = google_page(&google, &browser, &ada);
    let (bob_page, by_github) = github_page(&google, &browser, &bob);
    assert_ne!(by_google.account, by_github.account);
    assert_eq!(methods(&google, &ada_page), vec![Method::Google]);
    assert_eq!(methods(&google, &bob_page), github_only(&bob));

    // A link of another GitHub user joins nothing by email either.
    link_github(&google, &ada_page, &carol);
    assert_eq!(methods(&google, &ada_page), github_and_google(&carol));
    assert_eq!(methods(&google, &bob_page), github_only(&bob));
    let (_, again) = github_page(&google, &browser, &bob);
    assert_eq!(again.account, by_github.account);
    let (_, again) = google_page(&google, &browser, &ada);
    assert_eq!(again.account, by_google.account);
}
