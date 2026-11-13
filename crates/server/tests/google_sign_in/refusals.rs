//! Callbacks that sign no one in and create no account: a missing, wrong, expired or reused `state`,
//! one from another browser, a provider error, a refused code, a refused client secret and an ID
//! token with another sign-in's `nonce`.

use std::thread;
use std::time::Duration;

use skald_oidc_stand_in::{Endpoint, Fault, Person};

use super::common::browser::{Browser, Page};
use super::common::sign_in::{with_param, wrong_states};
use super::{Google, Setup, WRONG_SECRET, param};

/// Opens each of `callbacks` in `page`'s browser as the browser is now: none signs anyone in.
fn none_signs_in(google: &Google, page: &Page, callbacks: &[String]) {
    let before = page.cookies();
    for callback in callbacks {
        page.add_cookies(&before);
        page.goto(callback);
        google.assert_signed_out(page);
    }
}

#[test]
fn callback_without_state_signs_no_one_in() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986700", "ada.nostate@example.com");
    let page = browser.page();
    let callback = google.held_callback(&page, &ada);

    let without = [
        with_param(&callback, "state", None),
        with_param(&callback, "state", Some("")),
    ];
    none_signs_in(&google, &page, &without);
    google.assert_no_account(&ada, "no state");
}

#[test]
fn callback_with_wrong_state_signs_no_one_in() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986701", "ada.wrongstate@example.com");
    let page = browser.page();
    let callback = google.held_callback(&page, &ada);
    let state = param(&callback, "state").expect("the callback has a state");

    let wrong: Vec<String> = wrong_states(&state)
        .iter()
        .map(|wrong| with_param(&callback, "state", Some(wrong)))
        .collect();
    none_signs_in(&google, &page, &wrong);
    google.assert_no_account(&ada, "a wrong state");
}

#[test]
fn state_works_only_in_its_own_browser() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986702", "ada.otherbrowser@example.com");
    let callback = google.held_callback(&browser.page(), &ada);

    // A browser that never started a sign-in.
    let fresh = browser.page();
    fresh.goto(&callback);
    google.assert_signed_out(&fresh);

    // A browser that started a sign-in of its own, as a person lured to someone else's callback.
    let lured = browser.page();
    google.held_callback(&lured, &ada);
    lured.goto(&callback);
    google.assert_signed_out(&lured);

    google.assert_no_account(&ada, "another browser's state");
}

#[test]
fn expired_state_signs_no_one_in() {
    let browser = Browser::launch();
    let google = Google::launch(Setup {
        args: &["--sign-in-state-lifetime", "5"],
        ..Setup::default()
    });
    let ada = google.person("110248495921238986703", "ada.expired@example.com");
    let bob = google.person("110248495921238986704", "bob.expired@example.com");

    // Within its lifetime a state works.
    google.signed_up(&browser, &ada);

    let slow = browser.page();
    let callback = google.held_callback(&slow, &bob);
    thread::sleep(Duration::from_secs(7));
    slow.goto(&callback);
    google.assert_signed_out(&slow);
    google.assert_no_account(&bob, "an expired state");
}

#[test]
fn reused_state_signs_no_one_in() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986705", "ada.reused@example.com");
    let bob = google.person("110248495921238986706", "bob.reused@example.com");

    let first = browser.page();
    let callback = google.held_callback(&first, &ada);
    let state = param(&callback, "state");
    // The browser as it was before the callback.
    let jar = first.cookies();
    first.goto(&callback);
    google.account(&first);

    // The same browser state and `state` again, with a new code the provider gives for that
    // authorization request.
    let again = browser.page();
    again.add_cookies(&jar);
    again.goto(&google.provider.asking_sign_in_url(&bob.subject));
    let authorize = &google.provider.requests_to(Endpoint::Authorize)[0];
    let query = serde_urlencoded::to_string(&authorize.params).expect("a query");
    let visit = again.goto(&format!("{}?{query}", google.provider.authorize_url()));
    let replay = google.authorize_link(&again, &visit);
    assert_eq!(param(&replay, "state"), state, "the used state");
    assert_ne!(
        param(&replay, "code"),
        param(&callback, "code"),
        "a new code"
    );

    again.goto(&replay);
    google.assert_signed_out(&again);
    google.assert_no_account(&bob, "a reused state");
}

/// Checks that a sign-in of `person` while the stand-in plays `fault` signs no one in and creates
/// no account.
fn refused(google: &Google, browser: &Browser, person: &Person, fault: Fault) {
    google.provider.set_faults(std::slice::from_ref(&fault));
    let page = browser.page();
    google.sign_in(&page, person);
    google.assert_signed_out(&page);
    google.assert_no_account(person, &format!("{fault:?}"));
    if fault != Fault::AccessDenied {
        assert!(
            !google.provider.requests_to(Endpoint::Token).is_empty(),
            "{fault:?}: the server sent the code to the provider"
        );
    }
}

/// Checks that `person` signs in once the provider plays no fault: a refusal left nothing behind.
fn signs_in_again(google: &Google, browser: &Browser, person: &Person) {
    google.provider.set_faults(&[]);
    google.signed_up(browser, person);
}

#[test]
fn provider_errors_sign_no_one_in() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986707", "ada.errors@example.com");

    for fault in [
        Fault::AccessDenied,
        Fault::TokenStatus { status: 500 },
        Fault::TokenStatus { status: 503 },
        Fault::TokenStatus { status: 404 },
    ] {
        refused(&google, &browser, &ada, fault);
    }
    signs_in_again(&google, &browser, &ada);
}

#[test]
fn refused_code_signs_no_one_in() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986708", "ada.refused@example.com");

    refused(&google, &browser, &ada, Fault::InvalidGrant);
    signs_in_again(&google, &browser, &ada);
}

#[test]
fn refused_client_secret_signs_no_one_in() {
    let browser = Browser::launch();
    let google = Google::launch(Setup {
        secret: Some(WRONG_SECRET),
        ..Setup::default()
    });
    let ada = google.person("110248495921238986709", "ada.secret@example.com");

    let page = browser.page();
    google.sign_in(&page, &ada);
    google.assert_signed_out(&page);
    google.assert_no_account(&ada, "a refused client secret");
    assert!(
        !google.provider.requests_to(Endpoint::Token).is_empty(),
        "the server sent the code to the provider"
    );
    assert!(google.provider.issued().is_empty(), "no token was issued");
}

#[test]
fn id_token_with_another_sign_ins_nonce_signs_no_one_in() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986710", "ada.nonce@example.com");
    let bob = google.person("110248495921238986711", "bob.nonce@example.com");

    // Ada's sign-in, held at the provider: its nonce.
    let ada_page = browser.page();
    let ada_callback = google.held_callback(&ada_page, &ada);
    let ada_nonce = google.provider.requests_to(Endpoint::Authorize)[0].params["nonce"].clone();

    // Bob's sign-in gets an ID token with Ada's sign-in's nonce.
    google
        .provider
        .set_faults(&[Fault::Nonce { nonce: ada_nonce }]);
    let bob_page = browser.page();
    google.sign_in(&bob_page, &bob);
    google.assert_signed_out(&bob_page);
    google.assert_no_account(&bob, "another sign-in's nonce");
    assert_eq!(
        google.provider.issued().len(),
        1,
        "the provider gave the ID token"
    );

    // Without the fault, both sign in, each with its own nonce.
    google.provider.set_faults(&[]);
    ada_page.goto(&ada_callback);
    google.account(&ada_page);
    google.signed_up(&browser, &bob);
}
