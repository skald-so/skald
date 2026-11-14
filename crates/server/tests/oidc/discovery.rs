//! The issuer and the provider's metadata (OpenID Connect Discovery 1.0, §4).

use serde_json::json;
use skald_oidc_stand_in::{Key, Provider};
use skald_server::{OidcError, OidcProvider};

use crate::{LONG, claims, discover, passes, settings};

#[test]
fn an_http_issuer_off_loopback_is_refused() {
    for issuer in [
        "http://accounts.skald.invalid",
        "http://127.0.0.1.skald.invalid",
        "http://localhost.skald.invalid",
        "ftp://accounts.skald.invalid",
        "accounts.skald.invalid",
        "not a URL",
        "",
    ] {
        match OidcProvider::discover(settings(issuer, LONG)) {
            Err(OidcError::Issuer(..)) => {}
            Err(other) => panic!("{issuer:?}: refused, but not as an issuer: {other:?}"),
            Ok(_) => panic!("{issuer:?}: taken as an issuer"),
        }
    }
}

#[test]
fn an_https_issuer_is_taken_and_then_unreachable() {
    // The scheme passes; the name resolves nowhere, so the provider gives no metadata.
    match OidcProvider::discover(settings("https://accounts.skald.invalid", LONG)) {
        Err(OidcError::Provider(..)) => {}
        Err(other) => panic!("an https issuer is refused as such: {other:?}"),
        Ok(_) => panic!("a provider that does not exist is discovered"),
    }
}

#[test]
fn a_loopback_http_issuer_is_discovered() {
    let (provider, _key, oidc) = crate::started();
    assert!(provider.issuer().starts_with("http://127.0.0.1:"));
    assert!(provider.metadata_reads() >= 1, "the metadata was not read");
    passes(&oidc, &provider.sign(&claims(provider.issuer())));
}

#[test]
fn an_issuer_with_a_path_is_discovered_under_it() {
    // A corporate IdP's issuer, such as Keycloak's realm.
    let provider = Provider::start_at("/realms/acme", &[Key::rsa()]);
    let oidc = discover(&provider, LONG);
    assert!(provider.metadata_reads() >= 1, "the metadata was not read");
    passes(&oidc, &provider.sign(&claims(provider.issuer())));
}

#[test]
fn metadata_with_another_issuer_is_refused() {
    let provider = Provider::start(&[Key::rsa()]);
    let issuer = provider.issuer().to_owned();
    for other in [
        json!("https://accounts.google.com"),
        json!(format!("{issuer}/other")),
        json!(issuer.replacen("http:", "https:", 1)),
        json!(null),
    ] {
        let mut metadata = provider.metadata();
        metadata["issuer"] = other.clone();
        provider.set_metadata(&metadata);
        let result = OidcProvider::discover(settings(&issuer, LONG));
        assert!(result.is_err(), "metadata with the issuer {other} is taken");
    }
    // The same metadata with its own issuer is taken.
    provider.set_metadata(&provider.metadata());
    discover(&provider, LONG);
}

#[test]
fn malformed_metadata_gives_an_error() {
    let provider = Provider::start(&[Key::rsa()]);
    let issuer = provider.issuer().to_owned();
    let without = |field: &str| {
        let mut metadata = provider.metadata();
        metadata
            .as_object_mut()
            .expect("metadata is an object")
            .remove(field);
        metadata.to_string()
    };
    let mut bad_jwks_uri = provider.metadata();
    bad_jwks_uri["jwks_uri"] = json!("not a URL");
    for (what, body) in [
        ("no JSON", "{\"issuer\":".to_owned()),
        ("an empty body", String::new()),
        ("a list", "[]".to_owned()),
        (
            "an issuer that is no text",
            json!({ "issuer": 5 }).to_string(),
        ),
        ("no jwks_uri", without("jwks_uri")),
        ("a jwks_uri that is no URL", bad_jwks_uri.to_string()),
        (
            "no id_token_signing_alg_values_supported",
            without("id_token_signing_alg_values_supported"),
        ),
    ] {
        provider.set_metadata_body(&body);
        match OidcProvider::discover(settings(&issuer, LONG)) {
            Err(OidcError::Provider(..)) => {}
            Err(other) => panic!("{what}: refused, but not as the provider's fault: {other:?}"),
            Ok(_) => panic!("{what}: taken as metadata"),
        }
    }
}

