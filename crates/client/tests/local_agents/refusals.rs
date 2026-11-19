//! Sends that nothing on the device gets are refused, with nothing sent, and a message the server
//! refuses reaches no one.

use serde_json::json;

use crate::fixtures::{ALL, SEND, TO};
use crate::front::Answers;
use crate::lines::assert_handed_line;
use crate::machine::Finished;
use crate::solo::{
    API, SEND_TOOL, Solo, TEXT_FIELD, TO_FIELD, WEB, assert_not_sent, assert_not_shown,
};

/// The text of each refused send; it is nowhere else.
const REFUSED: &str = "local agents test: nothing on the device gets this one";

/// The text of the message the server refuses, and the problem it refuses it with.
const SERVER_REFUSED: &str = "local agents test: the server refuses this one";
const PROBLEM: &str = "not-a-member";

/// What api sends web once the refusals are over.
const AFTER: &str = "local agents test: web gets this one after the refusals";

/// An agent that never ran on Ada's device.
const GHOST: &str = "ghost";

/// No agent has registered in `clienta`. The person's `skald send --all` in `~/work/clientA`
/// fails, says why and sends nothing. Then sessions api and web start; the person's
/// `skald send --to ada/ghost` there, to an agent that does not run on Ada's device, fails, says
/// why and sends nothing. web's `inbox` and the person's `skald recv` show neither. api's send tool
/// `to` web still sends, and web gets it.
#[test]
fn send_nothing_on_the_device_gets_is_refused_and_sends_nothing() {
    let solo = Solo::new(4_657_004_001, "ada-sends-to-no-one");
    let mark = solo.mark();

    let to_all = solo.person_runs(&[SEND, ALL, REFUSED]);

    assert_refused(&solo, &to_all, mark);
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);

    let to_ghost = solo.person_runs(&[SEND, TO, &solo.short(GHOST), REFUSED]);

    assert_refused(&solo, &to_ghost, mark);
    assert_not_shown(&web.inbox(), REFUSED, WEB);
    assert_not_shown(&solo.person_reads(), REFUSED, "the person");
    let after = solo.sends(&mut api, &solo.short(WEB), AFTER);
    assert_handed_line(&web.inbox(), AFTER, after);
}

/// Fails unless `ran`, a run of `skald send`, failed and said why, and the front saw no message
/// sent in `clienta`'s default channel since `mark`.
#[track_caller]
fn assert_refused(solo: &Solo, ran: &Finished, mark: usize) {
    assert!(!ran.success, "{ran:?}");
    assert!(!ran.shown().trim().is_empty(), "no reason: {ran:?}");
    assert!(solo.sent_since(mark).is_empty(), "sent: {ran:?}");
}

/// Sessions api and web run in `~/work/clientA`. The front refuses api's next message, `to` web,
/// in place of the server, which stores it: the send tool's result says why and is no `sent`.
/// Once the front forwards again, api sends web another. web's `inbox` shows the second and not
/// the first; the person's `skald recv` does not show the first; api's `inbox` shows neither.
#[test]
fn message_the_server_refuses_reaches_no_one() {
    let solo = Solo::new(4_657_005_001, "ada-refused-locally");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let to_web = solo.short(WEB);
    solo.talk.front.answer(Answers::Refuse(PROBLEM.to_owned()));

    let called = api.call(
        SEND_TOOL,
        json!({ TO_FIELD: to_web, TEXT_FIELD: SERVER_REFUSED }),
    );

    assert_not_sent(&called, "the refused message");
    assert!(called.text.contains(PROBLEM), "{called:?}");
    solo.talk.front.answer(Answers::Forward);
    let after = solo.sends(&mut api, &to_web, AFTER);
    let got = web.inbox();
    assert_not_shown(&got, SERVER_REFUSED, WEB);
    assert_handed_line(&got, AFTER, after);
    assert_not_shown(&solo.person_reads(), SERVER_REFUSED, "the person");
    let own = api.inbox();
    for text in [SERVER_REFUSED, AFTER] {
        assert_not_shown(&own, text, API);
    }
}
