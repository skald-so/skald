//! Agents of two accounts' devices in one space exchange messages and replies through the MCP
//! tools, each `inbox` labelling the other's as `peer`, while a message also reaches the sending
//! device's own agents at `own`.

use serde_json::json;

use crate::receipts::{read_lines, shows_read};
use crate::replying::replied_lines;
use crate::senders::assert_labelled;
use crate::solo::{
    API, EVERYONE, OWN, PEER, SEND_TOOL, Solo, TEXT_FIELD, TO_FIELD, WEB, assert_got,
    assert_not_shown, replies, session_in,
};
use crate::talk::{ADA_HOST, BOB_HOST};
use crate::world::dir_of;

/// Bob's agent, in his `~/work/clientA/review`.
const REVIEW: &str = "review";

/// What api and review say to each other, and what api says to everyone; none is anywhere else.
const API_ASKS: &str = "across devices test: api asks bob's review for a code review";
const REVIEW_ANSWERS: &str = "across devices test: review approves api's change";
const REVIEW_ASKS: &str = "across devices test: review asks api for the changelog";
const API_ANSWERS: &str = "across devices test: api sends review the changelog";
const TO_ALL: &str = "across devices test: api tells everyone the release is cut";

/// Bob joins `clienta` by a fixture revision; his machine runs a session `--agent review` in its
/// `~/work/clientA/review`, and Ada's runs api and web. api's send tool `to` `bob/review` reaches
/// review's `inbox` from `ada/api`, Ada's machine, at `peer`; review's reply tool answers it, and
/// api's `inbox` shows the answer from `bob/review`, Bob's machine, at `peer`. review's send tool
/// `to` `ada/api` reaches api's `inbox` at `peer`; api's reply tool answers it, and review's
/// `inbox` shows the answer at `peer`. Ada's person shows `read` and `replied` of api's first
/// message once each. Then api's send tool `to` `*` reaches web's `inbox` at `own` and review's at
/// `peer`, and not api's.
#[test]
fn api_and_bob_s_session_exchange_messages_labelled_peer_and_web_gets_api_s_at_own() {
    let solo = Solo::new(4_657_008_001, "ada-works-with-bob");
    let (bob, folder) = solo.with_bob(4_657_008_002, "bob-reviews-for-ada");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let review_dir = dir_of(&bob.machine, "work/clientA/review");
    assert!(review_dir.starts_with(&folder), "{}", review_dir.display());
    let mut review = session_in(&bob.machine, &review_dir, REVIEW);
    let (ada_api, bob_review) = (solo.short(API), format!("{}/{REVIEW}", bob.handle));

    let asked = solo.sends(&mut api, &bob_review, API_ASKS);

    let got = review.inbox();
    assert_labelled(&got, API_ASKS, (&ada_api, ADA_HOST, PEER));
    replies(&mut review, &got, API_ASKS, REVIEW_ANSWERS);
    assert_labelled(&api.inbox(), REVIEW_ANSWERS, (&bob_review, BOB_HOST, PEER));
    let called = review.call(
        SEND_TOOL,
        json!({ TO_FIELD: ada_api, TEXT_FIELD: REVIEW_ASKS }),
    );
    assert!(!called.error, "{called:?}: {}", review.errors());
    let asks = api.inbox();
    assert_labelled(&asks, REVIEW_ASKS, (&bob_review, BOB_HOST, PEER));
    replies(&mut api, &asks, REVIEW_ASKS, API_ANSWERS);
    assert_labelled(&review.inbox(), API_ANSWERS, (&ada_api, ADA_HOST, PEER));
    let person = shows_read(solo.machine(), asked) + &solo.person_reads();
    assert_eq!(read_lines(&person, asked), 1, "not read once: {person}");
    let replied = replied_lines(&person, asked, &[REVIEW_ANSWERS]);
    assert_eq!(replied.len(), 1, "not replied once: {person}");

    let said = solo.sends(&mut api, EVERYONE, TO_ALL);

    assert_got(&web.inbox(), TO_ALL, said, (&ada_api, ADA_HOST, OWN));
    assert_labelled(&review.inbox(), TO_ALL, (&ada_api, ADA_HOST, PEER));
    assert_not_shown(&api.inbox(), TO_ALL, API);
}

/// What api sends Bob's web; it is nowhere else.
const TO_BOBS_WEB: &str = "across devices test: api asks bob's web, not ada's";

/// Bob joins `clienta` as above; his machine runs a session `--agent web` in its
/// `~/work/clientA/web`, and Ada's runs api and web. api's send tool `to` `bob/web` reaches Bob's
/// web's `inbox` from `ada/api` at `peer`, and not Ada's web's; Ada's person shows no `read` of it
/// before Bob's web has it, and one after.
#[test]
fn message_to_another_account_s_agent_reaches_it_and_not_this_device_s_agent_of_that_name() {
    let solo = Solo::new(4_657_008_011, "ada-asks-bob-s-web");
    let (bob, _) = solo.with_bob(4_657_008_012, "bob-runs-a-web-agent");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let mut bob_web = session_in(&bob.machine, &dir_of(&bob.machine, "work/clientA/web"), WEB);

    let asked = solo.sends(&mut api, &format!("{}/{WEB}", bob.handle), TO_BOBS_WEB);

    assert_not_shown(&web.inbox(), TO_BOBS_WEB, WEB);
    let person = solo.person_reads();
    assert_eq!(
        read_lines(&person, asked),
        0,
        "read on Ada's device: {person}"
    );
    let got = bob_web.inbox();
    assert_labelled(&got, TO_BOBS_WEB, (&solo.short(API), ADA_HOST, PEER));
    let person = shows_read(solo.machine(), asked) + &solo.person_reads();
    assert_eq!(read_lines(&person, asked), 1, "not read once: {person}");
}
