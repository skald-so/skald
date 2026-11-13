//! The claims of an ID token, and tokens that are no JWS at all.

use serde_json::json;
use skald_oidc_stand_in::{Key, base64url};
use skald_server::OidcError;

use crate::{CLIENT_ID, EMAIL, NONCE, SUBJECT, claims, now, passes, refused, started, with};

#[test]
fn a_valid_token_gives_its_subject_and_email() {
    let (provider, _key, oidc) = started();
    let identity = passes(&oidc, &provider.sign(&claims(provider.issuer())));
    assert_eq!(identity.subject, SUBJECT);
    assert_eq!(identity.email.as_deref(), Some(EMAIL));

    let another = provider.sign(&with(
        &claims(provider.issuer()),
        json!({ "sub": "2049", "email": "grace@example.org" }),
    ));
    let identity = oidc
        .check(&another, NONCE)
        .expect("another person's valid token");
    assert_eq!(identity.subject, "2049");
    assert_eq!(identity.email.as_deref(), Some("grace@example.org"));
}

#[test]
fn a_token_without_azp_or_email_gives_its_subject() {
    let (provider, _key, oidc) = started();
    let token = provider.sign(&with(
        &claims(provider.issuer()),
        json!({ "azp": null, "email": null, "email_verified": null }),
    ));
    let identity = passes(&oidc, &token);
    assert_eq!(identity.email, None);
}

#[test]
fn an_audience_list_of_the_client_id_alone_passes() {
    let (provider, _key, oidc) = started();
    let token = provider.sign(&with(
        &claims(provider.issuer()),
        json!({ "aud": [CLIENT_ID] }),
    ));
    passes(&oidc, &token);
}

#[test]
fn a_token_within_a_small_clock_skew_passes() {
    let (provider, _key, oidc) = started();
    let now = now();
    let ahead = with(&claims(provider.issuer()), json!({ "iat": now + 5 }));
    passes(&oidc, &provider.sign(&ahead));
    let just_expired = with(
        &claims(provider.issuer()),
        json!({ "iat": now - 3600, "exp": now - 5 }),
    );
    passes(&oidc, &provider.sign(&just_expired));
}

#[test]
fn another_issuer_is_refused() {
    let (provider, _key, oidc) = started();
    let issuer = provider.issuer();
    for (what, iss) in [
        ("Google's issuer", json!("https://accounts.google.com")),
        (
            "the issuer with a trailing slash",
            json!(format!("{issuer}/")),
        ),
        ("a path under the issuer", json!(format!("{issuer}/other"))),
        (
            "the issuer over https",
            json!(issuer.replacen("http:", "https:", 1)),
        ),
        ("no iss", json!(null)),
    ] {
        let token = provider.sign(&with(&claims(issuer), json!({ "iss": iss })));
        refused(&oidc, &token, what);
    }
}

#[test]
fn an_audience_without_the_client_id_is_refused() {
    let (provider, _key, oidc) = started();
    for (what, aud) in [
        ("another client", json!("another-client.apps.example")),
        (
            "a list of another client",
            json!(["another-client.apps.example"]),
        ),
        (
            "the client ID with a suffix",
            json!(format!("{CLIENT_ID}x")),
        ),
        ("an empty list", json!([])),
        ("no aud", json!(null)),
    ] {
        let token = provider.sign(&with(&claims(provider.issuer()), json!({ "aud": aud })));
        refused(&oidc, &token, what);
    }
}

#[test]
fn an_audience_with_the_client_id_and_another_is_refused() {
    let (provider, _key, oidc) = started();
    for aud in [
        json!([CLIENT_ID, "another-client.apps.example"]),
        json!(["another-client.apps.example", CLIENT_ID]),
    ] {
        for azp in [json!(CLIENT_ID), json!(null)] {
            let token = provider.sign(&with(
                &claims(provider.issuer()),
                json!({ "aud": aud, "azp": azp }),
            ));
            refused(&oidc, &token, &format!("aud {aud}, azp {azp}"));
        }
    }
}

#[test]
fn another_authorized_party_is_refused() {
    let (provider, _key, oidc) = started();
    for azp in ["another-client.apps.example", ""] {
        let token = provider.sign(&with(&claims(provider.issuer()), json!({ "azp": azp })));
        refused(&oidc, &token, &format!("azp {azp:?}"));
    }
}

#[test]
fn an_expired_token_is_refused() {
    let (provider, _key, oidc) = started();
    let now = now();
    for exp in [now - 600, now - 86_400, 0] {
        let token = provider.sign(&with(
            &claims(provider.issuer()),
            json!({ "iat": now - 90_000, "exp": exp }),
        ));
        refused(&oidc, &token, &format!("exp {exp}, now {now}"));
    }
}

#[test]
fn a_token_issued_ahead_is_refused() {
    let (provider, _key, oidc) = started();
    let now = now();
    for iat in [now + 600, now + 86_400] {
        let token = provider.sign(&with(
            &claims(provider.issuer()),
            json!({ "iat": iat, "exp": iat + 3600 }),
        ));
        refused(&oidc, &token, &format!("iat {iat}, now {now}"));
    }
}

