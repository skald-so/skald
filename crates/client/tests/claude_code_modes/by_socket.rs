//! The delivery settings on a session's cross-session socket: the helper writes a channel's
//! messages as its mode for their sender's level says, `inline` writes a peer's text labelled as
//! data, and each write counts one wake for each channel whose messages it carries.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::MessageId;

use crate::cloud::WAIT;
use crate::delivery::{self, BATCH, INLINE, PULL, assert_inline, assert_wakes, set_mode};
use crate::hooks::{Bob, Event, INBOX_WORD, assert_own_line, assert_signal};
use crate::lines::{agent_lines, assert_handed_line};
use crate::receipts::shows_read_once;
use crate::registered::{Ada, QUIET};
use crate::senders::{OWN, PEER};
use crate::solo::WEB;
use crate::standin::{Line, Standin, Take};

/// Ada with web's session started over its socket, taking each message into its conversation.
fn started(id: u64, login: &str) -> (Ada, Standin) {
    let ada = Ada::new(id, login);
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);
    (ada, session)
}

/// What api says to web; none is anywhere else.
const EACH: [&str; 3] = [
    "modes socket test: api asks web to open the ticket",
    "modes socket test: api asks web to assign it",
    "modes socket test: api asks web to close it",
];

/// With no setting, api sends web three messages, one after another: each goes into the socket as
/// a line of its own, which holds that message alone, as it comes, and the model replies; the
/// default channel's wakes rise by one each time.
#[test]
fn with_no_setting_each_message_goes_into_the_socket_at_once_and_counts_one_wake() {
    let (ada, session) = started(5_207_301_001, "ada-socket-wakes-now");
    let default = ada.web.solo.default_channel();

    for (at, text) in EACH.into_iter().enumerate() {
        let msg_id = ada.web.own(text);

        let line = session.wait_lines(at + 1).remove(at);
        let content = line.content();
        assert_own_line(&content, text, msg_id, &ada.web.api());
        assert_eq!(agent_lines(&content).len(), 1, "not one message: {content}");
        session.model_replies();
        assert_wakes(&ada.web.solo, default, at as u64 + 1);
    }
}

/// The window of `batch` in the tests of the socket, as the CLI takes it and as a duration.
const SOCKET_WINDOW: (&str, Duration) = ("40s", Duration::from_secs(40));

/// What each of api's ten messages starts with; nothing else holds it.
const TEN: &str = "modes socket test: api asks web, in batch, for step";

/// Waits for up to [`WAIT`] until the lines `session`'s socket got hold every text of `sent`;
/// fails unless they came on one connection, one handover, as lines that each hold one or more of
/// them and nothing else. Gives the lines' texts, a line apart.
#[track_caller]
fn one_connection_holds_all(session: &Standin, sent: &[(String, MessageId)]) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        let lines = session.lines();
        let content: Vec<String> = lines.iter().map(Line::content).collect();
        let content = content.join("\n");
        if sent.iter().all(|(text, _)| content.contains(text)) {
            let firsts = lines.iter().filter(|line| line.first).count();
            assert_eq!(firsts, 1, "not one connection: {lines:#?}");
            for line in &lines {
                let shown = line.content();
                let messages = agent_lines(&shown);
                assert!(
                    !messages.is_empty() && messages.iter().all(|got| got.text.starts_with(TEN)),
                    "a line that holds none of the ten: {shown}"
                );
            }
            return content;
        }
        assert!(Instant::now() < deadline, "not all ten came: {lines:#?}");
        thread::sleep(Duration::from_millis(200));
    }
}

