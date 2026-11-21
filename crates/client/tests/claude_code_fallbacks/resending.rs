//! Resending: a message goes by the session's first path, in the order socket, Channels,
//! `asyncRewake`; without a confirmation within the timeout, by the next; after the last it waits
//! for the hooks and `inbox`. A signal the agent does not follow with `inbox` goes again. One
//! confirmation ends it, and a message the session file already shows, queued too, goes by no
//! other path; the hooks and `inbox` give a message only after the last path. A helper killed while a message waits for its timeout leaves it to be resent, and `read` of it
//! goes once.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::client::Session;
use crate::cloud::WAIT;
use crate::fallbacks::{
    QUIET, RESEND_SETTING, Rewake, channel_of, events, rewake_of, session_start, wait_events, woke,
};
use crate::hooks::{Event, assert_own_line, assert_signal};
use crate::receipts::shows_read_once;
use crate::registered::{Ada, lines_stay};
use crate::solo::WEB;
use crate::standin::{SOCKET_VAR, Standin, TOKEN_VAR, Take};

/// How long the `asyncRewake` hook waits in these tests: well beyond what a test takes.
const REWAKE_WAITS: u64 = 120;

/// The settings Claude Code gives the session's MCP server and hooks, its socket and token, with
/// the resend timeout `resend`, in seconds.
fn settings(session: &Standin, resend: u64) -> Vec<(String, String)> {
    vec![
        (
            SOCKET_VAR.to_owned(),
            session.socket().to_string_lossy().into_owned(),
        ),
        (TOKEN_VAR.to_owned(), session.token().to_owned()),
        (RESEND_SETTING.to_owned(), resend.to_string()),
    ]
}

