//! A web session expires 30 days after its sign-in, and its cookie lasts until then; a device
//! session never expires.

use crate::common::browser::Browser;
use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::common::google::{Google, HOST, Setup};
use crate::sessions::{
    Api, CHROME_ON_MACOS, COOKIE, FIREFOX_ON_WINDOWS, Kind, LIFETIME, Laptop, now, token_of,
    web_sign_in,
};

/// Ada's web sessions, one whose sign-in is moved back 30 days and one moved back 29 days and 23
/// hours, and her laptop's device session, moved back 400 days. The first is refused as an ended
/// one, also to list sessions, and her current web session no longer lists it; the second still
/// works, listed as expiring in about an hour; the device session works and is listed.
#[test]
fn web_session_expires_30_days_after_its_sign_in() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_300_001, "ada-expires-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let expired = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let lasting = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let listed = api.list(&cookie);

    api.move_sign_in_back(token_of(&expired), "30 days");
    api.move_sign_in_back(token_of(&lasting), "29 days 23 hours");
    api.move_sign_in_back(&laptop.token, "400 days");

    assert!(!api.web_works(&expired), "30 days after its sign-in");
    let refused = api.list_response(&[("cookie", &expired)]);
    assert_eq!(refused.status, 401, "{}", text(&refused));
    assert!(api.web_works(&lasting), "within 30 days of its sign-in");
    assert!(api.device_works(&laptop.token), "a device session");

    let after = api.list(&cookie);
    assert_eq!(after.len(), listed.len() - 1, "{after:#?}");
    let device: Vec<_> = after
        .iter()
        .filter(|session| session.kind == Kind::Device)
        .collect();
    assert_eq!(device.len(), 1, "{after:#?}");
    assert_eq!(device[0].expires_at, None, "{after:#?}");
    let soon: Vec<_> = after
        .iter()
        .filter(|session| {
            session
                .expires_at
                .is_some_and(|at| (now()..now() + 2 * 3600).contains(&at))
        })
        .collect();
    assert_eq!(
        soon.len(),
        1,
        "the one moved back 29 days 23 hours: {after:#?}"
    );
    assert_eq!(
        soon[0].expires_at,
        Some(soon[0].started_at + LIFETIME),
        "{after:#?}"
    );
}

/// The cookie a web sign-in sets expires with its session, 30 days after the sign-in, and not when
/// the browser closes.
#[test]
fn web_sign_in_cookie_expires_with_its_session() {
    let browser = Browser::launch();
    let google = Google::launch(Setup {
        github: true,
        ..Setup::default()
    });
    let user = google.github_user(3_222_300_011, "ada-cookie-standin", "ada@example.test");
    let person = google.person("322230001201", "grace.cookie@example.test");

    for with_github in [true, false] {
        let page = browser.page();
        if with_github {
            google.github_sign_in(&page, &user);
        } else {
            google.sign_in(&page, &person);
        }
        let signed_in = now();
        let cookies = page.cookies_of(HOST);
        let session: Vec<_> = cookies
            .iter()
            .filter(|cookie| cookie.name == COOKIE)
            .collect();
        assert_eq!(session.len(), 1, "{cookies:?}");
        let expires = session[0].expires;
        assert!(
            expires > 0.0,
            "a session cookie of the browser: {cookies:?}"
        );
        let expected = (signed_in + LIFETIME) as f64;
        assert!(
            (expected - 600.0..=expected + 60.0).contains(&expires),
            "expires about {expected}: {cookies:?}"
        );
    }
}
