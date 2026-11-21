//! A run of a device without a session ends on a `link-abort` from either side, at the code's
//! expiry, once its device has a certificate, and when a new run of its device replaces it; its
//! messages and its pairing are then refused.

use crate::by_key::{sign_in_by_key, token_of};
use crate::common::device_flow::pause;
use crate::common::devices::{Device, assert_taken};
use crate::kit::{Fresh, Run, pair, send_pairing, until_refused};
use crate::relay::{LIFETIME, abort, assert_refused_link, grant, reply, request};
use crate::upload::issued_by;

/// Checks that `run`, paired with alice's account, has ended for both sides: its side gets no
/// messages, and alice's session relays nothing into it.
#[track_caller]
fn assert_ended(f: &Fresh, run: &Run, what: &str) {
    run.until_ended(what);
    until_refused(&f.alice.session, &grant(&run.session), what);
    assert!(
        run.send(&request(&run.session)).status != 201,
        "{what}: the new device's request is refused"
    );
}

/// Opens a run of `device`, alice's session pairs it and replies; gives the run.
#[track_caller]
fn replied(f: &Fresh, device: &Device) -> Run {
    let run = f.open(device);
    pair(&f.alice.session, &run.pairing);
    assert_taken(&f.alice.session.send_message(&reply(&run.session)));
    run
}

#[test]
fn a_link_abort_from_the_new_device_ends_its_run() {
    let f = Fresh::start(3_175_300_111);
    let tablet = Device::new("Tablet");
    let run = replied(&f, &tablet);
    assert_taken(&run.send(&abort(&run.session)));
    assert_ended(&f, &run, "aborted by the new device");

    // The device may open a new run.
    replied(&f, &tablet);
}

#[test]
fn a_link_abort_from_the_issuing_device_ends_its_run() {
    let f = Fresh::start(3_175_300_121);
    let run = replied(&f, &Device::new("Tablet"));
    assert_taken(&run.send(&request(&run.session)));
    assert_taken(&f.alice.session.send_message(&abort(&run.session)));
    assert_ended(&f, &run, "aborted by the issuing device");
}

#[test]
fn a_run_without_a_session_ends_at_the_codes_expiry() {
    let f = Fresh::with(3_175_300_131, &[LIFETIME, "3"]);
    let paired = f.open(&Device::new("Tablet"));
    pair(&f.alice.session, &paired.pairing);
    let unpaired = f.open(&Device::new("Phone"));
    pause(4);
    assert_ended(&f, &paired, "a paired run past its expiry");
    unpaired.until_ended("an unpaired run past its expiry");
    assert_refused_link(
        &send_pairing(&f.alice.session, &unpaired.pairing),
        "the pairing of an expired run",
    );
}

#[test]
fn a_run_without_a_session_ends_once_its_device_has_a_certificate() {
    let f = Fresh::start(3_175_300_141);
    let tablet = Device::new("Tablet");
    let run = replied(&f, &tablet);
    let cert = issued_by(
        &tablet,
        f.alice.account,
        skald_proto::DeviceKind::Client,
        &f.alice.laptop,
    );
    token_of(&sign_in_by_key(&f.flow, &f.nobody(), &tablet, &cert));
    assert_ended(&f, &run, "its device certified");
}

#[test]
fn a_new_run_of_a_device_replaces_its_open_one() {
    let f = Fresh::start(3_175_300_151);
    let tablet = Device::new("Tablet");
    let first = f.open(&tablet);
    let unpaired = f.open(&tablet);
    // The first run is gone, unpaired, and so is its pairing.
    first.until_ended("the replaced run");
    assert_refused_link(
        &send_pairing(&f.alice.session, &first.pairing),
        "the replaced run's pairing",
    );

    // A paired run is replaced too.
    pair(&f.alice.session, &unpaired.pairing);
    assert_taken(&f.alice.session.send_message(&reply(&unpaired.session)));
    let latest = f.open(&tablet);
    assert_ended(&f, &unpaired, "the replaced paired run");
    assert_eq!(
        pair(&f.alice.session, &latest.pairing),
        latest.hello,
        "the new run pairs"
    );
}
