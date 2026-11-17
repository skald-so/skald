//! A stream whose session ends or whose device is revoked, through any instance, closes, and a new
//! one is refused; another device's or session's stream stays open.

use std::time::Duration;

use crate::common::device_flow::text;
use crate::common::devices::machine;
use crate::fixture::Fixture;
use crate::https::{DEVICE_REFUSED, WAKE, assert_problem};
use crate::kit::by_bobs_laptop;
use crate::link::Entry;
use crate::sse::{Sse, sent_to_bobs_laptop};

/// How soon a stream closes once another instance ends its session or revokes its device.
const CLOSE: Duration = Duration::from_secs(3);

/// bob signs his laptop's session out through another instance: the laptop's stream on that
/// session closes, and a new one on it is refused with `401`; the laptop's stream on another
/// session of his account stays open and gets the next entry.
#[test]
fn signing_out_closes_the_stream_on_another_instance() {
    let f = Fixture::new(3_141_420_301);
    let t = &f.team;
    let other = t.flow.stand.instance(&[], &[]);
    let user = t.flow.stand.user(3_141_420_302, "bob-spaces-standin");
    let second = machine(&t.flow, &user);
    let server = &t.flow.stand.server;
    let on_session = Sse::of(&t.flow, &t.bob);
    let spare = on_session.https().challenge();
    let mut closing = on_session.open(None);
    let mut staying = Sse::on_session(server, &t.flow, &second.token, &t.bob.laptop).open(None);
    assert_eq!(closing.catch_up(), []);
    assert_eq!(staying.catch_up(), []);

    let signed_out = t.bob.session.on(&other).sign_out();
    assert_eq!(signed_out.status, 204, "{}", text(&signed_out));

    closing.assert_closes_within(CLOSE);
    let again = on_session.https().proof_on(&spare);
    assert_problem(&on_session.refused(None, Some(&again)), 401, None);
    let sent = sent_to_bobs_laptop(&f);
    assert_eq!(staying.entry_within(WAKE), Entry::Message(1, sent));
}

/// bob's laptop revokes bob's phone, which it certified, through another instance: the phone's
/// stream closes, and a new one is refused with `401` and `device-refused`; the laptop's stream
/// stays open and gets the next entry.
#[test]
fn revoking_the_device_closes_its_stream_on_another_instance() {
    let f = Fixture::new(3_141_420_311);
    let t = &f.team;
    let bob = &t.bob;
    let phone = by_bobs_laptop(&f, "Phone");
    let of_phone = Sse::to(&t.flow.stand.server, &t.flow, bob, &phone);
    let other = t.flow.stand.instance(&[], &[]);
    let mut closing = of_phone.open(None);
    let mut staying = Sse::of(&t.flow, bob).open(None);
    assert_eq!(closing.catch_up(), []);
    assert_eq!(staying.catch_up(), []);

    let revocation = bob.by_device(&bob.laptop, phone.id);
    let revoked = bob.session.on(&other).revoke(&revocation);
    assert_eq!(revoked.status, 201, "{}", text(&revoked));

    closing.assert_closes_within(CLOSE);
    let again = of_phone.https().proof();
    assert_problem(
        &of_phone.refused(None, Some(&again)),
        401,
        Some(DEVICE_REFUSED),
    );
    let sent = sent_to_bobs_laptop(&f);
    assert_eq!(staying.entry_within(WAKE), Entry::Message(1, sent));
}
