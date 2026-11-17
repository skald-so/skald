//! A device session of the account gets its devices as a web session does.

use crate::common::device_flow::{json, text};
use crate::sessions::Api;
use crate::shown::{ACCOUNT_DEVICES, flow, listed, person, shown};

/// Ada's laptop's device session gets her account's devices, the same as her web session does.
#[test]
fn a_device_session_gets_the_accounts_devices_as_a_web_session_does() {
    let flow = flow();
    let api = Api::of(&flow);
    let (ada, cookie) = person(&flow, 3_224_100_201, "ada");

    let response = ada.session.call("get", ACCOUNT_DEVICES, None);
    assert_eq!(response.status, 200, "{}", text(&response));
    let mut by_device: Vec<_> = listed(&api, &json(&response))
        .into_iter()
        .map(|device| device.item)
        .collect();
    by_device.sort_by_key(|item| item["name"].to_string());
    let by_web: Vec<_> = shown(&api, &cookie)
        .into_values()
        .map(|device| device.item)
        .collect();
    assert_eq!(by_device.len(), 2, "{by_device:?}");
    assert_eq!(by_device, by_web);
}
