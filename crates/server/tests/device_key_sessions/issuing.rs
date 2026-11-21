//! A device a linked device certified gets a session of its account by its key, which the server
//! takes as any device session and lists with the device; a retry after a lost answer leaves it one
//! usable session; once signed out, it gets one again the same way.

use crate::api::Session;
use crate::common::device_flow::{json, text};
use crate::common::devices::{DEVICES, devices_of};
use crate::kit::{ask, body_by, challenge, token_of};
use crate::linked::{Linked, TABLET};
use crate::web::Connection;

#[test]
fn a_device_a_linked_device_certified_gets_a_session_of_its_account_by_its_key() {
    let l = Linked::start(3_175_310_011);
    let flow = &l.fresh.flow;
    let token = l.session();

    let who = Session::of_token(flow, &token).call("get", "/session", None);
    assert_eq!(who.status, 200, "{}", text(&who));
    let who = json(&who);
    assert_eq!(
        who["account"].as_str(),
        Some(l.fresh.alice.account.to_string().as_str()),
        "a session of alice's account: {who}"
    );
    assert!(
        who.get("sign_in_method").is_none(),
        "no sign-in method: {who}"
    );

    // The server takes it as any device session, on every instance.
    assert!(l.api().device_works(&token), "the session works");
    let other = flow.stand.instance(&[], &[]);
    assert!(
        l.api().on(&other).device_works(&token),
        "on another instance"
    );
    Connection::open(&other, &flow.stand.public, &token, &l.tablet);

    // The certificate is stored as sent, and alice's other sessions get it.
    let held = devices_of(&l.fresh.alice.session.call("get", DEVICES, None));
    assert_eq!(
        held.get(&l.tablet.id.to_string()),
        Some(&Some(l.cert.clone())),
        "the tablet's certificate: {held:?}"
    );

    // Listed with its device among the account's sessions.
    let listed = l.tablet_sessions();
    assert_eq!(
        listed.len(),
        1,
        "one session of the tablet: {:#?}",
        l.sessions()
    );
    assert!(listed[0].expires_at.is_none(), "{:?}", listed[0].item);
    assert!(!listed[0].current, "{:?}", listed[0].item);
}

#[test]
fn a_request_repeated_after_a_lost_answer_leaves_one_usable_session() {
    let l = Linked::start(3_175_310_021);
    let nobody = l.nobody();
    let challenge = challenge(&nobody, l.tablet.id);
    let body = body_by(
        &l.tablet.signing.key,
        &challenge,
        &l.fresh.flow.stand.public,
        Some(&l.cert),
    );
    let first = token_of(&ask(&nobody, l.tablet.id, &body));
    let again = ask(&nobody, l.tablet.id, &body);
    let second = token_of(&again);
    assert!(l.api().device_works(&second), "the retry's session works");
    if first != second {
        assert!(
            !l.api().device_works(&first),
            "the lost answer's session no longer works"
        );
    }
    assert_eq!(
        l.tablet_sessions().len(),
        1,
        "one session of the tablet: {:#?}",
        l.sessions()
    );
}

#[test]
fn a_device_that_signed_out_gets_a_session_again_by_its_key() {
    let l = Linked::start(3_175_310_031);
    let flow = &l.fresh.flow;
    let first = l.session();
    let signed_out = Session::of_token(flow, &first).sign_out();
    assert_eq!(signed_out.status, 204, "{}", text(&signed_out));
    assert!(!l.api().device_works(&first), "signed out");
    assert_eq!(l.tablet_sessions().len(), 0, "{:#?}", l.sessions());

    // No run and no code: its key and its certificate.
    let second = l.session();
    assert_ne!(second, first, "a new session");
    assert!(l.api().device_works(&second), "the new session works");
    assert!(!l.api().device_works(&first), "the old one stays ended");
    let listed = l.tablet_sessions();
    assert_eq!(listed.len(), 1, "{:#?}", l.sessions());
    assert_eq!(listed[0].device_name.as_deref(), Some(TABLET));
    Connection::open(&flow.stand.server, &flow.stand.public, &second, &l.tablet);
}
