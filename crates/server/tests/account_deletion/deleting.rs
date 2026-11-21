//! Who deletes an account: a web session from a page of the server, or a device session, each with
//! a sign-in more recent than the setting. An older sign-in, a request from another site or without
//! `Origin`, and a service token delete nothing.

use crate::common::device_flow::DeviceFlow;
use crate::deletion::{
    OLDER, TOO_OLD, assert_deleted, assert_refused, bearer, delete_by_cookie, delete_by_token,
    delete_with, problem_of,
};
use crate::sessions::{Api, Laptop, token_of};
use crate::tokens::Stage;

/// Checks that the account of the web session `cookie` and the device session `token` is not
/// deleted: both still work.
#[track_caller]
fn assert_kept(api: &Api, cookie: &str, token: &str, what: &str) {
    assert!(api.web_works(cookie), "{what}: the web session");
    assert!(api.device_works(token), "{what}: the device session");
}

/// Ada's web session, signed in just now, deletes her account from a page of the server: `204`.
/// The web session and her laptop's device session are refused from then on.
#[test]
fn web_session_with_a_recent_sign_in_deletes_its_account() {
    let flow = DeviceFlow::start();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_303_010_001, "ada-web-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let cookie = flow.web_cookie(&ada);

    assert_deleted(&delete_by_cookie(&api, &cookie));

    assert!(!api.web_works(&cookie), "the web session that deleted it");
    assert!(!api.device_works(&laptop.token), "the laptop's session");
}

/// Ada's laptop, signed in just now, deletes her account with its device session: `204`. The
/// device session and her web session are refused from then on.
#[test]
fn device_session_with_a_recent_sign_in_deletes_its_account() {
    let flow = DeviceFlow::start();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_303_010_011, "ada-device-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let cookie = flow.web_cookie(&ada);

    assert_deleted(&delete_by_token(&api, &laptop.token));

    assert!(
        !api.device_works(&laptop.token),
        "the session that deleted it"
    );
    assert!(!api.web_works(&cookie), "the web session");
}

/// Ada's web session and her laptop's device session, whose sign-ins are moved back 10 minutes,
/// past the setting's 300 seconds, delete nothing: each is refused with `sign-in-too-old` and still
/// works. A web session she signs in with again then deletes her account.
#[test]
fn sign_in_older_than_the_setting_deletes_nothing() {
    let flow = DeviceFlow::start();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_303_010_021, "ada-old-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let cookie = flow.web_cookie(&ada);
    api.move_sign_in_back(token_of(&cookie), OLDER);
    api.move_sign_in_back(&laptop.token, OLDER);

    let by_web = delete_by_cookie(&api, &cookie);
    assert_eq!(problem_of(&by_web, "an old web sign-in"), TOO_OLD);
    let by_device = delete_by_token(&api, &laptop.token);
    assert_eq!(problem_of(&by_device, "an old device sign-in"), TOO_OLD);
    assert_kept(&api, &cookie, &laptop.token, "after the old sign-ins");

    let fresh = flow.web_cookie(&ada);
    assert_deleted(&delete_by_cookie(&api, &fresh));
    assert!(
        !api.device_works(&laptop.token),
        "after the fresh sign-in's deletion"
    );
}

/// Ada's web session, signed in just now, deletes nothing by a request from another site, one from
/// a site whose name starts with the server's, one without `Origin`, or one without the session;
/// from a page of the server it then deletes her account.
#[test]
fn request_from_another_site_or_without_origin_deletes_nothing() {
    let flow = DeviceFlow::start();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_303_010_031, "ada-site-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let cookie = flow.web_cookie(&ada);
    let look_alike = api.public.replacen("skald.test", "skald.test.evil.test", 1);
    let refusals: [(&str, &[(&str, &str)]); 4] = [
        (
            "another site",
            &[("cookie", &cookie), ("origin", "http://evil.test")],
        ),
        (
            "a look-alike site",
            &[("cookie", &cookie), ("origin", &look_alike)],
        ),
        ("no Origin", &[("cookie", &cookie)]),
        ("no session", &[("origin", api.public)]),
    ];

    for (what, headers) in refusals {
        assert_refused(&delete_with(&api, headers), what);
        assert_kept(&api, &cookie, &laptop.token, what);
    }

    assert_deleted(&delete_by_cookie(&api, &cookie));
    assert!(
        !api.device_works(&laptop.token),
        "after the page's deletion"
    );
}

/// A service token of alice's `service` device deletes nothing, with or without the server's
/// `Origin`: alice's session and the token still work. alice's own device session then deletes
/// her account; bob, the space's other admin, keeps it from blocking.
#[test]
fn service_token_deletes_nothing() {
    let stage = Stage::new(3_303_010_041);
    let flow = &stage.f.team.flow;
    let api = Api::of(flow);
    let alice = &stage.f.team.alice;
    let created = stage.create();
    let token = bearer(&created.token);

    let refusals: [(&str, &[(&str, &str)]); 2] = [
        ("a service token", &[("authorization", &token)]),
        (
            "a service token from a page",
            &[("authorization", &token), ("origin", api.public)],
        ),
    ];
    for (what, headers) in refusals {
        assert_refused(&delete_with(&api, headers), what);
        assert!(api.device_works(alice.session.token()), "{what}: alice");
        assert!(stage.works(&created.token), "{what}: the token");
    }

    assert_deleted(&delete_by_token(&api, alice.session.token()));
}
