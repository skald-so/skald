//! The account's sessions as a web session lists them, and who may list and end them.

use std::thread;
use std::time::Duration;

use crate::api::assert_refused;
use crate::common::device_flow::{DeviceFlow, INTERVAL, json, text};
use crate::common::devices::machine;
use crate::sessions::{
    Api, CHROME_ON_MACOS, Connection, FIREFOX_ON_WINDOWS, Kind, LIFETIME, Laptop, Listed,
    assert_no_place, assert_reveals_nothing, now, token_of, web_sign_in,
};

/// A server whose codes poll every second.
fn flow() -> DeviceFlow {
    DeviceFlow::start_with(&[INTERVAL, "1"])
}

/// The one session of `listed` of `kind` for which `pick` holds.
#[track_caller]
fn one(listed: &[Listed], kind: Kind, pick: impl Fn(&Listed) -> bool) -> Listed {
    let found: Vec<&Listed> = listed
        .iter()
        .filter(|session| session.kind == kind && pick(session))
        .collect();
    assert_eq!(found.len(), 1, "one {kind:?} session in {listed:#?}");
    found[0].clone()
}

/// Whether `value`, lowercased, holds `part`.
fn names(value: &Option<String>, part: &str) -> bool {
    value
        .as_deref()
        .is_some_and(|value| value.to_lowercase().contains(part))
}

/// Ada's laptop signed in by the device flow, approved by a web session of hers, and then her web
/// session in Chrome on macOS: its list gives the three of them, each with its own ID, which
/// reveals nothing of its token. Hers: web, current, Chrome on macOS, started at her sign-in, used
/// since, expiring 30 days after the sign-in. The laptop's: a device session named by its device,
/// no expiry. The approving one: web, not current. No IP address or location.
#[test]
fn list_gives_each_session_with_its_kind_place_start_last_use_and_expiry() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_100_001, "ada-sessions-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let before = now();
    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let after = now();

    let response = api.list_response(&[("cookie", &cookie)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    let listed = api.listed(&body);
    assert_eq!(listed.len(), 3, "{listed:#?}");
    assert_no_place(&body);

    let own = one(&listed, Kind::Web, |session| session.current);
    assert!(names(&own.browser, "chrome"), "{own:?}");
    assert!(names(&own.os, "mac"), "{own:?}");
    assert_eq!(own.device_name, None, "{own:?}");
    assert!(
        (before - 5..=after + 5).contains(&own.started_at),
        "started at the sign-in, {before}..{after}: {own:?}"
    );
    assert!(
        (own.started_at - 5..=now() + 5).contains(&own.last_used_at),
        "used since its start: {own:?}"
    );
    assert_eq!(
        own.expires_at,
        Some(own.started_at + LIFETIME),
        "30 days after its sign-in: {own:?}"
    );

    let device = one(&listed, Kind::Device, |_| true);
    assert!(!device.current, "{device:?}");
    assert_eq!(
        device.device_name.as_deref(),
        Some("ada-laptop"),
        "{device:?}"
    );
    assert_eq!(device.expires_at, None, "a device session never expires");
    assert!(device.started_at <= before + 5, "{device:?}");

    let approving = one(&listed, Kind::Web, |session| !session.current);
    assert!(
        approving.started_at <= device.started_at + 5,
        "{approving:?}"
    );

    let mut ids: Vec<&str> = listed.iter().map(|session| session.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "an ID each: {listed:#?}");
    assert_reveals_nothing(&own.id, token_of(&cookie));
    assert_reveals_nothing(&device.id, &laptop.token);
}

/// Two web sessions of one account, one signed in from Chrome on macOS and one from Firefox on
/// Windows: each is listed with the browser and OS of its own sign-in, to either.
#[test]
fn each_web_session_is_listed_with_the_browser_and_os_of_its_sign_in() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_100_011, "ada-browsers-standin");
    let chrome = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let firefox = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);

    for (cookie, current) in [(&chrome, "chrome"), (&firefox, "firefox")] {
        let listed = api.list(cookie);
        assert_eq!(listed.len(), 2, "{listed:#?}");
        let mac = one(&listed, Kind::Web, |session| {
            names(&session.browser, "chrome")
        });
        assert!(names(&mac.os, "mac"), "{mac:?}");
        assert!(!names(&mac.browser, "firefox"), "{mac:?}");
        let windows = one(&listed, Kind::Web, |session| {
            names(&session.browser, "firefox")
        });
        assert!(names(&windows.os, "windows"), "{windows:?}");
        assert!(!names(&windows.browser, "chrome"), "{windows:?}");
        let marked = if current == "chrome" { &mac } else { &windows };
        assert!(marked.current, "{current}: {listed:#?}");
    }
}

/// A machine signed out and in again resumes its device without registering it: its new device
/// session, once it connects as that device, is listed with the device's name.
#[test]
fn device_session_is_listed_with_the_device_it_connected_as() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_100_021, "ada-resumes-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-workstation");
    let ended = api.sign_out_with(&[("authorization", &format!("Bearer {}", laptop.token))]);
    assert_eq!(ended.status, 204, "{}", text(&ended));
    let resumed = machine(&flow, &ada);

    Connection::open(
        &flow.stand.server,
        &flow.stand.public,
        &resumed.token,
        &laptop.device,
    );

    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let listed = api.list(&cookie);
    let device = one(&listed, Kind::Device, |_| true);
    assert_eq!(
        device.device_name.as_deref(),
        Some("ada-workstation"),
        "{device:?}"
    );
}