/// The person sets `own` in the default channel to `batch` with a window of 40 seconds. api's MCP
/// session sends web ten messages there: until the window's end the socket gets no line; then all
/// ten as their lines from api, on one connection, and no other line in [`QUIET`] more. The model
/// replies: Ada's machine shows `read` of the first and the last once, and the default channel has
/// one wake.
#[test]
fn in_batch_ten_messages_within_one_window_go_into_the_socket_in_one_handover() {
    let (ada, session) = started(5_207_302_001, "ada-socket-batch-of-ten");
    let solo = &ada.web.solo;
    let default = solo.default_channel();
    set_mode(solo, default, OWN, BATCH, Some(SOCKET_WINDOW.0));
    let start = Instant::now();
    let sent = delivery::api_sends_ten(solo, TEN);

    assert!(
        start.elapsed() < SOCKET_WINDOW.1,
        "the ten sends took longer than the window: {:?}",
        start.elapsed()
    );
    thread::sleep(SOCKET_WINDOW.1.saturating_sub(start.elapsed()));
    assert!(
        session.lines().is_empty(),
        "within the window: {:#?}",
        session.lines()
    );

    let content = one_connection_holds_all(&session, &sent);
    for (text, msg_id) in &sent {
        assert_own_line(&content, text, *msg_id, &ada.web.api());
    }
    session.model_replies();
    let lines = session.lines();
    thread::sleep(QUIET);
    assert_eq!(session.lines().len(), lines.len(), "{:#?}", session.lines());
    delivery::shows_reads_once(ada.machine(), &[sent[0].1, sent[9].1]);
    assert_wakes(solo, default, 1);
}

/// What api says to web; none is anywhere else.
const PULLED: [&str; 3] = [
    "modes socket test: api asks web, in pull, to plan",
    "modes socket test: api asks web, in pull, to estimate",
    "modes socket test: api asks web, in pull, to start",
];

/// The person sets `own` in the default channel to `pull`. api sends web three messages there: in
/// [`QUIET`] the socket gets no line, and the session's `PostToolUse` and `Stop` print nothing.
/// web's `inbox` returns all three, Ada's machine shows `read` of each once, and the default channel
/// has no wake.
#[test]
fn in_pull_nothing_goes_into_the_socket_and_inbox_returns_every_message() {
    let (mut ada, session) = started(5_207_303_001, "ada-socket-pull");
    let default = ada.web.solo.default_channel();
    set_mode(&ada.web.solo, default, OWN, PULL, None);
    let pulled: Vec<_> = PULLED.iter().map(|text| ada.web.own(text)).collect();

    thread::sleep(QUIET);
    assert!(session.lines().is_empty(), "{:#?}", session.lines());
    for event in [Event::PostToolUse, Event::Stop] {
        (ada.hook(WEB, &session, event)).assert_silent("a hook with only pulled messages");
    }

    let shown = ada.web.session.inbox();
    for (text, msg_id) in PULLED.into_iter().zip(&pulled) {
        assert_handed_line(&shown, text, *msg_id);
    }
    delivery::shows_reads_once(ada.machine(), &pulled);
    assert_eq!(
        delivery::wakes(&ada.web.solo, default),
        0,
        "a pulled message woke it"
    );
    assert!(session.lines().is_empty(), "{:#?}", session.lines());
}

/// What review and api say to web; none is anywhere else.
const PEER_PULLED: &str = "modes socket test: review asks web, in pull, for the dashboards";
const OWN_THERE: &str = "modes socket test: api asks web, beside the pulled peer, for alerts";
const PEER_ELSEWHERE: &str = "modes socket test: review asks web, in the default channel, to page";

