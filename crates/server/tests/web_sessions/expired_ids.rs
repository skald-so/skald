//! An expired web session is no session to end by its ID.

use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::sessions::{Api, CHROME_ON_MACOS, FIREFOX_ON_WINDOWS, Kind, token_of, web_sign_in};

/// Ada's web session in Firefox expires; ending it by the ID the list gave before is refused as
/// no such session, `404`.
#[test]
fn an_expired_web_session_is_not_ended_by_its_id() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_700_011, "ada-expired-id-standin");
    let firefox = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    let listed = api.list(&cookie);
    let expired: Vec<_> = listed
        .iter()
        .filter(|session| session.kind == Kind::Web && !session.current)
        .collect();
    assert_eq!(expired.len(), 1, "{listed:#?}");

    api.move_sign_in_back(token_of(&firefox), "30 days");

    let refused = api.end(&cookie, &expired[0].id);
    assert_eq!(refused.status, 404, "{}", text(&refused));
}
