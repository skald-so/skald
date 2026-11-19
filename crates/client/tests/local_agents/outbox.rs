//! A message the device sent to its own agent is handed over with the `stored` the device writes:
//! one left in the outbox reaches the agent only from the run that gets it stored, and a CLI killed
//! around `stored` neither loses nor repeats the handover.

use serde_json::json;

use crate::fixtures::{SEND, TO, kill_once};
use crate::front::Answers;
use crate::lines::assert_handed_line;
use crate::solo::{
    API, SEND_TOOL, Solo, TEXT_FIELD, TO_FIELD, WEB, assert_not_sent, assert_not_shown,
};
use crate::talk::one_message_sent;
use crate::world::AGENT;

/// What api sends web while the server is down, and when its CLI is killed.
const LEFT: &str = "local agents test: api's message waits in the outbox";
const KILLED: &str = "local agents test: api's CLI is killed before it takes stored";

/// Sessions api and web run in `~/work/clientA`. With the server down for delivery, api's send
/// tool `to` web is no `sent`, and leaves its message in the outbox; web's `inbox` meanwhile does
/// not show it. Once the server is back, the person's `skald recv` sends it, the front sees it once
/// in `clienta`'s default channel, and the person's run shows it once. web's next `inbox` shows it
/// once, and the one after does not.
#[test]
fn message_left_in_the_outbox_reaches_web_only_from_the_run_that_gets_it_stored() {
    let solo = Solo::new(4_657_006_001, "ada-sends-while-offline");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let front = &solo.talk.front;
    front.down(true);

    let called = api.call(
        SEND_TOOL,
        json!({ TO_FIELD: solo.short(WEB), TEXT_FIELD: LEFT }),
    );

    assert_not_sent(&called, "the send with the server down");
    let meanwhile = web.call(crate::client::INBOX, json!({}));
    assert_not_shown(&meanwhile.text, LEFT, WEB);
    front.down(false);
    let mark = solo.mark();
    let person = solo.person_reads();
    let left = one_message_sent(front, solo.default_channel(), mark);
    assert_eq!(person.matches(LEFT).count(), 1, "{person}");
    assert_handed_line(&web.inbox(), LEFT, left);
    assert_not_shown(&web.inbox(), LEFT, WEB);
}

/// A session web runs in `~/work/clientA`. The front loses the server's `stored` answers; api's
/// `skald send --to ada/web`, which the server stores, is killed once a `stored` was lost. Once the
/// front forwards them again, the person's `skald recv` sends the message again and shows it once;
/// its next run does not. web's next `inbox` shows the message once, and the one after does not.
#[test]
fn cli_killed_before_it_takes_stored_hands_the_message_over_once_on_the_next_run() {
    let solo = Solo::new(4_657_007_001, "ada-killed-mid-send");
    let mut web = solo.session(WEB);
    let front = &solo.talk.front;
    front.answer(Answers::Lose);
    let mark = solo.mark();
    let to_web = solo.short(WEB);

    let started = solo
        .machine()
        .start_in(&solo.dir(API), &[AGENT, API, SEND, TO, &to_web, KILLED]);
    kill_once(started, "a lost stored", || front.lost() > 0);

    front.answer(Answers::Forward);
    let sent = solo.sent_since(mark);
    let killed = *sent.first().expect("the killed run sent its message");
    let person = solo.person_reads();
    assert_eq!(person.matches(KILLED).count(), 1, "{person}");
    assert_not_shown(&solo.person_reads(), KILLED, "the person's next run");
    assert_handed_line(&web.inbox(), KILLED, killed);
    assert_not_shown(&web.inbox(), KILLED, WEB);
}
