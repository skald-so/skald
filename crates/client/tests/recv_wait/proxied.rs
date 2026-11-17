//! Waits behind a proxy that blocks WebSocket and streaming, as in the tests of the transports'
//! fallback (`transport_fallback/main.rs`): over long-poll, a wait gets what comes, sending only
//! long-polls that wait while nothing comes; once the proxy lets WebSocket upgrades through, the
//! helper and a one-shot wait move to WebSocket within the interval; and a helper on long-poll
//! accepts a revision stored between two of its requests.

use std::thread;
use std::time::{Duration, Instant};

use crate::behind::{Behind, Forked, TIMEOUT, Transport, answered, recv};
use crate::cloud::WAIT;
use crate::filter::{EVENTS, POLL, Seen, WEBSOCKET};
use crate::fixtures::await_running;
use crate::helped::{Helped, ONE_SHOT};
use crate::kit::RECV;
use crate::machine::Run;
use crate::waits::{
    IDLE_WATCH, RETRY_SETTING, WAIT_FLAG, assert_keeps_waiting, await_helper_beside,
};
use crate::world::{devices_request, store};

/// The server's longest long-poll wait in these tests: short, so a connection on long-poll sends
/// its next request soon.
const POLL_WAIT: Duration = Duration::from_secs(2);

/// How long a connection on long-poll waits before it tries WebSocket again in these tests, in
/// seconds, and how much longer a test gives it to open one, whatever the load on the machine.
const RETRY: &str = "3";
const RETRY_SLACK: Duration = Duration::from_secs(3);

/// How long the proxy holds back the helper's next long-poll while a revision is stored.
const HELD_BACK: Duration = Duration::from_secs(4);

/// How long a test gives the helper to settle what it fetched.
const SETTLE: Duration = Duration::from_secs(1);

/// The request that gives the challenge an HTTPS request of a device proves.
const CHALLENGE: &str = "/session/challenge";

/// Ada and Bob behind the proxy, on a server whose long-poll waits at most [`POLL_WAIT`], the proxy
/// leaving long-poll alone working; nothing waits for Ada's machine.
fn on_long_poll(ids: [u64; 2], logins: [&str; 2]) -> Behind {
    let behind = Behind::start_with(ids, logins, TIMEOUT, |settings| {
        settings.long_poll_max_wait = Some(POLL_WAIT);
    });
    behind.leave(Transport::LongPoll);
    let settled = recv(&behind.ada.machine);
    assert!(settled.success, "{settled:?}");
    behind
}

/// Whether `seen` holds a long-poll with a wait above 0: a connection waiting for what comes.
fn waiting_poll(seen: &[Seen]) -> bool {
    (seen.iter()).any(|one| one.is("POST", POLL) && one.parameter("wait") != Some("0"))
}

/// Waits while `run` goes on until the proxy has seen a long-poll with a wait above 0 from its mark
/// `from` on.
#[track_caller]
fn await_waiting_poll(behind: &Behind, run: &mut Run, from: usize) {
    await_running(run, "a waiting long-poll", || {
        waiting_poll(&behind.filter.seen_since(from))
    });
}

/// From now on, the proxy lets WebSocket upgrades through. Fails unless a WebSocket opens, while
/// `run` goes on, within the retry interval and [`RETRY_SLACK`].
#[track_caller]
fn assert_moves_to_websocket(behind: &Behind, run: &mut Run) {
    let from = behind.filter.mark();
    let opened = Instant::now();
    behind.leave(Transport::WebSocket);
    await_running(run, "a WebSocket", || {
        answered(&behind.filter.seen_since(from), "GET", WEBSOCKET, 101)
    });
    let took = opened.elapsed();
    let limit = Duration::from_secs(RETRY.parse().expect("seconds")) + RETRY_SLACK;
    assert!(
        took <= limit,
        "a WebSocket opened {took:?} after the proxy let it through"
    );
}