/// Bob joins `clienta` and its other channel; web's session starts, and the person sets `peer` in
/// the other channel to `pull`. review sends web a message there: in [`QUIET`] the socket gets no
/// line. api sends web a message there: the socket gets its line, with no signal. review sends web
/// a message in the default channel: the socket gets the signal. Each channel has one wake; web's
/// `inbox` returns both of review's messages.
#[test]
fn a_mode_set_for_peer_in_one_channel_leaves_own_there_and_peer_elsewhere_at_now() {
    let (mut ada, session) = started(5_207_304_001, "ada-socket-pull-a-peer");
    let bob = Bob::join(&ada.web.solo, 5_207_304_002, "bob-pulled-on-ada-s-socket");
    delivery::bob_joins_other_channel(&ada.web.solo, &bob);
    let other = delivery::other_channel(&ada.web.solo);
    set_mode(&ada.web.solo, other, PEER, PULL, None);

    let pulled = delivery::review_sends_in(&ada.web.solo, &bob, other, PEER_PULLED);
    thread::sleep(QUIET);
    assert!(session.lines().is_empty(), "{:#?}", session.lines());

    let own = delivery::api_sends_in(&ada.web.solo, other, OWN_THERE);
    let content = session.wait_lines(1).remove(0).content();
    assert_own_line(&content, OWN_THERE, own, &ada.web.api());
    assert!(!content.contains(INBOX_WORD), "a signal: {content}");
    session.model_replies();

    let elsewhere = ada.web.peer(&bob, PEER_ELSEWHERE);
    let signal = session.wait_lines(2).remove(1).content();
    assert_signal(&signal, elsewhere, PEER_ELSEWHERE, &bob);
    session.model_replies();

    let messages = [(PEER_PULLED, pulled), (PEER_ELSEWHERE, elsewhere)];
    delivery::one_wake_each_and_inbox_gives(&ada.web.solo, &mut ada.web.session, &bob, messages);
}

/// What review and api say to web; none is anywhere else.
const INLINE_SAYS: &str = "modes socket test: review asks web, inline, to renew the certificate";
const OWN_INLINE: &str = "modes socket test: api asks web, beside inline, to reload the proxy";

/// Bob joins `clienta`; web's session starts, and the person sets `peer`'s text in the default
/// channel to `inline`. review sends web a message: the socket gets its line at `peer` after a line
/// that names data. The model replies: Bob's machine shows `read` of it once, with no `inbox` call,
/// and web's `inbox` does not give it again. api sends web a message: the socket gets its `own`
/// line. The default channel has two wakes.
#[test]
fn inline_for_peer_writes_a_peer_s_text_into_the_socket_labelled_as_data() {
    let (mut ada, session) = started(5_207_305_001, "ada-socket-inline");
    let bob = Bob::join(&ada.web.solo, 5_207_305_002, "bob-inline-on-ada-s-socket");
    let default = ada.web.solo.default_channel();
    delivery::set_text(&ada.web.solo, default, PEER, INLINE);

    let asked = ada.web.peer(&bob, INLINE_SAYS);
    let content = session.wait_lines(1).remove(0).content();
    assert_inline(&content, INLINE_SAYS, asked, &bob);
    session.model_replies();
    shows_read_once(&bob.person.machine, asked);
    let shown = ada.web.session.inbox();
    assert!(!shown.contains(INLINE_SAYS), "inbox gave it again: {shown}");

    let own = ada.web.own(OWN_INLINE);
    let content = session.wait_lines(2).remove(1).content();
    assert_own_line(&content, OWN_INLINE, own, &ada.web.api());
    session.model_replies();
    assert_wakes(&ada.web.solo, default, 2);
}

/// What api says to web; it is nowhere else.
const REFUSED_SAYS: &str = "modes socket test: api asks web, which refuses it on the socket";

/// web's session refuses what comes on its socket. api sends web a message; the socket gets it
/// and answers `refused`, so it never reached the model. web's `PostToolUse` then gives it, and the
/// default channel shows one wake: the hook's, none for the refused write.
#[test]
fn a_write_the_session_refuses_counts_no_wake_and_the_hook_counts_one() {
    let ada = Ada::new(5_207_306_001, "ada-socket-refused-wake");
    let session = Standin::new(Take::Receipt("refused"));
    ada.starts(WEB, &session);
    let default = ada.web.solo.default_channel();

    let msg_id = ada.web.own(REFUSED_SAYS);
    session.wait_lines(1);
    session.wait_receipts(1);
    let context = ada.hook_gives(WEB, &session);

    assert_own_line(&context, REFUSED_SAYS, msg_id, &ada.web.api());
    assert_wakes(&ada.web.solo, default, 1);
    thread::sleep(QUIET);
    assert_wakes(&ada.web.solo, default, 1);
}