fn borrowed(settings: &[(String, String)]) -> Vec<(&str, &str)> {
    (settings.iter())
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

/// web's paths beside the socket of `session`, its stand-in: its `skald mcp`, if `channel`, and its
/// `asyncRewake` hook, if `rewake`, started on `SessionStart` once the session registered.
struct Paths {
    channel: Option<Session>,
    rewake: Option<Rewake>,
}

/// `session` starts as web's with the resend timeout `resend`, in seconds, and web's other paths
/// as `channel` and `rewake` say; a second later they all run.
fn started(ada: &Ada, session: &Standin, resend: u64, channel: bool, rewake: bool) -> Paths {
    let given = settings(session, resend);
    let given = borrowed(&given);
    ada.starts_with(WEB, session, &given);
    let dir = ada.web.solo.dir(WEB);
    let channel = channel.then(|| channel_of(&ada.helped, &dir, WEB, &given));
    let rewake = rewake.then(|| {
        let mut input = session_start();
        input["session_id"] = json!(session.session_id());
        input["transcript_path"] = json!(session.file().to_string_lossy());
        rewake_of(&ada.helped, &dir, WEB, &input, REWAKE_WAITS, &given)
    });
    thread::sleep(Duration::from_secs(1));
    let mut paths = Paths { channel, rewake };
    if let Some(rewake) = &mut paths.rewake {
        rewake.assert_running("once the session started");
    }
    paths
}

impl Paths {
    fn channel(&mut self) -> &mut Session {
        self.channel.as_mut().expect("web's skald mcp runs")
    }

    fn rewake(&mut self) -> &mut Rewake {
        self.rewake.as_mut().expect("web's asyncRewake hook runs")
    }
}

/// How long the timeout is in a test that times each path, in seconds.
const TIMED: u64 = 3;

/// A wait that the timeout must have passed in: a second short of it.
fn nearly(timeout: u64) -> Duration {
    Duration::from_secs(timeout - 1)
}

/// What api says to web; it is nowhere else.
const ORDER_SAYS: &str = "resend test: api asks web, which never takes it in";

/// web's session starts, with a timeout of [`TIMED`] seconds; it loses what comes. web's `skald mcp`
/// and its `asyncRewake` hook run. api sends web a message; the socket gets its line. For a second more, no channel event comes and the hook still
/// runs. Then, once the timeout passed, one channel event, the message's line from api at `own`,
/// while the hook still runs; once it passed again, the hook exits with 2, its standard error the
/// line. Then web's `PostToolUse` gives the line, and the person's `skald recv` shows `read` of it
/// once. The socket got it once, and `skald mcp` sent one event.
#[test]
fn a_message_goes_by_the_socket_then_channels_then_async_rewake_then_the_hooks() {
    let ada = Ada::new(4_919_521_001, "ada-resend-in-order");
    let session = Standin::new(Take::Lose);
    let mut paths = started(&ada, &session, TIMED, true, true);
    let api = ada.web.api();

    let msg_id = ada.web.own(ORDER_SAYS);
    let line = session.wait_lines(1).remove(0);
    let written = Instant::now();
    assert!(line.content().contains(ORDER_SAYS), "{line:?}");

    let early = events(paths.channel(), 1, Duration::from_secs(1));
    assert!(early.is_empty(), "an event before the timeout: {early:#?}");
    paths.rewake().assert_running("before the timeout");
    let sent = wait_events(paths.channel(), 1, WAIT);
    let evented = Instant::now();
    assert!(
        written.elapsed() >= nearly(TIMED),
        "the event after {:?}",
        written.elapsed()
    );
    assert_own_line(&sent[0], ORDER_SAYS, msg_id, &api);
    paths.rewake().assert_running("before the second timeout");
    let (hooked, _) = paths.rewake().finished();
    assert!(
        evented.elapsed() >= nearly(TIMED),
        "the hook after {:?}",
        evented.elapsed()
    );
    assert_own_line(&woke(&hooked), ORDER_SAYS, msg_id, &api);

    let context = ada.hook_gives(WEB, &session);
    assert_own_line(&context, ORDER_SAYS, msg_id, &api);
    shows_read_once(ada.machine(), msg_id);
    lines_stay(&session, ORDER_SAYS, 1, Duration::from_secs(1));
    assert_eq!(
        events(paths.channel(), 2, Duration::ZERO).len(),
        1,
        "events again"
    );
}

/// What api says to web; it is nowhere else.
const CONFIRMED_SAYS: &str = "resend test: api asks web, which takes it in at once";

/// web's session starts, with a timeout of 2 seconds; it takes what comes into the conversation.
/// web's `skald mcp` and its `asyncRewake` hook run. api sends web a message; once the socket has
/// it, the model replies: the person's `skald recv` shows `read` of it once. Over [`QUIET`] more,
/// no channel event comes, the hook still runs, the socket got it once, and web's `PostToolUse`
/// prints nothing.
#[test]
fn one_confirmation_ends_the_resending() {
    let ada = Ada::new(4_919_522_001, "ada-resend-confirmed");
    let session = Standin::new(Take::Accept);
    let mut paths = started(&ada, &session, 2, true, true);

    let msg_id = ada.web.own(CONFIRMED_SAYS);
    session.wait_lines(1);
    session.model_replies();
    shows_read_once(ada.machine(), msg_id);

    thread::sleep(QUIET);
    let sent = events(paths.channel(), 1, Duration::ZERO);
    assert!(sent.is_empty(), "events after the confirmation: {sent:#?}");
    paths.rewake().assert_running("after the confirmation");
    lines_stay(&session, CONFIRMED_SAYS, 1, Duration::ZERO);
    (ada.hook(WEB, &session, Event::PostToolUse))
        .assert_silent("PostToolUse after the confirmation");
}

/// What api says to web; it is nowhere else.
const HELD_SAYS: &str = "resend test: api asks web, which holds it, then lets it in";

/// web's session starts, with a timeout of 5 seconds; it holds what comes. web's `skald mcp` and
/// its `asyncRewake` hook run. api sends web a message; the socket gets it and answers `held`. One
/// channel event follows, the message's line from api at `own`. The session releases the message,
/// takes it into the conversation, and the model replies: the person's `skald recv` shows `read`
/// of it once. Over [`QUIET`] and the timeout more, no other channel event comes, the hook still
/// runs, and the socket got the message once.
#[test]
fn a_message_held_on_the_socket_goes_by_the_next_path_and_its_release_adds_nothing() {
    let ada = Ada::new(4_919_523_001, "ada-resend-held");
    let session = Standin::new(Take::Receipt("held"));
    let mut paths = started(&ada, &session, 5, true, true);

    let msg_id = ada.web.own(HELD_SAYS);
    session.wait_lines(1);
    session.wait_receipts(1);
    let sent = wait_events(paths.channel(), 1, WAIT);
    assert_own_line(&sent[0], HELD_SAYS, msg_id, &ada.web.api());
    session.release();
    session.model_replies();
    shows_read_once(ada.machine(), msg_id);

    thread::sleep(QUIET + Duration::from_secs(5));
    assert_eq!(
        events(paths.channel(), 2, Duration::ZERO).len(),
        1,
        "events again"
    );
    paths.rewake().assert_running("after the release");
    lines_stay(&session, HELD_SAYS, 1, Duration::ZERO);
}

/// What review says to web in each test; neither is anywhere else.
const IGNORED_SAYS: &str = "resend test: review asks web, which does not call inbox";
const FOLLOWED_SAYS: &str = "resend test: review asks web, which calls inbox";

/// The signal goes again: by a second line on the socket, or by the hook.
fn signalled_again(session: &Standin, paths: &mut Paths) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        if !paths.rewake().running() {
            return woke(&paths.rewake().finished().0);
        }
        if let Some(line) = session.lines().get(1) {
            return line.content();
        }
        assert!(Instant::now() < deadline, "the signal did not go again");
        thread::sleep(Duration::from_millis(100));
    }
}