/// Behind the proxy, Ada's `skald recv --wait` in one-shot mode, retrying WebSocket every
/// [`RETRY`] seconds, waits over long-poll; Bob's device sends a message, which it shows once over
/// long-poll alone before it exits. Her next `skald recv --wait` in one-shot mode waits over
/// long-poll too; once the proxy lets WebSocket upgrades through, it opens a WebSocket within the
/// interval and [`RETRY_SLACK`], and shows Bob's next message once.
#[test]
fn one_shot_wait_gets_messages_over_long_poll_and_moves_to_websocket_within_the_interval() {
    let mut behind = on_long_poll(
        [4_148_071_001, 4_148_071_002],
        ["ada-waits-behind-a-proxy", "bob-writes-through-a-proxy"],
    );
    let helped = Helped::new(&behind.ada.machine);
    let retry = [(RETRY_SETTING, RETRY)];
    let from = behind.filter.mark();
    let mut waiting = helped.start(&retry, &[ONE_SHOT, RECV, WAIT_FLAG]);
    await_waiting_poll(&behind, &mut waiting, from);
    let over_poll = "bob, while she waits over long-poll";

    behind.bob_sends(over_poll);

    let got = waiting.finish();
    assert!(got.success, "{got:?}");
    behind.assert_shows_bobs(&got.shown(), over_poll);
    let seen = behind.filter.seen_since(from);
    assert!(
        !answered(&seen, "GET", WEBSOCKET, 101) && !answered(&seen, "GET", EVENTS, 200),
        "not over long-poll alone: {seen:#?}"
    );

    let from = behind.filter.mark();
    let mut waiting = helped.start(&retry, &[ONE_SHOT, RECV, WAIT_FLAG]);
    await_waiting_poll(&behind, &mut waiting, from);
    assert_moves_to_websocket(&behind, &mut waiting);
    let over_websocket = "bob, once the proxy lets websockets through";
    behind.bob_sends(over_websocket);
    let got = waiting.finish();
    assert!(got.success, "{got:?}");
    behind.assert_shows_bobs(&got.shown(), over_websocket);
    helped.assert_none();
}

/// Behind the proxy, the person's `skald recv --wait`, retrying WebSocket every [`RETRY`] seconds,
/// starts a helper, which waits over long-poll. Once the proxy lets WebSocket upgrades through, the
/// helper opens a WebSocket within the interval and [`RETRY_SLACK`]; Bob's device sends a message,
/// which the waiting session shows once.
#[test]
fn helper_on_long_poll_moves_to_websocket_within_the_interval() {
    let mut behind = on_long_poll(
        [4_148_072_001, 4_148_072_002],
        ["ada-helper-behind-a-proxy", "bob-writes-to-her-helper"],
    );
    let helped = Helped::new(&behind.ada.machine);
    let from = behind.filter.mark();
    let mut session = helped.start(&[(RETRY_SETTING, RETRY)], &[RECV, WAIT_FLAG]);
    await_waiting_poll(&behind, &mut session, from);

    assert_moves_to_websocket(&behind, &mut session);

    let text = "bob, to the helper on its websocket";
    behind.bob_sends(text);
    let got = session.finish();
    assert!(got.success, "{got:?}");
    behind.assert_shows_bobs(&got.shown(), text);
}

/// Ada's machine follows a space of hers, `notices-to-helper`. Behind the proxy, the person's
/// `skald recv --wait` starts a helper, which waits over long-poll; the proxy holds its next
/// long-poll back for [`HELD_BACK`], and meanwhile a revision of the space adding Bob is stored.
/// The helper fetches the space's revisions and devices once its long-poll goes on, with no other
/// command running. Once the relay gives another revision with that number, `skald members` warns
/// of a fork: the helper accepted the revision.
#[test]
fn helper_on_long_poll_accepts_a_revision_stored_while_its_next_request_is_held_back() {
    let behind = on_long_poll(
        [4_148_073_001, 4_148_073_002],
        ["ada-helper-takes-notices", "bob-joins-between-polls"],
    );
    let forked = Forked::create(&behind, "notices-to-helper");
    let (cloud, ada) = (behind.cloud(), &behind.ada);
    let followed = recv(&ada.machine);
    assert!(followed.success, "{followed:?}");
    let helped = Helped::new(&ada.machine);
    let from = behind.filter.mark();
    let mut helper = helped.start(&[], &[RECV, WAIT_FLAG]);
    await_waiting_poll(&behind, &mut helper, from);
    let delayed = behind.filter.delayed_polls();
    behind.filter.delay_next_poll(HELD_BACK);
    await_running(&mut helper, "a long-poll held back", || {
        behind.filter.delayed_polls() > delayed
    });
    let since = cloud.relay.mark();

    store(cloud, &ada.token, &forked.late);

    let devices = devices_request(forked.space);
    let fetched = || cloud.count_sent(since, &devices) > 0;
    await_running(&mut helper, "the space's devices fetched", fetched);
    thread::sleep(SETTLE);
    forked.assert_late_accepted(&behind, "the helper on long-poll");
}