/// A session's last use follows a later request, to the minute: over a minute after the list
/// first gives it, a request of the device session and one of another web session move their last
/// use to about now.
#[test]
fn last_use_follows_a_later_request() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_100_031, "ada-last-use-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let other = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let first = api.list(&cookie);
    let device = one(&first, Kind::Device, |_| true);
    let web = one(&first, Kind::Web, |session| {
        names(&session.browser, "firefox")
    });

    thread::sleep(Duration::from_secs(65));
    assert!(api.device_works(&laptop.token));
    assert!(api.web_works(&other));
    let used = now();

    let later = api.list(&cookie);
    for before in [device, web] {
        let after = one(&later, before.kind, |session| session.id == before.id);
        assert!(
            after.last_used_at >= before.last_used_at + 60,
            "moved on by the later request: {before:?} then {after:?}"
        );
        assert!(
            (used - 65..=used + 5).contains(&after.last_used_at),
            "about {used}: {after:?}"
        );
        assert_eq!(after.started_at, before.started_at, "{after:?}");
    }
}

/// Bob's web session lists none of Ada's sessions, and ends none of them by its ID: refused, and
/// each still works.
#[test]
fn another_accounts_session_is_neither_listed_nor_ended() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_100_041, "ada-own-standin");
    let bob = flow.stand.user(3_222_100_042, "bob-other-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let ada_web = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let bob_web = web_sign_in(&flow, &bob, CHROME_ON_MACOS);
    let adas = api.list(&ada_web);
    assert_eq!(adas.len(), 3, "{adas:#?}");

    let bobs = api.list(&bob_web);
    assert_eq!(bobs.len(), 1, "Bob's own only: {bobs:#?}");
    assert!(adas.iter().all(|session| session.id != bobs[0].id));
    for session in &adas {
        assert_refused(&api.end(&bob_web, &session.id), "Ada's session");
    }

    assert!(api.web_works(&ada_web));
    assert!(api.device_works(&laptop.token));
    assert_eq!(api.list(&ada_web).len(), 3);
    assert!(api.web_works(&bob_web));
}

/// A device session lists no session and ends no other: neither one by its ID nor every other.
/// It still ends itself, as a machine signs out.
#[test]
fn device_session_lists_nothing_and_ends_no_other() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_100_051, "ada-device-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let desktop = machine(&flow, &ada);
    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let listed = api.list(&cookie);
    let bearer = format!("Bearer {}", laptop.token);
    // With the server's own Origin too: a page cannot carry a bearer token, but a machine can
    // send any header.
    let headers = [("authorization", bearer.as_str()), ("origin", api.public)];

    assert_refused(&api.list_response(&headers), "a device session's list");
    for session in &listed {
        if session.device_name.as_deref() != Some("ada-laptop") {
            assert_refused(&api.end_with(&session.id, &headers), "another session");
        }
    }
    assert_refused(&api.end_others_with(&headers), "every other session");

    assert!(api.web_works(&cookie));
    assert!(api.device_works(&desktop.token));
    assert!(api.device_works(&laptop.token));
    assert_eq!(api.list(&cookie).len(), listed.len());
    let ended = api.sign_out_with(&[("authorization", &bearer)]);
    assert_eq!(ended.status, 204, "{}", text(&ended));
    assert!(!api.device_works(&laptop.token));
}

/// A web session's requests from another site, or without an `Origin`, end nothing: neither a
/// session by its ID, nor every other, nor itself. From a page of the server, the same requests
/// end them.
#[test]
fn request_from_another_site_ends_nothing() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_100_061, "ada-site-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let other = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let listed = api.list(&cookie);
    let device = one(&listed, Kind::Device, |_| true);

    for origin in [Some("http://other.test:8080"), Some("null"), None] {
        let mut headers = vec![("cookie", cookie.as_str())];
        headers.extend(origin.map(|origin| ("origin", origin)));
        let what = format!("Origin {origin:?}");
        assert_refused(&api.end_with(&device.id, &headers), &what);
        assert_refused(&api.end_others_with(&headers), &what);
        assert_refused(&api.sign_out_with(&headers), &what);
        assert!(api.device_works(&laptop.token), "{what}");
        assert!(api.web_works(&other), "{what}");
        assert!(api.web_works(&cookie), "{what}");
    }

    let ended = api.end(&cookie, &device.id);
    assert_eq!(ended.status, 204, "{}", text(&ended));
    assert!(!api.device_works(&laptop.token));
    let ended = api.end_others(&cookie);
    assert_eq!(ended.status, 204, "{}", text(&ended));
    assert!(!api.web_works(&other));
    let ended = api.sign_out(&cookie);
    assert_eq!(ended.status, 204, "{}", text(&ended));
    assert!(!api.web_works(&cookie));
}
