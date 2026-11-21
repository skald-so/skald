//! The helper writes each message for the agent, as it arrives, into the socket of its registered
//! session, once, with the session's token: an `own` message as its line, labelled as a request
//! from the person's own agent; a `peer` one only as the signal, and the agent's `inbox` call
//! hands it over. Each message goes to the last registered session of its agent that still runs.

use crate::hooks::{Bob, assert_own_line, assert_signal};
use crate::lines::agent_line_of;
use crate::receipts::{shows_no_read, shows_read_once};
use crate::registered::{Ada, QUIET, lines_stay, no_read_for};
use crate::senders::{DATA, PEER, holds_word};
use crate::solo::{API, WEB};
use crate::standin::{Line, Standin, Take};
use crate::talk::BOB_HOST;

/// Fails unless `line` came after the auth line with `session`'s token and names its session.
#[track_caller]
fn assert_for(line: &Line, session: &Standin) {
    assert!(line.authed, "not after the session's token: {line:?}");
    assert_eq!(
        line.value["session_id"].as_str(),
        Some(session.session_id()),
        "{line:?}"
    );
}

/// What api says to web; it is nowhere else.
const OWN_SAYS: &str = "socket test: api asks web to rerun the migration";

/// web's Claude Code session in `~/work/clientA/web` starts: its `SessionStart` hook prints
/// nothing. api sends web a message. Within the wait, the session's socket gets one line, after the
/// auth line with the session's token and naming the session, whose text is api's message as its
/// line from api at `own`; in [`QUIET`] more it gets no other.
#[test]
fn own_message_goes_into_the_session_s_socket_once_with_its_token_as_text() {
    let ada = Ada::new(4_919_401_001, "ada-socket-own-text");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);

    let msg_id = ada.web.own(OWN_SAYS);

    let line = session.wait_lines(1).remove(0);
    assert_for(&line, &session);
    assert_own_line(&line.content(), OWN_SAYS, msg_id, &ada.web.api());
    lines_stay(&session, OWN_SAYS, 1, QUIET);
    assert_eq!(session.lines().len(), 1, "{:#?}", session.lines());
}

/// What Bob's agent review says to web; it is nowhere else.
const PEER_SAYS: &str = "socket test: review asks web for the deploy log";

/// Bob joins `clienta`; web's session starts. review sends web a message from Bob's machine. The
/// session's socket gets one line, after the session's token: the signal alone, which names
/// `inbox` and holds neither the text, the sender nor the reference. The session takes it and the
/// model replies; Bob's `skald recv` shows no `read` of the message in [`QUIET`]. web's `inbox`
/// then gives review's line at `peer`, after a warning line that names data, and Bob's
/// `skald recv` shows `read` of it once.
#[test]
fn peer_message_goes_into_the_socket_only_as_the_signal_and_inbox_hands_it_over() {
    let mut ada = Ada::new(4_919_402_001, "ada-socket-peer-signal");
    let bob = Bob::join(&ada.web.solo, 4_919_402_002, "bob-socket-peer-signal");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);

    let msg_id = ada.web.peer(&bob, PEER_SAYS);

    let line = session.wait_lines(1).remove(0);
    assert_for(&line, &session);
    assert_signal(&line.content(), msg_id, PEER_SAYS, &bob);
    session.model_replies();
    no_read_for(&bob.person.machine, msg_id, QUIET, "before inbox");
    let shown = ada.web.session.inbox();
    let got = agent_line_of(&shown, PEER_SAYS);
    assert_eq!(
        (got.from.as_str(), got.device.as_str(), got.level.as_str()),
        (bob.review().as_str(), BOB_HOST, PEER),
        "{shown}"
    );
    let warned = shown.lines().take_while(|line| !line.contains(PEER_SAYS));
    assert!(
        warned.into_iter().any(|line| holds_word(line, DATA)),
        "no warning line: {shown}"
    );
    shows_read_once(&bob.person.machine, msg_id);
    assert_eq!(session.lines().len(), 1, "{:#?}", session.lines());
}

/// What api and web say to each other; none is anywhere else.
const TO_WEB: &str = "socket test: api asks web for the schema";
const TO_API: &str = "socket test: web asks api for the routes";
const AFTER_GONE: &str = "socket test: api asks web again, the newer session gone";

/// Two Claude Code sessions of web start in `~/work/clientA/web`, the older one first, then one of
/// api in `~/work/clientA/api`. api sends web a message, and web's MCP session sends api one. The
/// newer session of web gets api's message and nothing else; the older one gets nothing; api's
/// session gets web's message and nothing else. The newer session of web exits without
/// `SessionEnd`, its socket gone; api sends web another message: the older session gets it. The
/// person's `skald recv` shows `read` of none of api's messages, which no model took.
#[test]
fn each_message_goes_to_the_last_registered_session_of_its_agent_that_still_runs() {
    let mut ada = Ada::new(4_919_403_001, "ada-socket-two-sessions");
    let (older, newer, api) = (
        Standin::new(Take::Queue),
        Standin::new(Take::Queue),
        Standin::new(Take::Queue),
    );
    ada.starts(WEB, &older);
    ada.starts(WEB, &newer);
    ada.starts(API, &api);

    let to_web = ada.web.own(TO_WEB);
    let to_api_peer = ada.web.api();
    let to_api = (ada.web.solo).sends(&mut ada.web.session, &to_api_peer, TO_API);

    let got = newer.wait_lines(1).remove(0);
    assert_for(&got, &newer);
    assert_own_line(&got.content(), TO_WEB, to_web, &ada.web.api());
    let line = api.wait_lines(1).remove(0);
    assert_for(&line, &api);
    assert_own_line(&line.content(), TO_API, to_api, &ada.web.solo.short(WEB));
    lines_stay(&newer, TO_API, 0, QUIET);
    assert_eq!(older.lines().len(), 0, "{:#?}", older.lines());
    assert_eq!(newer.lines().len(), 1, "{:#?}", newer.lines());
    assert_eq!(api.lines().len(), 1, "{:#?}", api.lines());

    newer.gone();
    let again = ada.web.own(AFTER_GONE);

    let line = older.wait_lines(1).remove(0);
    assert_for(&line, &older);
    assert_own_line(&line.content(), AFTER_GONE, again, &ada.web.api());
    shows_no_read(ada.machine(), to_web, "no model took api's first message");
    shows_no_read(ada.machine(), again, "no model took api's second message");
}
