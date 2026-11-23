//! Requests that run while a deletion of their account does, their session read before it ends:
//! each waits for the deletion and then acts for no deleted account. A device code is not approved,
//! a revocation does not serve the account's devices as unrevoked, and no device registers. A run
//! without a session that its device registers and is deleted under does not open.

use std::thread;

use serde_json::json;
use skald_proto::DeviceKind;

use crate::common::device_flow::{Poll, pause, text};
use crate::common::devices::{DEVICES, Device};
use crate::common::{Response, post_form};
use crate::deletion::{
    VERIFICATION, assert_deleted, assert_refused, assert_revoked, count, delete_by_token, deletes,
};
use crate::held::{Held, until_waiting};
use crate::runs::{Fresh, OPEN, register};
use crate::sessions::Api;

/// Starts a deletion of alice's account and holds it once it holds her account's row lock; `racing`
/// then sends its request, and once that request waits too, the deletion goes on. Gives the
/// request's answer, after the deletion's.
fn while_deleting(fresh: &Fresh, racing: impl FnOnce() -> Response + Send) -> Response {
    let flow = &fresh.flow;
    let db = &flow.stand.db;
    let api = Api::of(flow);
    let token = fresh.alice.session.token();
    let held = Held::sign_in_methods(db, fresh.alice.account);
    thread::scope(|scope| {
        let deleting = scope.spawn(|| delete_by_token(&api, token));
        until_waiting(db, 1, "the deletion");
        let request = scope.spawn(racing);
        until_waiting(db, 2, "the request");
        held.release();
        assert_deleted(&deleting.join().expect("the deletion"));
        request.join().expect("the request")
    })
}

/// alice's web session approves a device code on the verification page while her account's
/// deletion runs: the page says she is not signed in, and the machine's poll gets no token.
#[test]
fn device_code_decided_during_a_deletion_is_not_approved() {
    let fresh = Fresh::start(3_303_091_001);
    let flow = &fresh.flow;
    let cookie = flow.web_cookie(&fresh.alice_user());
    let grant = flow.authorize();
    let page = format!("{}{VERIFICATION}", flow.stand.server.base());
    let public = flow.stand.public.clone();

    let decided = while_deleting(&fresh, || {
        let fields = [
            ("decision", "approve"),
            ("user_code", grant.user_code.as_str()),
        ];
        let headers = [("origin", public.as_str()), ("cookie", cookie.as_str())];
        post_form(&page, &headers, &fields)
    });

    assert_eq!(decided.status, 403, "the decision: {}", text(&decided));
    pause(grant.interval);
    if let Poll::Token(_) = flow.poll(&grant.device_code) {
        panic!("a token for the deleted account");
    }
}

/// alice's session stores a revocation of her phone, by her laptop, while her account's deletion
/// runs: refused, nothing stored, and every device of hers stays revoked.
#[test]
fn revocation_stored_during_a_deletion_revokes_and_restores_nothing() {
    let fresh = Fresh::start(3_303_091_011);
    let alice = &fresh.alice;
    let db = &fresh.flow.stand.db;
    let phone = alice.certified_by(db, &alice.laptop, DeviceKind::Client, "Phone");
    let revocation = alice.by_device(&alice.laptop, phone.id);
    let session = &alice.session;

    let stored = while_deleting(&fresh, || session.revoke(&revocation));

    assert_refused(&stored, "the revocation");
    let account = alice.account.to_string();
    let revocations = "SELECT count(*) FROM revocations WHERE account_id = $1::uuid";
    assert_eq!(count(db, revocations, &account), 0, "stored revocations");
    let unrevoked = "SELECT count(*) FROM devices WHERE account_id = $1::uuid AND NOT revoked";
    assert_eq!(count(db, unrevoked, &account), 0, "her unrevoked devices");
}

/// alice's session registers a new tablet while her account's deletion runs: refused, and the
/// server holds no tablet.
#[test]
fn device_registered_during_a_deletion_is_refused() {
    let fresh = Fresh::start(3_303_091_021);
    let tablet = Device::new("Tablet");
    let session = &fresh.alice.session;

    let registered = while_deleting(&fresh, || {
        session.call("post", DEVICES, Some(&tablet.registration()))
    });

    assert_refused(&registered, "the registration");
    let devices = "SELECT count(*) FROM devices WHERE id = $1::uuid";
    let held = count(&fresh.flow.stand.db, devices, &tablet.id.to_string());
    assert_eq!(held, 0, "the tablet");
}

/// A new tablet, registered nowhere, opens a run without a session; the request reads no account of
/// the tablet, then waits on the tablet's lock for opening runs. Meanwhile alice's session registers
/// the tablet and her account is deleted. The run is then refused with `device-revoked`, and the
/// server holds no run of the tablet.
#[test]
fn run_whose_device_registers_and_is_deleted_meanwhile_is_refused() {
    let fresh = Fresh::start(3_303_091_031);
    let flow = &fresh.flow;
    let db = &flow.stand.db;
    let alice = &fresh.alice;
    let tablet = Device::new("Tablet");
    let nobody = fresh.nobody();
    let body = json!({ "device": tablet.id.to_string() });
    let held = Held::link_runs(db, tablet.id);

    let opened = thread::scope(|scope| {
        let opening = scope.spawn(|| nobody.call("post", OPEN, Some(&body)));
        until_waiting(db, 1, "the run");
        register(&alice.session, &tablet);
        deletes(&Api::of(flow), alice);
        held.release();
        opening.join().expect("the run")
    });

    assert_revoked(&opened, "the tablet's run");
    let runs = "SELECT count(*) FROM link_runs WHERE device_id = $1::uuid";
    assert_eq!(
        count(db, runs, &tablet.id.to_string()),
        0,
        "the tablet's runs"
    );
}