/// Bob joins `clienta`. web's session starts, with a timeout of [`TIMED`] seconds; it takes what
/// comes into the conversation. web's `asyncRewake` hook runs. review sends web a message from
/// Bob's machine: the socket gets the signal alone, and the model replies without calling `inbox`.
/// Then the signal alone goes again. web's `inbox` then shows review's text, and Bob's `skald recv`
/// shows `read` of it once.
#[test]
fn a_signal_the_agent_does_not_follow_with_inbox_goes_again() {
    let mut ada = Ada::new(4_919_524_001, "ada-resend-signal-ignored");
    let bob = ada.web.with_bob(4_919_524_002, "bob-resend-signal-ignored");
    let session = Standin::new(Take::Accept);
    let mut paths = started(&ada, &session, TIMED, false, true);

    let msg_id = ada.web.peer(&bob, IGNORED_SAYS);
    let line = session.wait_lines(1).remove(0);
    assert_signal(&line.content(), msg_id, IGNORED_SAYS, &bob);
    session.model_replies();

    let again = signalled_again(&session, &mut paths);
    assert_signal(&again, msg_id, IGNORED_SAYS, &bob);
    let shown = ada.web.session.inbox();
    assert!(shown.contains(IGNORED_SAYS), "inbox: {shown}");
    shows_read_once(&bob.person.machine, msg_id);
}

/// Bob joins `clienta`. web's session starts, with a timeout of 2 seconds; it takes what comes
/// into the conversation. web's `asyncRewake` hook runs. review sends web a message from Bob's
/// machine: the socket gets the signal, the model replies, and web's `inbox` shows review's text.
/// Over [`QUIET`] more, the hook still runs and the socket got the signal once.
#[test]
fn a_signal_the_agent_follows_with_inbox_goes_no_more() {
    let mut ada = Ada::new(4_919_525_001, "ada-resend-signal-followed");
    let bob = ada
        .web
        .with_bob(4_919_525_002, "bob-resend-signal-followed");
    let session = Standin::new(Take::Accept);
    let mut paths = started(&ada, &session, 2, false, true);

    let msg_id = ada.web.peer(&bob, FOLLOWED_SAYS);
    let line = session.wait_lines(1).remove(0);
    assert_signal(&line.content(), msg_id, FOLLOWED_SAYS, &bob);
    session.model_replies();
    let shown = ada.web.session.inbox();
    assert!(shown.contains(FOLLOWED_SAYS), "inbox: {shown}");

    thread::sleep(QUIET);
    paths.rewake().assert_running("after inbox");
    assert_eq!(
        session.lines().len(),
        1,
        "the signal again: {:#?}",
        session.lines()
    );
}

/// What api says to web; it is nowhere else.
const KILLED_SAYS: &str = "resend test: api asks web as the helper dies";

/// web's session starts, with a timeout of [`TIMED`] seconds; it loses what comes. api sends web a
/// message; once the socket has it, the helper is killed. web's `PostToolUse`, run until it gives something, gives the message's line from api at
/// `own`; the person's `skald recv` shows `read` of it once; the socket got it once, and the next
/// `PostToolUse` prints nothing.
#[test]
fn a_helper_killed_while_a_message_waits_for_its_timeout_leaves_it_to_be_resent_once() {
    let ada = Ada::new(4_919_526_001, "ada-resend-helper-killed");
    let session = Standin::new(Take::Lose);
    let _paths = started(&ada, &session, TIMED, false, false);

    let msg_id = ada.web.own(KILLED_SAYS);
    session.wait_lines(1);
    ada.helped.kill(ada.helped.one_helper());

    let context = ada.hook_gives(WEB, &session);
    assert_own_line(&context, KILLED_SAYS, msg_id, &ada.web.api());
    shows_read_once(ada.machine(), msg_id);
    lines_stay(&session, KILLED_SAYS, 1, Duration::from_secs(1));
    (ada.hook(WEB, &session, Event::PostToolUse)).assert_silent("PostToolUse once it gave it");
}