#[test]
fn a_wrong_or_missing_nonce_is_refused() {
    let (provider, _key, oidc) = started();
    for (what, nonce) in [
        ("another sign-in's nonce", json!("another-sign-in-nonce")),
        ("the nonce with a suffix", json!(format!("{NONCE}x"))),
        ("an empty nonce", json!("")),
        ("no nonce", json!(null)),
    ] {
        let token = provider.sign(&with(&claims(provider.issuer()), json!({ "nonce": nonce })));
        refused(&oidc, &token, what);
    }
    // A valid token of this sign-in, checked for another sign-in.
    let token = provider.sign(&claims(provider.issuer()));
    for nonce in ["another-sign-in-nonce", ""] {
        let result = oidc.check(&token, nonce);
        assert!(
            matches!(result, Err(OidcError::Token(..))),
            "checked for the nonce {nonce:?}: {result:?}"
        );
    }
}

#[test]
fn a_token_without_a_required_claim_is_refused() {
    let (provider, _key, oidc) = started();
    for claim in ["iss", "sub", "aud", "exp", "iat"] {
        let mut removed = serde_json::Map::new();
        removed.insert(claim.to_owned(), json!(null));
        let token = provider.sign(&with(&claims(provider.issuer()), removed.into()));
        refused(&oidc, &token, &format!("no {claim}"));
    }
}

#[test]
fn claims_of_the_wrong_type_are_refused() {
    let (provider, _key, oidc) = started();
    for (what, over) in [
        ("exp as text", json!({ "exp": "soon" })),
        ("iat as text", json!({ "iat": "now" })),
        ("aud as a number", json!({ "aud": 5 })),
        ("sub as a number", json!({ "sub": 110169484 })),
        ("iss as an object", json!({ "iss": { "url": "x" } })),
        ("nonce as a list", json!({ "nonce": [NONCE] })),
    ] {
        let token = provider.sign(&with(&claims(provider.issuer()), over));
        refused(&oidc, &token, what);
    }
}

#[test]
fn malformed_tokens_are_refused() {
    let (provider, key, oidc) = started();
    let valid = provider.sign(&claims(provider.issuer()));
    let parts: Vec<&str> = valid.split('.').collect();
    let header = json!({ "alg": "RS256", "kid": key.kid() }).to_string();
    let claims_text = claims(provider.issuer()).to_string();
    let cases = [
        ("an empty token", String::new()),
        ("dots alone", "..".to_owned()),
        ("one part", parts[0].to_owned()),
        ("two parts", format!("{}.{}", parts[0], parts[1])),
        ("four parts", format!("{valid}.{}", parts[2])),
        (
            "five parts, as an encrypted JWT",
            format!("{valid}.{}.{}", parts[1], parts[2]),
        ),
        ("a header that is no base64url", format!("*{}", &valid[1..])),
        (
            "a payload that is no base64url",
            format!("{}.!!!.{}", parts[0], parts[2]),
        ),
        (
            "a signature that is no base64url",
            format!("{}.{}.!!!", parts[0], parts[1]),
        ),
        ("a padded signature", format!("{valid}==")),
        (
            "a header that is no JSON",
            signed_parts("{alg", &claims_text, &key),
        ),
        (
            "a header that is a list",
            signed_parts("[\"RS256\"]", &claims_text, &key),
        ),
        (
            "a payload that is no JSON",
            signed_parts(&header, "{\"sub\":", &key),
        ),
        (
            "a payload that is a list",
            signed_parts(&header, "[]", &key),
        ),
        (
            "a payload that is null",
            signed_parts(&header, "null", &key),
        ),
        ("the payload of another token under this one's signature", {
            let other = provider.sign(&with(&claims(provider.issuer()), json!({ "sub": "1" })));
            let other: Vec<&str> = other.split('.').collect();
            format!("{}.{}.{}", parts[0], other[1], parts[2])
        }),
        ("a megabyte of text", "a".repeat(1 << 20)),
    ];
    for (what, token) in cases {
        refused(&oidc, &token, what);
    }
    // The valid token still passes: the cases above fail on their own faults.
    passes(&oidc, &valid);
}

/// A token of the raw `header` and `payload` texts, signed by `key`.
fn signed_parts(header: &str, payload: &str, key: &Key) -> String {
    let input = format!("{}.{}", base64url(header), base64url(payload));
    format!("{input}.{}", base64url(key.signature(input.as_bytes())))
}

#[test]
fn every_truncation_of_a_token_is_refused() {
    let (provider, _key, oidc) = started();
    let valid = provider.sign(&claims(provider.issuer()));
    for end in 0..valid.len() {
        let result = oidc.check(&valid[..end], NONCE);
        assert!(
            result.is_err(),
            "the first {end} characters pass: {result:?}"
        );
    }
    passes(&oidc, &valid);
}

#[test]
fn every_flipped_byte_of_a_token_is_refused() {
    let (provider, _key, oidc) = started();
    let valid = provider.sign(&claims(provider.issuer()));
    let parts: Vec<Vec<u8>> = valid
        .split('.')
        .map(|part| {
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, part)
                .expect("base64url")
        })
        .collect();
    for part in 0..3 {
        for byte in 0..parts[part].len() {
            let mut flipped = parts.clone();
            flipped[part][byte] ^= 0x01;
            let token = flipped.iter().map(base64url).collect::<Vec<_>>().join(".");
            let result = oidc.check(&token, NONCE);
            assert!(
                result.is_err(),
                "byte {byte} of part {part} flipped passes: {result:?}"
            );
        }
    }
    passes(&oidc, &valid);
}
