//! Ending a session ends only a device session: a web session can neither be ended nor end one.
//! Every response is checked against the OpenAPI spec in `api/openapi.json`.

mod common;

use common::device_flow::{DeviceFlow, json};
use common::devices::{assert_refused, machine, request};
use common::sign_in::{Account, SESSION};

#[test]
fn web_session_neither_ends_nor_is_ended() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_669_001, "ada-web-session-standin");
    let machine = machine(&flow, &ada);
    let web_token = machine
        .cookie
        .strip_prefix("skald_session=")
        .unwrap_or_else(|| panic!("a session cookie: {}", machine.cookie));
    let bearer = format!("Bearer {web_token}");

    for headers in [
        vec![("cookie", machine.cookie.as_str())],
        vec![("authorization", bearer.as_str())],
    ] {
        let response = request(&flow, "delete", SESSION, &headers, None, true);
        assert_refused(&response, 401);
    }

    let web = request(
        &flow,
        "get",
        SESSION,
        &[("cookie", &machine.cookie)],
        None,
        true,
    );
    assert_eq!(web.status, 200);
    assert_eq!(Account::of(&json(&web)), machine.account);
    assert_eq!(flow.account_of(&machine.token), machine.account);
}