/// What api says to web; it is nowhere else.
const QUEUED_SAYS: &str = "resend test: api asks web, which queues it behind a tool";

/// web's session starts, with a timeout of 2 seconds; it queues what comes, as a session whose
/// tool runs. web's `skald mcp` and its `asyncRewake` hook run. api sends web a message; the
/// socket gets its line, which the session file shows queued. Over [`QUIET`] more, no channel
/// event comes, the hook still runs and web's `PostToolUse` prints nothing. The tool ends: the
/// session takes the line into the conversation, and the model replies. The person's `skald recv`
/// shows `read` of it once, and the socket got it once.
#[test]
fn a_line_the_session_queued_goes_by_no_other_path() {
    let ada = Ada::new(4_919_527_001, "ada-resend-queued");
    let session = Standin::new(Take::Queue);
    let mut paths = started(&ada, &session, 2, true, true);

    let msg_id = ada.web.own(QUEUED_SAYS);
    session.wait_lines(1);
    thread::sleep(QUIET);
    let sent = events(paths.channel(), 1, Duration::ZERO);
    assert!(sent.is_empty(), "events for a queued line: {sent:#?}");
    paths.rewake().assert_running("with the line queued");
    (ada.hook(WEB, &session, Event::PostToolUse)).assert_silent("PostToolUse with the line queued");

    session.dequeue();
    session.model_replies();
    shows_read_once(ada.machine(), msg_id);
    lines_stay(&session, QUEUED_SAYS, 1, Duration::ZERO);
}

/// What api says to web; it is nowhere else.
const WAITING_SAYS: &str = "resend test: api asks web, whose hooks wait for the last path";

/// The timeout of the test of what the hooks and `inbox` give while the paths wait, in seconds:
/// long enough for both to run within it.
const CHECKED: u64 = 8;

/// Fails unless web's `PostToolUse` prints nothing and its `inbox` shows no [`WAITING_SAYS`], as
/// `when` says.
fn nothing_yet(ada: &mut Ada, session: &Standin, when: &str) {
    (ada.hook(WEB, session, Event::PostToolUse)).assert_silent(&format!("PostToolUse {when}"));
    let shown = ada.web.session.inbox();
    assert!(!shown.contains(WAITING_SAYS), "inbox {when}: {shown}");
}

/// web's session starts, with a timeout of [`CHECKED`] seconds; it loses what comes. web's `skald
/// mcp` and its `asyncRewake` hook run. api sends web a message. Once the channel event comes,
/// and again once the hook exits with 2, each within the timeout, web's `PostToolUse` prints
/// nothing and its `inbox` shows no text of api's. Then `PostToolUse`, run until it gives
/// something, gives the message's line from api at `own`.
#[test]
fn the_hooks_and_inbox_give_a_message_only_after_the_last_path() {
    let mut ada = Ada::new(4_919_528_001, "ada-resend-hooks-wait");
    let session = Standin::new(Take::Lose);
    let mut paths = started(&ada, &session, CHECKED, true, true);
    let api = ada.web.api();

    let msg_id = ada.web.own(WAITING_SAYS);
    session.wait_lines(1);
    let sent = wait_events(paths.channel(), 1, WAIT);
    let evented = Instant::now();
    assert_own_line(&sent[0], WAITING_SAYS, msg_id, &api);
    nothing_yet(&mut ada, &session, "while Channels waits");
    paths.rewake().assert_running("while Channels waits");
    assert!(
        evented.elapsed() < Duration::from_secs(CHECKED),
        "checked late"
    );

    let (hooked, _) = paths.rewake().finished();
    let woken = Instant::now();
    assert_own_line(&woke(&hooked), WAITING_SAYS, msg_id, &api);
    nothing_yet(&mut ada, &session, "while asyncRewake waits");
    assert!(
        woken.elapsed() < Duration::from_secs(CHECKED),
        "checked late"
    );

    let context = ada.hook_gives(WEB, &session);
    assert_own_line(&context, WAITING_SAYS, msg_id, &api);
}
