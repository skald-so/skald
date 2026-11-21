//! The delivery settings on the hooks of a session that gets nothing over a socket: `PostToolUse`
//! and `Stop` give a channel's messages as its mode for their sender's level says, `inline` puts a
//! peer's text in labelled as data, and each hook that gives messages counts one wake for each
//! channel they came in.

use std::thread;
use std::time::{Duration, Instant};

use crate::cloud::WAIT;
use crate::delivery::{
    BATCH, INLINE, PULL, api_sends_in, assert_inline, assert_wakes, bob_joins_other_channel,
    one_wake_each_and_inbox_gives, other_channel, review_sends_in, set_mode, set_text, wakes,
};
use crate::hooks::{Event, INBOX_WORD, Web, assert_own_line, assert_signal};
use crate::lines::assert_handed_line;
use crate::receipts::shows_read_once;
use crate::senders::{OWN, PEER, data_warnings};
use crate::solo::assert_got;
use crate::talk::BOB_HOST;

/// Runs web's `PostToolUse` every half second until it gives the session something, for up to
/// [`WAIT`]; fails if none does or a run fails. Gives its text.
#[track_caller]
fn post_tool_use_gives(web: &Web) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        let hooked = web.hook(Event::PostToolUse);
        if !hooked.stdout.is_empty() {
            return hooked.context(Event::PostToolUse);
        }
        hooked.assert_silent("PostToolUse that gives nothing");
        assert!(Instant::now() < deadline, "PostToolUse gave nothing");
        thread::sleep(Duration::from_millis(500));
    }
}

/// What api says to web; none is anywhere else.
const FIRST: &str = "modes hook test: api asks web to bump the version";
const BESIDE: &str = "modes hook test: api asks web, in the other channel, for the changelog";
const SECOND: &str = "modes hook test: api asks web to push the tag";
const BY_INBOX: &str = "modes hook test: api asks web, by inbox, for the release notes";

/// With no setting, api sends web a message in the default channel and one in the other: web's
/// `PostToolUse` gives both at once, and the listing gives one wake for each channel. api sends
/// another in the default channel: web's `Stop` gives it, and the default channel has two wakes,
/// the other still one. api sends one in the other channel, and web's `inbox` hands it over: the
/// wakes stay as they were.
#[test]
fn each_hook_handover_counts_one_wake_for_each_channel_whose_messages_it_carries() {
    let mut web = Web::new(5_207_201_001, "ada-hooks-count-wakes");
    let (default, other) = (web.solo.default_channel(), other_channel(&web.solo));
    let api = web.api();
    let first = web.own(FIRST);
    let beside = api_sends_in(&web.solo, other, BESIDE);

    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    assert_own_line(&context, FIRST, first, &api);
    assert_own_line(&context, BESIDE, beside, &api);
    assert_wakes(&web.solo, default, 1);
    assert_wakes(&web.solo, other, 1);

    let second = web.own(SECOND);
    let context = web.hook(Event::Stop).context(Event::Stop);

    assert_own_line(&context, SECOND, second, &api);
    assert_wakes(&web.solo, default, 2);
    assert_eq!(wakes(&web.solo, other), 1, "the other channel woke");

    let by_inbox = api_sends_in(&web.solo, other, BY_INBOX);
    assert_handed_line(&web.session.inbox(), BY_INBOX, by_inbox);

    assert_eq!(
        wakes(&web.solo, default),
        2,
        "inbox woke the default channel"
    );
    assert_eq!(wakes(&web.solo, other), 1, "inbox woke the other channel");
}

/// What api says to web; none is anywhere else.
const PULLED: [&str; 3] = [
    "modes hook test: api asks web, in pull, to lint",
    "modes hook test: api asks web, in pull, to format",
    "modes hook test: api asks web, in pull, to test",
];
const NOT_PULLED: &str = "modes hook test: api asks web, outside pull, to build";

/// The person sets `own` in the default channel to `pull`. api sends web three messages there:
/// web's `PostToolUse` and `Stop` print nothing. api sends one in the other channel: web's
/// `PostToolUse` gives it and none of the three. web's `inbox` returns all three; the default
/// channel has no wake, the other one.
#[test]
fn in_pull_no_hook_gives_the_channel_s_messages_and_inbox_returns_them_all() {
    let mut web = Web::new(5_207_202_001, "ada-hooks-pull");
    let (default, other) = (web.solo.default_channel(), other_channel(&web.solo));
    set_mode(&web.solo, default, OWN, PULL, None);
    let pulled: Vec<_> = PULLED.iter().map(|text| web.own(text)).collect();

    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse with only pulled messages");
    web.hook(Event::Stop)
        .assert_silent("Stop with only pulled messages");

    let not_pulled = api_sends_in(&web.solo, other, NOT_PULLED);
    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    assert_own_line(&context, NOT_PULLED, not_pulled, &web.api());
    for text in PULLED {
        assert!(!context.contains(text), "a pulled message: {context}");
    }
    let shown = web.session.inbox();
    for (text, msg_id) in PULLED.into_iter().zip(pulled) {
        assert_handed_line(&shown, text, msg_id);
    }
    assert_eq!(wakes(&web.solo, default), 0, "a pulled message woke it");
    assert_eq!(wakes(&web.solo, other), 1, "the other channel's wakes");
}

/// The window of `batch` in the tests of the hooks, as the CLI takes it and as a duration.
const HOOK_WINDOW: (&str, Duration) = ("30s", Duration::from_secs(30));

/// What api says to web; none is anywhere else.
const BATCHED: [&str; 3] = [
    "modes hook test: api asks web, in batch, to rebase",
    "modes hook test: api asks web, in batch, to squash",
    "modes hook test: api asks web, in batch, to merge",
];

