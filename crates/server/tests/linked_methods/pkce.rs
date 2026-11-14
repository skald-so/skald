//! GitHub sign-ins and links send an S256 PKCE challenge (RFC 7636) and exchange the code with its
//! verifier; a code exchanged with a verifier other than its challenge's signs in and links nothing.

use ring::digest::{SHA256, digest};
use skald_github_stand_in::{Endpoint, Request};
use skald_oidc_stand_in::base64url;

use super::common::browser::Browser;
use super::common::sign_in::{
    CALLBACK as GITHUB_CALLBACK, SignIn, github_authorize_link, is_s256_challenge, with_param,
};
use super::{
    assert_unlinked, both, github_and_google, github_page, google_page, held_github_link,
    link_github, methods,
};

/// The S256 challenge of `verifier`.
fn s256(verifier: &str) -> String {
    base64url(digest(&SHA256, verifier.as_bytes()))
}

/// A challenge whose verifier the server does not have.
fn foreign_challenge() -> String {
    s256("a-verifier-the-server-never-made-0123456789abcdefghijk")
}

/// Checks that `request`, an authorization request to GitHub, sends an S256 challenge; gives it.
#[track_caller]
fn challenge_of(request: &Request) -> String {
    let param = |name: &str| request.params.get(name).map(String::as_str);
    assert_eq!(param("code_challenge_method"), Some("S256"), "{request:?}");
    let challenge = param("code_challenge").unwrap_or_default();
    assert!(
        is_s256_challenge(challenge),
        "an S256 challenge is 43 characters of base64url: {request:?}"
    );
    challenge.to_owned()
}

#[test]
fn github_sign_ins_and_links_send_a_challenge_and_exchange_with_its_verifier() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988200", "ada.pkce@example.com");
    let bob = google.github_user(5_310_448_201, "bob-pkce-standin", "bob.pkce@example.org");
    let carol = google.github_user(5_310_448_202, "carol-pkce-standin", "carol.p@example.org");

    // A link, then a sign-in.
    let (page, _) = google_page(&google, &browser, &ada);
    link_github(&google, &page, &bob);
    assert_eq!(methods(&google, &page), github_and_google(&bob));
    github_page(&google, &browser, &carol);

    let github = google.github();
    let asked = github.requests_to(Endpoint::Authorize);
    let exchanged = github.requests_to(Endpoint::Token);
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(exchanged.len(), 2, "{exchanged:?}");
    let callback = format!("{}{GITHUB_CALLBACK}", google.public);
    let mut challenges = Vec::new();
    for (request, exchange) in asked.iter().zip(&exchanged) {
        assert_eq!(
            request.params.get("redirect_uri"),
            Some(&callback),
            "the sign-in callback: {request:?}"
        );
        let challenge = challenge_of(request);
        let verifier = exchange.params.get("code_verifier").cloned();
        let verifier = verifier.unwrap_or_else(|| panic!("a code_verifier: {exchange:?}"));
        assert!(
            (43..=128).contains(&verifier.len()),
            "a PKCE verifier: {exchange:?}"
        );
        assert_eq!(s256(&verifier), challenge, "the verifier of its challenge");
        challenges.push(challenge);
    }
    assert_ne!(
        challenges[0], challenges[1],
        "each has a challenge of its own"
    );
    assert_eq!(github.tokens().len(), 2);
}

#[test]
fn github_sign_in_with_a_wrong_verifier_signs_no_one_in() {
    let browser = Browser::launch();
    let stand = SignIn::start();
    let ada = stand.user(5_310_448_211, "ada-verifier-standin");

    let page = browser.page();
    let visit = page.goto(&stand.github.asking_sign_in_url(ada.id));
    assert_eq!(
        visit.status,
        Some(200),
        "GitHub stand-in sign-in: {visit:?}"
    );
    page.goto(&stand.start_url(None));
    let asked = stand.github.requests_to(Endpoint::Authorize);
    challenge_of(&asked[0]);

    // GitHub gives a code for another challenge, as for an attacker who started the sign-in.
    let query = serde_urlencoded::to_string(&asked[0].params).expect("a query");
    let authorize = format!("{}/login/oauth/authorize?{query}", stand.github.url());
    let foreign = with_param(&authorize, "code_challenge", Some(&foreign_challenge()));
    let visit = page.goto(&foreign);
    page.goto_settled(&stand.authorize_link(&page, &visit));

    stand.assert_signed_out(&page);
    assert_eq!(stand.rows_of(&ada), Vec::<String>::new());
    assert!(
        !stand.github.requests_to(Endpoint::Token).is_empty(),
        "the server sent the code to GitHub"
    );
    assert_eq!(stand.github.tokens(), Vec::<String>::new());

    let again = browser.page();
    stand.sign_in(&again, &ada);
    assert_eq!(stand.account(&again).github_login, ada.login);
}

#[test]
fn github_link_with_a_wrong_verifier_links_nothing() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988220", "ada.verifier@example.com");
    let bob = google.github_user(5_310_448_221, "bob-verifier-standin", "bob.ver@example.org");

    let (page, who) = google_page(&google, &browser, &ada);
    held_github_link(&google, &page, &bob);
    let github = google.github();
    let asked = github.requests_to(Endpoint::Authorize);
    challenge_of(&asked[0]);

    let query = serde_urlencoded::to_string(&asked[0].params).expect("a query");
    let authorize = format!("{}/login/oauth/authorize?{query}", github.url());
    let foreign = with_param(&authorize, "code_challenge", Some(&foreign_challenge()));
    let visit = page.goto(&foreign);
    page.goto_settled(&github_authorize_link(
        github,
        &google.public,
        &page,
        &visit,
    ));

    assert_unlinked(&google, &page, &who, &bob, "a wrong verifier");
    assert!(
        !github.requests_to(Endpoint::Token).is_empty(),
        "the server sent the code to GitHub"
    );
    assert_eq!(github.tokens(), Vec::<String>::new());

    link_github(&google, &page, &bob);
    assert_eq!(methods(&google, &page), github_and_google(&bob));
}
