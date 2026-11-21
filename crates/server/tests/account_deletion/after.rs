//! What a deleted account loses at once, on every instance: each of its sessions, whose
//! connections close with a reason of their own, its service agents' tokens, its link runs and its
//! pending device codes.

use crate::api::Session;
use crate::common::device_flow::{DeviceFlow, Poll, pause, text};
use crate::common::devices::{assert_taken, machine};
use crate::common::post_form;
use crate::deletion::{
    CLOSE, VERIFICATION, account_deleted, assert_deleted, assert_refused, delete_by_token,
};
use crate::relay::{WAITING, fresh_session, hello, reply};
use crate::sessions::{Api, Connection, Laptop};
use crate::tokens::Stage;

/// With two instances on one database: Ada's laptop is connected to each when its device session
/// deletes her account through the first. Within a second, each closes its connection with code
/// 1008 and the reason `account-deleted`. On both instances, the laptop's session, another
/// machine's device session and her web sessions are refused.
#[test]
fn every_session_of_a_deleted_account_is_refused_and_its_connections_close_on_every_instance() {
    let flow = DeviceFlow::start();
    let api = Api::of(&flow);
    let other = flow.stand.instance(&[], &[]);
    let ada = flow.stand.user(3_303_020_001, "ada-ends-standin");
    let laptop = Laptop::sign_in(&flow, &ada, "ada-laptop");
    let desktop = machine(&flow, &ada);
    let cookie = flow.web_cookie(&ada);
    let server = &flow.stand.server;
    let mut on_first = Connection::open(server, api.public, &laptop.token, &laptop.device);
    let mut on_other = Connection::open(&other, api.public, &laptop.token, &laptop.device);

    assert_deleted(&delete_by_token(&api, &laptop.token));

    assert_eq!(
        on_other.closed_within(CLOSE),
        account_deleted(),
        "on the other instance"
    );
    assert_eq!(
        on_first.closed_within(CLOSE),
        account_deleted(),
        "on the instance that deleted it"
    );
    for api in [api.on(server), api.on(&other)] {
        assert!(!api.device_works(&laptop.token), "the laptop's session");
        assert!(!api.device_works(&desktop.token), "the desktop's session");
        assert!(!api.web_works(&cookie), "a web session");
        assert!(!api.web_works(&desktop.cookie), "the desktop's approver");
    }
}

/// alice has a token of her `service` device, a link run of her desktop opened from another
/// machine of her account, and a device code she approved on the verification page that its
/// machine has not polled yet. Once her laptop's device session deletes her account, the token is
/// refused, the run's next message and its waiting device are refused, and a poll of the code gives
/// no token.
#[test]
fn service_token_link_run_and_device_code_of_a_deleted_account_are_refused() {
    let github = 3_303_020_101;
    let stage = Stage::new(github);
    let flow = &stage.f.team.flow;
    let api = Api::of(flow);
    let alice = &stage.f.team.alice;
    let created = stage.create();

    let user = flow.stand.user(github, "alice-spaces-standin");
    let new_side = Session::of_token(flow, &machine(flow, &user).token);
    let run = fresh_session();
    assert_taken(&new_side.send_hello(alice.uncertified.id, &hello(&run)));

    let approver = stage.cookie(alice, github);
    let grant = flow.authorize();
    let page = format!("{}{VERIFICATION}", flow.stand.server.base());
    let fields = [
        ("decision", "approve"),
        ("user_code", grant.user_code.as_str()),
    ];
    let headers = [
        ("origin", flow.stand.public.as_str()),
        ("cookie", &approver),
    ];
    let approved = post_form(&page, &headers, &fields);
    assert_eq!(approved.status, 200, "approved: {}", text(&approved));

    assert_deleted(&delete_by_token(&api, alice.session.token()));

    assert!(!stage.works(&created.token), "the service token");
    assert_refused(&new_side.send_message(&reply(&run)), "the run's reply");
    assert_refused(&new_side.call("get", WAITING, None), "the waiting device");
    pause(grant.interval);
    match flow.poll(&grant.device_code) {
        Poll::Error(error) if ["access_denied", "expired_token"].contains(&error.as_str()) => {}
        other => panic!("the approved code of a deleted account: {other:?}"),
    }
}
