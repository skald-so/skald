//! A waiting long-poll whose session ends or whose device is revoked, through any instance, ends
//! with the refusal a new request would get; another device's or session's long-poll waits on.

use std::time::{Duration, Instant};

use crate::common::device_flow::text;
use crate::common::devices::machine;
use crate::fixture::Fixture;
use crate::https::{
    Batch, DEVICE_REFUSED, Https, LONG, Waiting, assert_problem, batch_of, refusal_of,
};
use crate::kit::by_bobs_laptop;

/// How soon a waiting long-poll ends once another instance ends its session or revokes its
/// device.
const END: Duration = Duration::from_secs(3);
/// The wait of a long-poll that must outlast the end of another one.
const OUTLAST: u64 = 10;

/// The refusal that ends `waiting` within [`END`] of `since`.
#[track_caller]
fn ended(waiting: Waiting, since: Instant) -> crate::common::Response {
    let (response, at) = waiting.answer();
    let took = at.saturating_duration_since(since);
    assert!(took <= END, "ended {took:?} after, not within {END:?}");
    response
}

/// Checks that `waiting`, which outlasts the end of another long-poll, ends empty at the end of
/// its wait.
#[track_caller]
fn assert_waited_on(waiting: Waiting) {
    waiting.assert_waiting();
    let (response, _) = waiting.answer();
    assert_eq!(batch_of(&response), Batch::default());
}

/// bob signs his laptop's session out through another instance: the laptop's waiting long-poll
/// ends with `401` and the problem a new request on that session gets; a long-poll of bob's
/// laptop on another session of his account waits on.
#[test]
fn signing_out_ends_a_waiting_long_poll_on_another_instance_with_the_refusal() {
    let f = Fixture::new(3_141_410_301);
    let t = &f.team;
    let other = t.flow.stand.instance(&[], &[]);
    let bob = Https::of(&t.flow, &t.bob);
    let user = t.flow.stand.user(3_141_410_302, "bob-spaces-standin");
    let second = machine(&t.flow, &user);
    let on_second = Https::on_session(&t.flow.stand.server, &t.flow, &second.token, &t.bob.laptop);
    bob.poll_now(0);
    on_second.poll_now(0);
    let spare = bob.challenge();
    let waiting = bob.start_poll(0, LONG);
    let staying = on_second.start_poll(0, OUTLAST);
    waiting.assert_waiting();

    let signed_out = t.bob.session.on(&other).sign_out();
    assert_eq!(signed_out.status, 204, "{}", text(&signed_out));
    let response = ended(waiting, Instant::now());

    assert_problem(&response, 401, None);
    let again = bob.poll_request(0, 0, Some(bob.proof_on(&spare))).call();
    assert_problem(&again, 401, None);
    assert_eq!(refusal_of(&response), refusal_of(&again));
    assert_waited_on(staying);
}

/// bob's laptop revokes bob's phone, which it certified, through another instance: the phone's
/// waiting long-poll ends with `401` and the problem `device-refused`, as a new request of the
/// phone gets; the laptop's long-poll waits on.
#[test]
fn revoking_the_device_ends_its_waiting_long_poll_on_another_instance_with_the_refusal() {
    let f = Fixture::new(3_141_410_311);
    let t = &f.team;
    let bob = &t.bob;
    let other = t.flow.stand.instance(&[], &[]);
    let phone = by_bobs_laptop(&f, "Phone");
    let on_phone = Https::to(&t.flow.stand.server, &t.flow, bob, &phone);
    let laptop = Https::of(&t.flow, bob);
    on_phone.poll_now(0);
    laptop.poll_now(0);
    let waiting = on_phone.start_poll(0, LONG);
    let staying = laptop.start_poll(0, OUTLAST);
    waiting.assert_waiting();

    let revoked = bob
        .session
        .on(&other)
        .revoke(&bob.by_device(&bob.laptop, phone.id));
    assert_eq!(revoked.status, 201, "{}", text(&revoked));
    let response = ended(waiting, Instant::now());

    assert_problem(&response, 401, Some(DEVICE_REFUSED));
    let again = on_phone.poll_request(0, 0, Some(on_phone.proof())).call();
    assert_eq!(refusal_of(&response), refusal_of(&again));
    assert_waited_on(staying);
}