/// Asserts that discovering `issuer` is refused as the provider's fault; `what` names the case.
#[track_caller]
fn refused_provider(issuer: &str, what: &str) {
    match OidcProvider::discover(settings(issuer, LONG)) {
        Err(OidcError::Provider(..)) => {}
        Err(other) => panic!("{what}: refused, but not as the provider's fault: {other:?}"),
        Ok(_) => panic!("{what}: the provider is discovered"),
    }
}

#[test]
fn a_jwks_uri_neither_https_nor_http_on_loopback_is_refused() {
    let provider = Provider::start(&[Key::rsa()]);
    let issuer = provider.issuer().to_owned();
    // `localhost` reaches the stand-in's keys, but a name is not known to be loopback.
    let localhost = format!("{}/keys", issuer.replacen("127.0.0.1", "localhost", 1));
    for jwks_uri in [
        localhost.as_str(),
        "http://keys.skald.invalid/keys",
        "ftp://keys.skald.invalid/keys",
    ] {
        let mut metadata = provider.metadata();
        metadata["jwks_uri"] = json!(jwks_uri);
        provider.set_metadata(&metadata);
        refused_provider(&issuer, jwks_uri);
    }
    assert_eq!(
        provider.key_reads(),
        0,
        "keys were read off a refused jwks_uri"
    );
}

#[test]
fn an_authorization_or_token_endpoint_neither_https_nor_http_on_loopback_is_refused() {
    let provider = Provider::start(&[Key::rsa()]);
    let issuer = provider.issuer().to_owned();
    for (field, path) in [
        ("authorization_endpoint", "authorize"),
        ("token_endpoint", "token"),
    ] {
        // `localhost` reaches the stand-in, but a name is not known to be loopback.
        let localhost = format!("{}/{path}", issuer.replacen("127.0.0.1", "localhost", 1));
        for endpoint in [
            localhost,
            format!("http://accounts.skald.invalid/{path}"),
            format!("ftp://accounts.skald.invalid/{path}"),
        ] {
            let mut metadata = provider.metadata();
            metadata[field] = json!(endpoint);
            provider.set_metadata(&metadata);
            refused_provider(&issuer, &format!("{field} {endpoint}"));
        }
    }
    // The same metadata with the stand-in's own endpoints is taken.
    provider.set_metadata(&provider.metadata());
    discover(&provider, LONG);
}

#[test]
fn a_redirect_from_the_metadata_or_the_keys_is_not_followed() {
    for status in [301, 302, 303, 307, 308] {
        // Each redirect leads to a second stand-in whose answer would pass every check.
        let key = Key::rsa();
        let provider = Provider::start(std::slice::from_ref(&key));
        let elsewhere = Provider::start(std::slice::from_ref(&key));
        elsewhere.set_metadata(&provider.metadata());
        provider.redirect_metadata(
            status,
            &format!("{}/.well-known/openid-configuration", elsewhere.issuer()),
        );
        refused_provider(
            provider.issuer(),
            &format!("metadata redirected by {status}"),
        );
        assert_eq!(
            elsewhere.metadata_reads(),
            0,
            "a {status} redirect from the metadata is followed"
        );

        let provider = Provider::start(std::slice::from_ref(&key));
        let elsewhere = Provider::start(std::slice::from_ref(&key));
        provider.redirect_keys(status, &format!("{}/keys", elsewhere.issuer()));
        refused_provider(provider.issuer(), &format!("keys redirected by {status}"));
        assert_eq!(
            elsewhere.key_reads(),
            0,
            "a {status} redirect from the keys is followed"
        );
    }
}

#[test]
fn an_unreachable_provider_gives_an_error() {
    let mut provider = Provider::start(&[Key::rsa()]);
    let issuer = provider.issuer().to_owned();
    provider.stop();
    match OidcProvider::discover(settings(&issuer, LONG)) {
        Err(OidcError::Provider(..)) => {}
        Err(other) => panic!("refused, but not as the provider's fault: {other:?}"),
        Ok(_) => panic!("a provider that is down is discovered"),
    }
}
