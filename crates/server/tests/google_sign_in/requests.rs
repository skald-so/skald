//! What the server asks the provider: the authorization request (OpenID Connect Core 1.0,
//! §3.1.2.1) with `openid` and `email` only, `state`, `nonce` and an S256 PKCE challenge (RFC 7636),
//! and the token request with the PKCE verifier.

use std::collections::{BTreeMap, BTreeSet};

use ring::digest::{SHA256, digest};
use skald_oidc_stand_in::{Endpoint, base64url};

use super::common::browser::Browser;
use super::{CALLBACK, CLIENT_ID, Google};

/// Whether `text` is a PKCE verifier (RFC 7636, §4.1): 43 to 128 unreserved characters.
fn is_verifier(text: &str) -> bool {
    (43..=128).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

/// Signs two people in, each once; checks that each is signed in.
fn two_sign_ins(google: &Google, browser: &Browser) {
    for (subject, email) in [
        ("110248495921238986600", "ada.requests@example.com"),
        ("110248495921238986601", "bob.requests@example.com"),
    ] {
        let person = google.person(subject, email);
        google.signed_up(browser, &person);
    }
}

#[test]
fn authorization_request_asks_for_openid_and_email_only_with_state_nonce_and_pkce_challenge() {
    let browser = Browser::launch();
    let google = Google::start();
    two_sign_ins(&google, &browser);

    let authorize = google.provider.requests_to(Endpoint::Authorize);
    assert_eq!(authorize.len(), 2, "{authorize:?}");
    let callback = format!("{}{CALLBACK}", google.public);
    let fixed = BTreeMap::from([
        ("client_id", CLIENT_ID),
        ("redirect_uri", callback.as_str()),
        ("response_type", "code"),
        ("code_challenge_method", "S256"),
    ]);
    for request in &authorize {
        let param = |name: &str| request.params.get(name).map(String::as_str);
        let sent: BTreeMap<&str, &str> = fixed
            .keys()
            .map(|name| (*name, param(name).unwrap_or_default()))
            .collect();
        assert_eq!(sent, fixed, "{request:?}");
        let scopes: BTreeSet<&str> = param("scope")
            .unwrap_or_default()
            .split(' ')
            .filter(|scope| !scope.is_empty())
            .collect();
        assert_eq!(scopes, BTreeSet::from(["openid", "email"]), "{request:?}");
        for name in ["state", "nonce"] {
            assert!(
                param(name).is_some_and(|value| value.len() >= 16),
                "{name}: {request:?}"
            );
        }
        let challenge = param("code_challenge").unwrap_or_default();
        assert!(
            challenge.len() == 43
                && challenge
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "an S256 challenge is 43 characters of base64url: {request:?}"
        );
    }
    for name in ["state", "nonce", "code_challenge"] {
        assert_ne!(
            authorize[0].params[name], authorize[1].params[name],
            "each sign-in has a {name} of its own"
        );
    }
}

#[test]
fn token_request_carries_the_pkce_verifier() {
    let browser = Browser::launch();
    let google = Google::start();
    two_sign_ins(&google, &browser);

    let authorize = google.provider.requests_to(Endpoint::Authorize);
    let token = google.provider.requests_to(Endpoint::Token);
    let codes = google.provider.codes();
    assert_eq!(token.len(), 2, "{token:?}");
    assert_eq!(codes.len(), 2, "{codes:?}");
    assert_eq!(
        google.provider.issued().len(),
        2,
        "each code got its tokens"
    );
    let callback = format!("{}{CALLBACK}", google.public);
    for ((asked, exchanged), code) in authorize.iter().zip(&token).zip(&codes) {
        let param = |name: &str| exchanged.params.get(name).map(String::as_str);
        assert_eq!(
            param("grant_type"),
            Some("authorization_code"),
            "{exchanged:?}"
        );
        assert_eq!(param("code"), Some(code.as_str()), "{exchanged:?}");
        assert_eq!(
            param("redirect_uri"),
            Some(callback.as_str()),
            "{exchanged:?}"
        );
        let verifier = param("code_verifier").unwrap_or_default();
        assert!(is_verifier(verifier), "a PKCE verifier: {exchanged:?}");
        assert_eq!(
            base64url(digest(&SHA256, verifier.as_bytes())),
            asked.params["code_challenge"],
            "the verifier of its sign-in's challenge"
        );
    }
    assert_ne!(
        token[0].params["code_verifier"], token[1].params["code_verifier"],
        "each sign-in has a verifier of its own"
    );
}
