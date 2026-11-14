//! A sign-up refused on the device flow's verification page refuses its device code too: the
//! machine's poll gets the refusal in the `OAuthError` format at once, not when the code expires.

use super::common::browser::{Browser, SIGN_IN_GITHUB, SIGN_IN_GOOGLE};
use super::common::device_flow::{Grant, Machine, json, pause};
use super::*;

/// The `error`s that do not end a device flow, or say something other than a refusal.
const NOT_A_REFUSAL: [&str; 5] = [
    "authorization_pending",
    "slow_down",
    "expired_token",
    "invalid_grant",
    "invalid_request",
];

/// The person at `page`, signed in to the provider whose sign-in `link` finds, opens `grant`'s
/// verification page with its code and signs in there with that provider; gives where it ended.
fn sign_in_on_the_page(page: &Page, grant: &Grant, link: &str) -> Visit {
    page.goto_settled(&grant.verification_uri_complete);
    let offered = page.count(link);
    assert_eq!(
        offered,
        1,
        "{link} on the verification page: {}",
        page.text()
    );
    page.click_settled(&format!("{link} >> nth=0"))
}

/// Checks that polls for `grant` answer its refused sign-up: `400`, `application/json`, an
/// `OAuthError` whose `error` refuses and whose `error_description` says sign-up is closed; again on
/// a later poll. Each answer is checked against the spec by `machine`.
#[track_caller]
fn assert_poll_refused(machine: &Machine, grant: &Grant) {
    for poll in ["first", "later"] {
        pause(grant.interval);
        let response = machine.poll_response(&grant.device_code);
        assert_eq!(response.status, 400, "{poll} poll: {}", shown(&response));
        assert!(
            response.content_type.starts_with("application/json"),
            "{poll} poll: {}: {}",
            response.content_type,
            shown(&response)
        );
        let body = json(&response);
        let error = body["error"].as_str().expect("an error");
        assert!(!NOT_A_REFUSAL.contains(&error), "{poll} poll: {body}");
        let description = body["error_description"].as_str().unwrap_or_default();
        assert!(says_closed(description), "{poll} poll: {body}");
    }
}

/// A person whose GitHub login is not listed signs in with GitHub on the verification page: the
/// page refuses the sign-up, and the machine's poll gets the refusal.
#[test]
fn refused_github_sign_up_on_the_page_refuses_the_device_code() {
    let browser = Browser::launch();
    let google = closed(Lists {
        logins: "someone-else",
        ..Lists::default()
    });
    let machine = Machine::at(google.server.base());
    let bob = github_user(&google, 7_310_000_201, "bob-device");
    let grant = machine.authorize();
    machine.assert_pending(&grant);

    let page = browser.page();
    github_session(&page, google.github(), &bob);
    let visit = sign_in_on_the_page(&page, &grant, SIGN_IN_GITHUB);
    assert_refused(
        &google,
        &page,
        &visit,
        "an unlisted login on the verification page",
    );

    assert_poll_refused(&machine, &grant);
    assert_no_github_account(&google, &bob, "an unlisted login on the verification page");
}

/// A person whose listed Google email is not verified signs in with Google on the verification
/// page: the page refuses the sign-up, and the machine's poll gets the refusal.
#[test]
fn refused_google_sign_up_on_the_page_refuses_the_device_code() {
    let browser = Browser::launch();
    let google = closed(Lists {
        emails: "fay.device@example.com",
        ..Lists::default()
    });
    let machine = Machine::at(google.server.base());
    let fay = person(
        &google,
        "110248495921238991201",
        "fay.device@example.com",
        false,
        None,
    );
    let grant = machine.authorize();

    let page = browser.page();
    google.provider_session(&page, &fay);
    let visit = sign_in_on_the_page(&page, &grant, SIGN_IN_GOOGLE);
    assert_refused(
        &google,
        &page,
        &visit,
        "an unverified email on the verification page",
    );

    assert_poll_refused(&machine, &grant);
    google.assert_no_account(&fay, "an unverified email on the verification page");
}

/// The control: a listed login signs in on the verification page of a closed server and approves
/// the code; the machine gets a token of the new account.
#[test]
fn listed_login_signs_up_on_the_page_and_approves() {
    let browser = Browser::launch();
    let google = closed(Lists {
        logins: "ada-device",
        ..Lists::default()
    });
    let machine = Machine::at(google.server.base());
    let ada = github_user(&google, 7_310_000_202, "ada-device");
    let grant = machine.authorize();

    let page = browser.page();
    github_session(&page, google.github(), &ada);
    let visit = sign_in_on_the_page(&page, &grant, SIGN_IN_GITHUB);
    assert_eq!(visit.status, Some(200), "{visit:?}: {}", page.text());
    let token = machine.approve(&page, &grant);
    let who = machine.who_of(&token);
    assert_eq!(who.github_login.as_deref(), Some("ada-device"), "{who:?}");
}
