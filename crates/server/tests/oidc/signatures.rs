//! The signature of an ID token: by a key the provider lists, with an asymmetric algorithm its
//! metadata lists; never `none` or an HMAC.

use serde_json::json;
use skald_oidc_stand_in::{Key, Provider, base64url, hmac, signed, unsigned};

use crate::{CLIENT_SECRET, LONG, claims, discover, passes, refused, started};

#[test]
fn a_key_the_provider_does_not_list_is_refused() {
    let (provider, key, oidc) = started();
    let claims = claims(provider.issuer());
    let other = Key::rsa();
    for (what, header) in [
        ("its own key ID", json!({})),
        ("no key ID", json!({ "kid": null })),
        (
            "the key ID of the provider's key",
            json!({ "kid": key.kid() }),
        ),
    ] {
        refused(&oidc, &signed(&other, header, &claims), what);
    }
    let other_ec = Key::ec();
    refused(
        &oidc,
        &signed(&other_ec, json!({ "kid": key.kid() }), &claims),
        "an ES256 key under the provider's key ID",
    );
}

#[test]
fn a_broken_signature_is_refused() {
    let (provider, _key, oidc) = started();
    let valid = provider.sign(&claims(provider.issuer()));
    let (input, signature) = valid.rsplit_once('.').expect("a compact JWS");
    let mut bytes =
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, signature)
            .expect("base64url");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x80;
    refused(
        &oidc,
        &format!("{input}.{}", base64url(&bytes)),
        "a flipped bit",
    );
    refused(&oidc, &format!("{input}."), "an empty signature");
    refused(
        &oidc,
        &format!("{input}.{}", base64url(&bytes[..last])),
        "a signature a byte short",
    );
    refused(
        &oidc,
        &format!("{input}.{}", base64url([0; 256])),
        "a signature of zeros",
    );
    passes(&oidc, &valid);
}

#[test]
fn alg_none_is_refused() {
    let (provider, key, oidc) = started();
    let claims = claims(provider.issuer());
    let valid = provider.sign(&claims);
    let (_, signature) = valid.rsplit_once('.').expect("a compact JWS");
    for alg in ["none", "None", "NONE", "nOnE"] {
        for header in [json!({}), json!({ "kid": key.kid() })] {
            let token = unsigned(alg, header.clone(), &claims);
            refused(&oidc, &token, &format!("alg {alg}, header {header}"));
            // The provider's real signature under an alg of none.
            let (input, _) = token.rsplit_once('.').expect("a compact JWS");
            refused(
                &oidc,
                &format!("{input}.{signature}"),
                &format!("alg {alg} with a signature, header {header}"),
            );
        }
    }
}

#[test]
fn an_hmac_keyed_with_the_providers_public_key_is_refused() {
    let (provider, key, oidc) = started();
    let claims = claims(provider.issuer());
    for (form, secret) in key.public_forms() {
        for alg in ["HS256", "HS384", "HS512"] {
            for header in [json!({ "kid": key.kid() }), json!({})] {
                let token = hmac(alg, &secret, header.clone(), &claims);
                refused(
                    &oidc,
                    &token,
                    &format!("{alg} keyed with the {form}, header {header}"),
                );
            }
        }
    }
}

#[test]
fn an_hs256_token_keyed_with_the_client_secret_is_refused() {
    let (provider, key, oidc) = started();
    let claims = claims(provider.issuer());
    for alg in ["HS256", "HS384", "HS512"] {
        for header in [json!({}), json!({ "kid": key.kid() })] {
            let token = hmac(alg, CLIENT_SECRET.as_bytes(), header.clone(), &claims);
            refused(
                &oidc,
                &token,
                &format!("{alg} keyed with the client secret, header {header}"),
            );
        }
    }
}

#[test]
fn none_and_hmac_are_refused_even_when_the_metadata_lists_them() {
    let key = Key::rsa();
    let provider = Provider::start(std::slice::from_ref(&key));
    // As corporate IdPs such as Keycloak list them: the provider is still usable, by RS256 alone.
    provider.set_algs(&["RS256", "HS256", "HS384", "HS512", "none"]);
    let oidc = discover(&provider, LONG);
    let claims = claims(provider.issuer());
    passes(&oidc, &provider.sign(&claims));
    refused(&oidc, &unsigned("none", json!({}), &claims), "alg none");
    for alg in ["HS256", "HS384", "HS512"] {
        let token = hmac(alg, CLIENT_SECRET.as_bytes(), json!({}), &claims);
        refused(
            &oidc,
            &token,
            &format!("{alg} keyed with the client secret"),
        );
        for (form, secret) in key.public_forms() {
            let token = hmac(alg, &secret, json!({ "kid": key.kid() }), &claims);
            refused(&oidc, &token, &format!("{alg} keyed with the {form}"));
        }
    }
}

#[test]
fn an_algorithm_the_metadata_does_not_list_is_refused() {
    let rsa = Key::rsa();
    let ec = Key::ec();
    // The provider lists the P-256 key, but its metadata lists RS256 alone.
    let provider = Provider::start(&[rsa, ec.clone()]);
    let oidc = discover(&provider, LONG);
    let token = signed(&ec, json!({}), &claims(provider.issuer()));
    refused(&oidc, &token, "ES256, which the metadata does not list");
}

#[test]
fn an_asymmetric_algorithm_the_metadata_lists_passes() {
    let ec = Key::ec();
    let provider = Provider::start(std::slice::from_ref(&ec));
    provider.set_algs(&["ES256"]);
    let oidc = discover(&provider, LONG);
    passes(&oidc, &provider.sign(&claims(provider.issuer())));
}

#[test]
fn a_key_the_token_carries_is_not_trusted() {
    let (provider, key, oidc) = started();
    let claims = claims(provider.issuer());
    let attacker = Key::rsa();
    // The attacker's own provider, listing the attacker's key.
    let elsewhere = Provider::start(std::slice::from_ref(&attacker));
    let jku = format!("{}/keys", elsewhere.issuer());
    for (what, header) in [
        ("an embedded jwk", json!({ "jwk": attacker.jwk() })),
        (
            "an embedded jwk under the provider's key ID",
            json!({ "jwk": attacker.with_kid(key.kid()).jwk(), "kid": key.kid() }),
        ),
        ("a jku of the attacker's keys", json!({ "jku": jku })),
        (
            "a jku of the attacker's keys under the provider's key ID",
            json!({ "jku": jku, "kid": key.kid() }),
        ),
    ] {
        refused(&oidc, &signed(&attacker, header, &claims), what);
    }
    assert_eq!(elsewhere.key_reads(), 0, "the attacker's keys were read");
}
