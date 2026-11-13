//! The provider's keys from its `jwks_uri`: read again when a token names a key they lack, at most
//! once per interval.

use std::thread;
use std::time::Duration;

use serde_json::json;
use skald_oidc_stand_in::{Key, Provider, signed};
use skald_server::{OidcError, OidcProvider};

use crate::{LONG, NONCE, claims, discover, passes, refused, settings};

#[test]
fn a_rotated_key_passes_without_a_restart() {
    let old = Key::rsa();
    let provider = Provider::start(std::slice::from_ref(&old));
    let interval = Duration::from_millis(300);
    let oidc = discover(&provider, interval);
    passes(&oidc, &provider.sign(&claims(provider.issuer())));

    // The provider rotates: the new key signs, the old one is still listed for a while.
    let new = Key::rsa();
    provider.set_keys(&[new.clone(), old.clone()]);
    thread::sleep(interval * 2);
    passes(&oidc, &provider.sign(&claims(provider.issuer())));
    passes(&oidc, &signed(&old, json!({}), &claims(provider.issuer())));

    // Then it rotates again, and a third key signs.
    let newer = Key::rsa();
    provider.set_keys(&[newer, new]);
    thread::sleep(interval * 2);
    passes(&oidc, &provider.sign(&claims(provider.issuer())));
}

#[test]
fn unknown_key_ids_read_the_keys_at_most_once_per_interval() {
    let (provider, key, oidc) = crate::started();
    let before = provider.key_reads();
    let claims = claims(provider.issuer());
    // Forged key IDs from many requests at once.
    thread::scope(|scope| {
        for thread in 0..8 {
            let (oidc, key, claims) = (&oidc, &key, &claims);
            scope.spawn(move || {
                for token in 0..25 {
                    let forged = key.with_kid(&format!("forged-{thread}-{token}"));
                    refused(oidc, &signed(&forged, json!({}), claims), "a forged key ID");
                }
            });
        }
    });
    let reads = provider.key_reads() - before;
    assert!(
        reads <= 1,
        "200 tokens with unknown key IDs read the keys {reads} times within the interval"
    );
    passes(&oidc, &provider.sign(&claims));
}

#[test]
fn unknown_key_ids_read_the_keys_again_after_the_interval() {
    let key = Key::rsa();
    let provider = Provider::start(std::slice::from_ref(&key));
    let interval = Duration::from_millis(500);
    let oidc = discover(&provider, interval);
    thread::sleep(interval * 2);
    for round in 0..3 {
        let before = provider.key_reads();
        for token in 0..20 {
            let forged = key.with_kid(&format!("forged-{round}-{token}"));
            refused(
                &oidc,
                &signed(&forged, json!({}), &claims(provider.issuer())),
                "forged",
            );
        }
        let reads = provider.key_reads() - before;
        assert!(
            (1..=2).contains(&reads),
            "round {round}: 20 tokens with unknown key IDs read the keys {reads} times"
        );
        thread::sleep(interval * 2);
    }
}

#[test]
fn a_known_key_is_not_read_again() {
    let (provider, _key, oidc) = crate::started();
    passes(&oidc, &provider.sign(&claims(provider.issuer())));
    let before = provider.key_reads();
    for _ in 0..20 {
        passes(&oidc, &provider.sign(&claims(provider.issuer())));
    }
    assert_eq!(
        provider.key_reads(),
        before,
        "valid tokens read the keys again"
    );
}

#[test]
fn with_the_provider_down_known_keys_pass_and_unknown_ones_give_an_error() {
    let key = Key::rsa();
    let mut provider = Provider::start(std::slice::from_ref(&key));
    let interval = Duration::from_millis(100);
    let oidc = discover(&provider, interval);
    let claims = claims(provider.issuer());
    let token = provider.sign(&claims);
    provider.stop();
    thread::sleep(interval * 3);
    passes(&oidc, &token);
    let unknown = signed(&Key::rsa(), json!({}), &claims);
    let result = oidc.check(&unknown, NONCE);
    assert!(
        result.is_err(),
        "a token by an unknown key passes: {result:?}"
    );
    // The failed read keeps the keys read before.
    passes(&oidc, &token);
}

#[test]
fn malformed_keys_give_an_error() {
    let key = Key::rsa();
    let valid_jwk = key.jwk();
    let mut broken_n = valid_jwk.clone();
    broken_n["n"] = json!("!!not base64url!!");
    let mut no_n = valid_jwk.clone();
    no_n.as_object_mut().expect("a JWK object").remove("n");
    for (what, body) in [
        ("no JSON", "{\"keys\": [".to_owned()),
        ("an empty body", String::new()),
        ("a list", "[]".to_owned()),
        ("keys that are no list", json!({ "keys": 1 }).to_string()),
        (
            "a key that is no object",
            json!({ "keys": ["key"] }).to_string(),
        ),
        (
            "a modulus that is no base64url",
            json!({ "keys": [broken_n] }).to_string(),
        ),
        (
            "a key without its modulus",
            json!({ "keys": [no_n] }).to_string(),
        ),
    ] {
        let provider = Provider::start(std::slice::from_ref(&key));
        provider.set_keys_body(&body);
        // The keys may be read at discovery or at the first check: either refuses.
        match OidcProvider::discover(settings(provider.issuer(), LONG)) {
            Err(OidcError::Provider(..)) => {}
            Err(other) => panic!("{what}: refused, but not as the provider's fault: {other:?}"),
            Ok(oidc) => {
                let result = oidc.check(&provider.sign(&claims(provider.issuer())), NONCE);
                assert!(result.is_err(), "{what}: a token passes: {result:?}");
            }
        }
    }
}
