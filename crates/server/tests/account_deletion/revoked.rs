//! A deleted account's devices, each served as a revoked one, open nothing later: no session by a
//! device's key, for a device a revoked one certifies too, and no link run without a session; a run
//! one of them opened before the deletion ends with it.

use skald_proto::DeviceKind;

use crate::by_key::sign_in_by_key;
use crate::common::TestDb;
use crate::common::devices::Device;
use crate::deletion::{assert_refused, assert_revoked, count, deletes};
use crate::relay::request;
use crate::runs::Fresh;
use crate::sessions::Api;
use crate::upload::issued_by;

/// How many link runs `db` holds of `device`.
fn runs_of(db: &TestDb, device: &Device) -> i64 {
    let sql = "SELECT count(*) FROM link_runs WHERE device_id = $1::uuid";
    count(db, sql, &device.id.to_string())
}

/// Marks `device` unrevoked in `db`.
fn unrevoke(db: &TestDb, device: &Device) {
    db.execute(&format!(
        "UPDATE devices SET revoked = false WHERE id = '{}'",
        device.id
    ));
}

/// alice's account is deleted. Her laptop, revoked with it, certifies a new tablet, and her
/// desktop, registered there without a certificate and marked unrevoked as a racing request could
/// leave it. Each asks for a session by its key with that certificate: refused with
/// `device-revoked`. The server holds no tablet, no certificate of the desktop and no session of
/// her account.
#[test]
fn device_a_deleted_accounts_device_certifies_gets_no_session() {
    let fresh = Fresh::start(3_303_090_001);
    let flow = &fresh.flow;
    let alice = &fresh.alice;
    deletes(&Api::of(flow), alice);
    unrevoke(&flow.stand.db, &alice.uncertified);

    let tablet = Device::new("Tablet");
    let desktop = &alice.uncertified;
    for (device, what) in [(&tablet, "a new tablet"), (desktop, "her desktop")] {
        let cert = issued_by(device, alice.account, DeviceKind::Client, &alice.laptop);
        let response = sign_in_by_key(flow, &fresh.nobody(), device, &cert);
        assert_revoked(&response, what);
    }

    let db = &flow.stand.db;
    let tablet = tablet.id.to_string();
    let desktop = desktop.id.to_string();
    let devices = "SELECT count(*) FROM devices WHERE id = $1::uuid";
    assert_eq!(count(db, devices, &tablet), 0, "the tablet");
    let certified = "SELECT count(*) FROM devices WHERE id = $1::uuid AND cert IS NOT NULL";
    assert_eq!(
        count(db, certified, &desktop),
        0,
        "the desktop's certificate"
    );
    // A session by a device's key is of its certificate's account.
    let sessions = "SELECT count(*) FROM sessions WHERE account_id = $1::uuid";
    let account = alice.account.to_string();
    assert_eq!(count(db, sessions, &account), 0, "a session of her account");
}

/// Once alice's account is deleted, her desktop, registered there without a certificate and marked
/// unrevoked as a racing request could leave it, opens no run without a session: refused with
/// `device-revoked`, and the server holds no run of it.
#[test]
fn deleted_accounts_device_opens_no_run_without_a_session() {
    let fresh = Fresh::start(3_303_090_011);
    let alice = &fresh.alice;
    deletes(&Api::of(&fresh.flow), alice);
    unrevoke(&fresh.flow.stand.db, &alice.uncertified);

    assert_revoked(&fresh.send_open(&alice.uncertified), "her desktop's run");
    assert_eq!(runs_of(&fresh.flow.stand.db, &alice.uncertified), 0);
}

/// alice's desktop, registered on her account without a certificate, opened a run without a
/// session and sent its hello. Once her account is deleted, the run's token gets and relays no
/// message, and the server holds no run of the desktop.
#[test]
fn run_without_a_session_of_a_deleted_accounts_device_ends() {
    let fresh = Fresh::start(3_303_090_021);
    let alice = &fresh.alice;
    let run = fresh.open(&alice.uncertified);
    deletes(&Api::of(&fresh.flow), alice);

    assert_refused(&run.send(&request(&run.session)), "the run's request");
    assert_refused(&run.messages_response(), "the run's messages");
    assert_eq!(runs_of(&fresh.flow.stand.db, &alice.uncertified), 0);
}
