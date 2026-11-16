//! Ending sessions: one by its ID, every other one, and the web session itself. An ended session
//! is refused at once; a CLI session's device stays with its certificate.

use crate::common::Response;
use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::common::devices::machine;
use crate::sessions::{
    Api, CHROME_ON_MACOS, FIREFOX_ON_WINDOWS, Kind, Laptop, Listed, web_sign_in,
};

/// A server whose codes poll every second.
fn flow() -> DeviceFlow {
    DeviceFlow::start_with(&[INTERVAL, "1"])
}

/// Checks that `response` says the session ended: a `204` without a body.
#[track_caller]
fn assert_ended(response: &Response) {
    assert_eq!(response.status, 204, "{}", text(response));
    assert!(response.body.is_empty(), "{}", text(response));
}

/// The IDs of `listed`, sorted.
fn ids(listed: &[Listed]) -> Vec<String> {
    let mut ids: Vec<String> = listed.iter().map(|session| session.id.clone()).collect();
    ids.sort();
    ids
}

/// The ID of the one session of `listed` of `kind` that is not the current one and for which
/// `pick` holds.
#[track_caller]
fn id_of(listed: &[Listed], kind: Kind, pick: impl Fn(&Listed) -> bool) -> String {
    let found: Vec<&Listed> = listed
        .iter()
        .filter(|session| session.kind == kind && !session.current && pick(session))
        .collect();
    assert_eq!(found.len(), 1, "one {kind:?} session in {listed:#?}");
    found[0].id.clone()
}

/// Ada ends her laptop's CLI session from her web session: at once, the server refuses it on
/// every request a machine makes with it, and no longer lists it. Her other sessions work, and
/// her laptop's device stays on the account with its certificate, as does the master key.
#[test]
fn ended_cli_session_is_refused_at_once_and_its_device_stays() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_200_001, "ada-ends-cli-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let other = machine(&flow, &ada);
    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let before = (other.devices(), other.master());
    assert!(
        before.0.contains_key(&laptop.device.id.to_string()),
        "{before:?}"
    );
    let listed = api.list(&cookie);
    let id = id_of(&listed, Kind::Device, |session| {
        session.device_name.as_deref() == Some("ada-laptop")
    });

    assert_ended(&api.end(&cookie, &id));

    assert!(!api.device_works(&laptop.token), "the ended session");
    let after = api.list(&cookie);
    assert!(after.iter().all(|session| session.id != id), "{after:#?}");
    assert_eq!(after.len(), listed.len() - 1, "{after:#?}");
    assert!(api.device_works(&other.token));
    assert!(api.web_works(&cookie));
    assert_eq!((other.devices(), other.master()), before);
}

/// Ada ends her web session in Firefox from the one in Chrome: at once, the server refuses it,
/// also to list or end sessions, and no longer lists it; the Chrome one works.
#[test]
fn ended_web_session_is_refused_at_once() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_200_011, "ada-ends-web-standin");
    let firefox = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let chrome = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let listed = api.list(&chrome);
    assert_eq!(listed.len(), 2, "{listed:#?}");
    let id = id_of(&listed, Kind::Web, |_| true);

    assert_ended(&api.end(&chrome, &id));

    assert!(!api.web_works(&firefox), "the ended session");
    let refused = api.list_response(&[("cookie", &firefox)]);
    assert_eq!(refused.status, 401, "{}", text(&refused));
    assert_eq!(api.end_others(&firefox).status, 401);
    assert!(api.web_works(&chrome));
    let after = api.list(&chrome);
    assert_eq!(after.len(), 1, "{after:#?}");
    assert!(after[0].current, "{after:#?}");
}

/// "Sign out everywhere else": every other session of the account ends at once, web and CLI alike,
/// and the list then gives only the current one.
#[test]
fn ending_every_other_session_leaves_only_this_one() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_200_021, "ada-everywhere-standin");
    let chrome = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let firefox = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let desktop = machine(&flow, &ada);
    let listed = api.list(&chrome);
    assert_eq!(listed.len(), 6, "{listed:#?}");
    let own: Vec<&Listed> = listed.iter().filter(|session| session.current).collect();
    assert_eq!(own.len(), 1, "{listed:#?}");

    assert_ended(&api.end_others(&chrome));

    for token in [&laptop.token, &desktop.token] {
        assert!(!api.device_works(token), "a machine");
    }
    for cookie in [&firefox, &desktop.cookie] {
        assert!(!api.web_works(cookie), "another browser");
    }
    assert!(api.web_works(&chrome));
    let after = api.list(&chrome);
    assert_eq!(ids(&after), vec![own[0].id.clone()], "{after:#?}");
    assert!(after[0].current);
}

/// Signing the web session out ends it at once, and no other session: the account's other web
/// session and its machine still work, and no longer list it.
#[test]
fn signing_a_web_session_out_ends_it_and_no_other() {
    let flow = flow();
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_200_031, "ada-signs-out-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let firefox = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let chrome = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let listed = api.list(&firefox);
    let id = id_of(&listed, Kind::Web, |session| {
        session
            .browser
            .as_deref()
            .is_some_and(|browser| browser.to_lowercase().contains("chrome"))
    });

    assert_ended(&api.sign_out(&chrome));

    assert!(!api.web_works(&chrome), "the signed-out session");
    assert_eq!(api.sign_out(&chrome).status, 401, "signed out already");
    assert!(api.web_works(&firefox));
    assert!(api.device_works(&laptop.token));
    let after = api.list(&firefox);
    assert_eq!(after.len(), listed.len() - 1, "{after:#?}");
    assert!(after.iter().all(|session| session.id != id), "{after:#?}");
}
