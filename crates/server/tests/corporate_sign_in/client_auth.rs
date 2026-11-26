use serde_json::json;
use skald_oidc_stand_in::{Endpoint, Key, Person, Provider, TokenAuth};

use super::common::browser::Browser;
use super::common::google::Google;
use super::common::sign_in::page_session;
use super::{START, Site, TestDb, assert_conforms};

const METHODS: &str = "token_endpoint_auth_methods_supported";

#[test]
fn the_token_exchange_uses_the_discovered_client_authentication_method() {
    let browser = Browser::launch();
    let key = Key::rsa();
    let person = Person::new("worker", "worker@example.test");
    for (advertised, required) in [
        (None, TokenAuth::Basic),
        (Some(vec!["client_secret_basic"]), TokenAuth::Basic),
        (Some(vec!["client_secret_post"]), TokenAuth::Post),
        (
            Some(vec!["client_secret_post", "client_secret_basic"]),
            TokenAuth::Basic,
        ),
    ] {
        let db = TestDb::create();
        let provider = Provider::start(std::slice::from_ref(&key));
        let mut metadata = provider.metadata();
        match &advertised {
            Some(methods) => metadata[METHODS] = json!(methods),
            None => {
                metadata.as_object_mut().unwrap().remove(METHODS);
            }
        }
        provider.set_metadata(&metadata);
        provider.require_token_auth(required);
        let site = Site::start(&db, &provider, None, false);
        let page = browser.page();
        let visit = site.sign_in(&page, &provider, &person, START);
        assert_eq!(
            visit.last(),
            format!("{}/health", site.web),
            "{advertised:?}"
        );
        assert_eq!(
            page_session(&page, &site.web).1["sign_in_method"],
            "corporate"
        );
        assert_eq!(
            provider.requests_to(Endpoint::Token).len(),
            1,
            "{advertised:?}"
        );
    }
}

#[test]
fn an_issuer_with_no_supported_client_authentication_method_is_refused() {
    let db = TestDb::create();
    let provider = Provider::start(&[Key::rsa()]);
    let mut metadata = provider.metadata();
    metadata[METHODS] = json!(["private_key_jwt"]);
    provider.set_metadata(&metadata);
    let site = Site::start(&db, &provider, None, false);
    let response = site.server.get(START);
    assert_eq!(response.status, 502);
    assert_conforms(START, &response);
    assert!(provider.requests_to(Endpoint::Token).is_empty());
}

#[test]
fn google_keeps_its_registered_post_authentication_method() {
    let browser = Browser::launch();
    let site = Google::start();
    site.provider.require_token_auth(TokenAuth::Post);
    let person = site.person("google-worker", "google-worker@example.test");
    assert_eq!(site.signed_up(&browser, &person).username, "google-worker");
}