/// Waits for up to [`WAIT`] while `run`, if any, goes on until the proxy has seen no request for
/// [`SETTLE`].
#[track_caller]
fn await_settled(behind: &Behind, mut run: Option<&mut Run>) {
    let deadline = Instant::now() + WAIT;
    loop {
        let from = behind.filter.mark();
        thread::sleep(SETTLE);
        if let Some(run) = run.as_mut() {
            run.assert_running();
        }
        let seen = behind.filter.seen_since(from);
        if seen.is_empty() {
            return;
        }
        assert!(Instant::now() < deadline, "never settled: {seen:#?}");
    }
}

/// The requests of `seen` other than long-polls with a wait above 0 and the challenges that prove
/// them.
fn not_waiting(seen: &[Seen]) -> Vec<&Seen> {
    let waits = |one: &Seen| one.is("POST", POLL) && one.parameter("wait") != Some("0");
    (seen.iter())
        .filter(|one| !one.is("POST", CHALLENGE) && !waits(one))
        .collect()
}

/// Behind the proxy, the person's `skald recv --wait` starts a helper, which waits over long-poll.
/// While nothing comes, for two of the server's longest long-poll waits, the proxy sees only
/// long-polls with a wait above 0 and the challenges that prove them. Bob's device then sends a
/// message, which the session shows once; the helper keeps running, and, with no session left,
/// sends nothing more once its long-poll in flight has ended.
#[test]
fn helper_on_long_poll_sends_only_long_polls_that_wait_and_none_once_no_session_waits() {
    let mut behind = on_long_poll(
        [4_148_074_001, 4_148_074_002],
        ["ada-polls-quietly", "bob-writes-to-the-poller"],
    );
    let helped = Helped::new(&behind.ada.machine);
    let from = behind.filter.mark();
    let mut session = helped.start(&[], &[RECV, WAIT_FLAG]);
    await_waiting_poll(&behind, &mut session, from);
    let from = behind.filter.mark();

    assert_keeps_waiting(&mut session);
    thread::sleep((2 * POLL_WAIT).saturating_sub(IDLE_WATCH));

    let seen = behind.filter.seen_since(from);
    let other = not_waiting(&seen);
    assert!(
        other.is_empty(),
        "not only long-polls that wait: {other:#?}"
    );
    assert!(waiting_poll(&seen), "no long-poll that waits: {seen:#?}");
    let helper = await_helper_beside(&helped, &[&session]);
    let text = "bob, to the quiet poller";
    behind.bob_sends(text);
    let got = session.finish();
    assert!(got.success, "{got:?}");
    behind.assert_shows_bobs(&got.shown(), text);
    await_settled(&behind, None);
    let from = behind.filter.mark();
    thread::sleep(POLL_WAIT + SETTLE);
    let seen = behind.filter.seen_since(from);
    assert!(seen.is_empty(), "requests with no session: {seen:#?}");
    assert_eq!(
        helped.helpers(),
        [helper],
        "the helper did not keep running"
    );
}

/// Behind the proxy, Ada's `skald recv --wait` in one-shot mode, retrying WebSocket every
/// [`RETRY`] seconds, waits over long-poll. Once the proxy lets WebSocket upgrades through and the
/// wait has opened a WebSocket, it sends nothing more once its long-poll in flight has ended. Bob's
/// device then sends a message, which the wait shows once.
#[test]
fn one_shot_wait_that_moved_to_websocket_long_polls_no_more() {
    let mut behind = on_long_poll(
        [4_148_075_001, 4_148_075_002],
        ["ada-leaves-long-poll", "bob-writes-over-websocket"],
    );
    let helped = Helped::new(&behind.ada.machine);
    let from = behind.filter.mark();
    let mut waiting = helped.start(&[(RETRY_SETTING, RETRY)], &[ONE_SHOT, RECV, WAIT_FLAG]);
    await_waiting_poll(&behind, &mut waiting, from);
    assert_moves_to_websocket(&behind, &mut waiting);

    await_settled(&behind, Some(&mut waiting));
    let from = behind.filter.mark();
    thread::sleep(POLL_WAIT + SETTLE);

    waiting.assert_running();
    let seen = behind.filter.seen_since(from);
    assert!(seen.is_empty(), "requests on WebSocket: {seen:#?}");
    let text = "bob, once she left long-poll";
    behind.bob_sends(text);
    let got = waiting.finish();
    assert!(got.success, "{got:?}");
    behind.assert_shows_bobs(&got.shown(), text);
}