/// The person sets `own` in the default channel to `batch` with a window of 30 seconds. api sends
/// web three messages there, and within the window web's `PostToolUse` and `Stop` print nothing.
/// Once the window has ended, `PostToolUse` gives all three in one handover, the default channel
/// has one wake, and the next `PostToolUse` prints nothing.
#[test]
fn in_batch_the_hooks_give_a_window_s_messages_at_its_end_in_one_handover() {
    let web = Web::new(5_207_203_001, "ada-hooks-batch");
    let default = web.solo.default_channel();
    set_mode(&web.solo, default, OWN, BATCH, Some(HOOK_WINDOW.0));
    let start = Instant::now();
    let batched: Vec<_> = BATCHED.iter().map(|text| web.own(text)).collect();

    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse within the window");
    web.hook(Event::Stop)
        .assert_silent("Stop within the window");
    assert!(
        start.elapsed() < HOOK_WINDOW.1,
        "the sends and hooks took longer than the window: {:?}",
        start.elapsed()
    );

    thread::sleep(HOOK_WINDOW.1.saturating_sub(start.elapsed()));
    let context = post_tool_use_gives(&web);

    for (text, msg_id) in BATCHED.into_iter().zip(batched) {
        assert_own_line(&context, text, msg_id, &web.api());
    }
    assert_wakes(&web.solo, default, 1);
    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse after the window's handover");
}

/// What review and api say to web; none is anywhere else.
const PEER_PULLED: &str = "modes hook test: review asks web, in pull, for the metrics";
const OWN_THERE: &str = "modes hook test: api asks web, beside the pulled peer, for the logs";
const PEER_ELSEWHERE: &str = "modes hook test: review asks web, in the default channel, for a fix";

/// Bob joins `clienta` and its other channel; the person sets `peer` in the other channel to
/// `pull`. review sends web a message there: web's `PostToolUse` and `Stop` print nothing. api
/// sends web a message there: `PostToolUse` gives it as its line, with no signal. review sends web
/// a message in the default channel: `PostToolUse` gives the signal. Each channel has one wake;
/// web's `inbox` returns both of review's messages.
#[test]
fn a_mode_set_for_peer_in_one_channel_leaves_own_there_and_peer_elsewhere_at_now() {
    let mut web = Web::new(5_207_204_001, "ada-hooks-pull-a-peer");
    let bob = web.with_bob(5_207_204_002, "bob-pulled-in-one-channel");
    bob_joins_other_channel(&web.solo, &bob);
    let other = other_channel(&web.solo);
    set_mode(&web.solo, other, PEER, PULL, None);

    let pulled = review_sends_in(&web.solo, &bob, other, PEER_PULLED);
    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse with a pulled peer message");
    web.hook(Event::Stop)
        .assert_silent("Stop with a pulled peer message");

    let own = api_sends_in(&web.solo, other, OWN_THERE);
    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);
    assert_own_line(&context, OWN_THERE, own, &web.api());
    assert!(!context.contains(INBOX_WORD), "a signal: {context}");

    let elsewhere = web.peer(&bob, PEER_ELSEWHERE);
    let signal = web.hook(Event::PostToolUse).context(Event::PostToolUse);
    assert_signal(&signal, elsewhere, PEER_ELSEWHERE, &bob);

    let messages = [(PEER_PULLED, pulled), (PEER_ELSEWHERE, elsewhere)];
    one_wake_each_and_inbox_gives(&web.solo, &mut web.session, &bob, messages);
}

/// What review and api say to web; none is anywhere else.
const INLINE_SAYS: &str = "modes hook test: review asks web, inline, to pin the dependency";
const OWN_INLINE: &str = "modes hook test: api asks web, beside inline, to vendor it";
const SIGNALLED: &str = "modes hook test: review asks web, in the other channel, to bisect";

/// Bob joins `clienta` and its other channel; the person sets `peer`'s text in the default channel
/// to `inline`. review sends web a message there: web's `PostToolUse` gives its line at `peer`
/// after a line that names data, and Bob's machine shows `read` of it once, with no `inbox` call.
/// api sends web a message: `PostToolUse` gives its `own` line, with no such line. review sends web
/// a message in the other channel: `Stop` gives only the signal, and `inbox` then gives it and not
/// the inline one again.
#[test]
fn inline_for_peer_puts_a_peer_s_text_in_by_the_hooks_labelled_as_data() {
    let mut web = Web::new(5_207_205_001, "ada-hooks-inline");
    let bob = web.with_bob(5_207_205_002, "bob-inline-in-ada-s-hooks");
    bob_joins_other_channel(&web.solo, &bob);
    let (default, other) = (web.solo.default_channel(), other_channel(&web.solo));
    set_text(&web.solo, default, PEER, INLINE);

    let asked = web.peer(&bob, INLINE_SAYS);
    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);
    assert_inline(&context, INLINE_SAYS, asked, &bob);
    shows_read_once(&bob.person.machine, asked);

    let own = web.own(OWN_INLINE);
    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);
    assert_own_line(&context, OWN_INLINE, own, &web.api());
    assert!(data_warnings(&context).is_empty(), "a warning: {context}");

    let elsewhere = review_sends_in(&web.solo, &bob, other, SIGNALLED);
    let signal = web.hook(Event::Stop).context(Event::Stop);
    assert_signal(&signal, elsewhere, SIGNALLED, &bob);
    let shown = web.session.inbox();
    assert_got(
        &shown,
        SIGNALLED,
        elsewhere,
        (&bob.review(), BOB_HOST, PEER),
    );
    assert!(
        !shown.contains(INLINE_SAYS),
        "the inline one again: {shown}"
    );
}
