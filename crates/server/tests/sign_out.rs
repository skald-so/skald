//! Ending a session on the server: a machine signed in through the device flow ends its session,
//! and the server refuses it on every request from then on, while the account's other sessions,
//! its devices and its master key stay. Every response is checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The API, as these tests use it: `DELETE /session`, stating the protocol version, with
//! `Authorization: Bearer <token>`, ends the session of that token and answers `204` with no body;
//! without a valid session it answers `401` with a problem details object.

mod common;

use std::collections::BTreeMap;

use common::device_flow::{DeviceFlow, json, text};
use common::devices::{
    DEVICES, Device, Key, MASTER, Machine, assert_refused, assert_taken, machine, request,
};
use common::sign_in::{Account, SESSION};

/// Checks that the server ended `machine`'s session: `response`, the answer to ending it, is a
/// `204` without a body.
fn assert_ended(response: &common::Response) {
    assert_eq!(response.status, 204, "{}", text(response));
    assert!(response.body.is_empty(), "{}", text(response));
}

/// `GET /session` with the request headers `headers`, stating the protocol version.
fn session(flow: &DeviceFlow, headers: &[(&str, &str)]) -> common::Response {
    request(flow, "get", SESSION, headers, None, true)
}

/// The account a request with `headers` acts as; fails if it acts as none.
fn account_of(flow: &DeviceFlow, headers: &[(&str, &str)]) -> Account {
    let response = session(flow, headers);
    assert_eq!(response.status, 200, "{headers:?}: {}", text(&response));
    Account::of(&json(&response))
}

/// Every request a machine sends with its session, with `machine`'s token: each must answer `401`
/// with a problem details object.
fn assert_refused_everywhere(flow: &DeviceFlow, machine: &Machine, device: &Device) {
    let bearer = format!("Bearer {}", machine.token);
    let headers = [("authorization", bearer.as_str())];
    let mut answers: Vec<common::Response> = [SESSION, DEVICES, MASTER]
        .into_iter()
        .map(|path| request(flow, "get", path, &headers, None, true))
        .collect();
    let master = Key::generate();
    answers.push(machine.register(&device.registration()));
    answers.push(machine.give_master(&master, &device.cert(&machine.account, &master)));
    answers.push(machine.sign_out());
    for response in &answers {
        assert_refused(response, 401);
        assert!(
            !text(response).contains(&machine.account.id),
            "{}",
            text(response)
        );
    }
}

/// A machine ends its session; at once, and on every request from then on, the server refuses it:
/// the session's account, the devices, the master key, a registration, a master key given and
/// ending it again.
#[test]
fn ended_session_is_refused_on_every_request() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_668_001, "ada-signs-out-standin");
    let machine = machine(&flow, &ada);
    assert_eq!(flow.account_of(&machine.token), machine.account);

    assert_ended(&machine.sign_out());

    assert_refused_everywhere(&flow, &machine, &Device::new("Laptop"));
    // Still refused later, after other requests.
    flow.assert_refused(Some(&format!("Bearer {}", machine.token)));
}

/// Ending one machine's session leaves the account's other sessions working: another machine's,
/// and the web session of the person who approved it.
#[test]
fn ending_one_session_leaves_the_accounts_other_sessions_working() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_668_002, "ada-other-sessions-standin");
    let first = machine(&flow, &ada);
    let second = machine(&flow, &ada);
    assert_eq!(first.account, second.account);

    assert_ended(&first.sign_out());

    flow.assert_refused(Some(&format!("Bearer {}", first.token)));
    assert_eq!(flow.account_of(&second.token), second.account);
    assert_eq!(second.devices(), BTreeMap::new());
    for cookie in [&first.cookie, &second.cookie] {
        assert_eq!(
            account_of(&flow, &[("cookie", cookie.as_str())]),
            first.account
        );
    }
    // The other machine can still end its own.
    assert_ended(&second.sign_out());
    flow.assert_refused(Some(&format!("Bearer {}", second.token)));
}

/// Ending another account's machine's session leaves this account's sessions as they were.
#[test]
fn ending_a_session_of_one_account_leaves_another_accounts_sessions() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_668_003, "ada-one-account-standin");
    let bob = flow
        .stand
        .user(3_120_668_004, "bob-another-account-standin");
    let ada_machine = machine(&flow, &ada);
    let bob_machine = machine(&flow, &bob);

    assert_ended(&ada_machine.sign_out());

    flow.assert_refused(Some(&format!("Bearer {}", ada_machine.token)));
    assert_eq!(flow.account_of(&bob_machine.token), bob_machine.account);
}

/// The device of an ended session stays: the account's other sessions get it with its certificate
/// as it sent it, and the account keeps its master key.
#[test]
fn ending_a_session_keeps_the_accounts_devices_and_master_key() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_668_005, "ada-keeps-device-standin");
    let first = machine(&flow, &ada);
    let device = Device::new("Laptop");
    let master = Key::generate();
    let cert = device.cert(&first.account, &master);
    first.register_device(&device);
    assert_taken(&first.give_master(&master, &cert));
    let before = first.devices();
    assert_eq!(before.get(&device.id.to_string()), Some(&Some(cert)));

    assert_ended(&first.sign_out());

    let second = machine(&flow, &ada);
    assert_eq!(second.devices(), before);
    assert_eq!(second.master(), Some(master.public().to_vec()));
    // Its registration, sent again from a new session, counts as stored.
    second.register_device(&device);
    assert_eq!(second.devices(), before);
}

/// Ending a session without a valid one, with no `Authorization` header or an unknown token, is
/// refused and ends nothing.
#[test]
fn ending_a_session_without_a_valid_one_is_refused_and_ends_nothing() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_668_006, "ada-no-session-standin");
    let machine = machine(&flow, &ada);
    let unknown = format!("Bearer {}x", machine.token);

    for headers in [vec![], vec![("authorization", unknown.as_str())]] {
        let response = request(&flow, "delete", SESSION, &headers, None, true);
        assert_refused(&response, 401);
    }
    assert_eq!(flow.account_of(&machine.token), machine.account);
}

/// Ending a session states the protocol version: a request without it is refused and ends nothing.
#[test]
fn ending_a_session_states_the_protocol_version() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_668_007, "ada-version-standin");
    let machine = machine(&flow, &ada);
    let bearer = format!("Bearer {}", machine.token);

    let response = request(
        &flow,
        "delete",
        SESSION,
        &[("authorization", &bearer)],
        None,
        false,
    );

    assert_eq!(response.status, 400, "{}", text(&response));
    assert_eq!(
        json(&response)["type"],
        "https://skald.so/problems/protocol-version-invalid"
    );
    assert_eq!(flow.account_of(&machine.token), machine.account);
}
