//! A device's session by its key ends, its connections closed on every instance, once its device
//! is revoked, signs out, or a web session ends it.

use std::time::Duration;

use tungstenite::protocol::frame::coding::CloseCode;

use crate::api::Session;
use crate::common::Closed;
use crate::common::device_flow::text;
use crate::linked::Linked;
use crate::web::Connection;

/// How long a test waits for the connections of an ended session to close.
const CLOSE: Duration = Duration::from_secs(5);

/// The tablet's session and its connections to two instances, the first and `other`.
fn connected(l: &Linked, other: &crate::common::Server) -> (String, [Connection; 2]) {
    let flow = &l.fresh.flow;
    let token = l.session();
    let public = &flow.stand.public;
    let connections = [
        Connection::open(&flow.stand.server, public, &token, &l.tablet),
        Connection::open(other, public, &token, &l.tablet),
    ];
    (token, connections)
}

/// Checks that each of `connections` closes with code 1008 and one of `reasons`, and that neither
/// instance takes the session `token` any more.
#[track_caller]
fn assert_ended(
    l: &Linked,
    other: &crate::common::Server,
    token: &str,
    connections: &mut [Connection; 2],
    reasons: &[&str],
) {
    for (at, connection) in connections.iter_mut().enumerate() {
        match connection.closed_within(CLOSE) {
            Some(Closed::Frame(CloseCode::Policy, reason))
                if reasons.contains(&reason.as_str()) => {}
            closed => panic!("connection {at}: closed with one of {reasons:?}, not {closed:?}"),
        }
    }
    assert!(
        !l.api().device_works(token),
        "the first instance refuses it"
    );
    assert!(
        !l.api().on(other).device_works(token),
        "the other refuses it"
    );
    assert_eq!(l.tablet_sessions().len(), 0, "{:#?}", l.sessions());
}

#[test]
fn the_session_ends_once_its_device_is_revoked() {
    let l = Linked::start(3_175_310_211);
    let other = l.fresh.flow.stand.instance(&[], &[]);
    let (token, mut connections) = connected(&l, &other);
    let alice = &l.fresh.alice;
    alice.revoke(&alice.by_device(&alice.laptop, l.tablet.id));
    assert_ended(
        &l,
        &other,
        &token,
        &mut connections,
        &["device-refused", "session-ended"],
    );
}

#[test]
fn the_session_ends_once_its_device_signs_out() {
    let l = Linked::start(3_175_310_221);
    let other = l.fresh.flow.stand.instance(&[], &[]);
    let (token, mut connections) = connected(&l, &other);
    let signed_out = Session::of_token(&l.fresh.flow, &token).sign_out();
    assert_eq!(signed_out.status, 204, "{}", text(&signed_out));
    assert_ended(&l, &other, &token, &mut connections, &["session-ended"]);
}

#[test]
fn the_session_ends_once_a_web_session_ends_it() {
    let l = Linked::start(3_175_310_231);
    let other = l.fresh.flow.stand.instance(&[], &[]);
    let (token, mut connections) = connected(&l, &other);
    let listed = l.tablet_sessions();
    assert_eq!(listed.len(), 1, "{:#?}", l.sessions());
    let ended = l.api().end(&l.cookie, &listed[0].id);
    assert_eq!(ended.status, 204, "{}", text(&ended));
    assert_ended(&l, &other, &token, &mut connections, &["session-ended"]);

    // alice's web session and her laptop's session stay.
    assert!(l.api().web_works(&l.cookie));
    assert!(l.api().device_works(l.fresh.alice.session.token()));
}
