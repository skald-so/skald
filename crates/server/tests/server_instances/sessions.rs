//! A session ended or a device revoked through one instance has its connections on the others
//! closed.

use tungstenite::protocol::frame::coding::CloseCode;

use crate::common::Closed;
use crate::common::device_flow::text;
use crate::fixture::Fixture;
use crate::instances::{CLOSE, WAKE, instance, sender, store, watch};
use crate::link::Entry;

#[test]
fn signing_out_through_one_instance_closes_the_sessions_connections_on_the_others() {
    let fixture = Fixture::new(3_121_290_041);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let other = instance(&fixture, &[]);
    let third = instance(&fixture, &[]);
    let mut on_first = watch(first, &fixture, &t.bob);
    let mut on_third = watch(&third, &fixture, &t.bob);
    let mut alice = watch(&third, &fixture, &t.alice);

    let ended = t.bob.session.on(&other).sign_out();
    assert_eq!(ended.status, 204, "{}", text(&ended));

    let session_ended = Some(Closed::Frame(CloseCode::Policy, "session-ended".to_owned()));
    assert_eq!(on_first.closed_within(CLOSE), session_ended, "on the first");
    assert_eq!(on_third.closed_within(CLOSE), session_ended, "on the third");

    // The connection of another session stays and still gets its entries.
    let mut carol = sender(&third, &fixture, &t.carol);
    let (_, sent) = store(&mut carol, &fixture, t.alice.laptop.id);
    assert_eq!(alice.entry_within(WAKE), Entry::Message(1, sent));
    assert_eq!(alice.closed_within(CLOSE), None);
}

#[test]
fn revoking_a_device_through_one_instance_closes_its_connections_on_the_others() {
    let fixture = Fixture::new(3_131_320_441);
    let t = &fixture.team;
    let first = &t.flow.stand.server;
    let other = instance(&fixture, &[]);
    let mut on_first = watch(first, &fixture, &t.bob);
    let mut on_other = watch(&other, &fixture, &t.bob);
    let mut carol = watch(&other, &fixture, &t.carol);

    let revocation = t.bob.by_master(t.bob.laptop.id, &[]);
    let stored = t.bob.session.on(&other).revoke(&revocation);
    assert_eq!(stored.status, 201, "{}", text(&stored));

    let device_refused = Some(Closed::Frame(
        CloseCode::Policy,
        "device-refused".to_owned(),
    ));
    assert_eq!(
        on_first.closed_within(CLOSE),
        device_refused,
        "on the first"
    );
    assert_eq!(
        on_other.closed_within(CLOSE),
        device_refused,
        "on the other"
    );
    // The connection of another device stays.
    assert_eq!(carol.closed_within(CLOSE), None);
}
