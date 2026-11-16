//! A session's last use is written at most once a minute: requests within a minute of it leave it.

use crate::common::device_flow::{DeviceFlow, INTERVAL};
use crate::sessions::{
    Api, CHROME_ON_MACOS, Connection, FIREFOX_ON_WINDOWS, Kind, Laptop, Listed, web_sign_in,
};

/// The last use of each session in `listed`, by ID, as the list gives it.
fn last_uses(listed: &[Listed]) -> Vec<(String, String)> {
    let mut uses: Vec<(String, String)> = listed
        .iter()
        .map(|session| {
            let at = session.item["last_used_at"].as_str().expect("a last use");
            (session.id.clone(), at.to_owned())
        })
        .collect();
    uses.sort();
    uses
}

/// Right after their sign-ins, requests of a web session, of a device session and of its device's
/// proved connection leave every last use as the list first gave it.
#[test]
fn requests_within_a_minute_leave_the_last_use() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let api = Api::of(&flow);
    let ada = flow.stand.user(3_222_700_001, "ada-minute-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let other = web_sign_in(&flow, &ada, FIREFOX_ON_WINDOWS);
    let cookie = web_sign_in(&flow, &ada, CHROME_ON_MACOS);
    // The device's first connection records the device, and with it the use.
    Connection::open(
        &flow.stand.server,
        &flow.stand.public,
        &laptop.token,
        &laptop.device,
    );
    let first = api.list(&cookie);
    assert!(first.iter().any(|session| session.kind == Kind::Device));

    assert!(api.web_works(&other));
    assert!(api.device_works(&laptop.token));
    Connection::open(
        &flow.stand.server,
        &flow.stand.public,
        &laptop.token,
        &laptop.device,
    );

    assert_eq!(last_uses(&api.list(&cookie)), last_uses(&first));
}
