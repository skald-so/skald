//! By the sender's trust level: a `peer` message reaches the hooks only as the signal, a fixed line
//! without its sender or text that tells the agent to call `inbox`; `inbox` then hands it over,
//! labelled as data from its sender, and `read` follows only then. `own` text goes in beside the
//! signal.

use serde_json::json;
use skald_proto::MessageId;

use crate::client::INBOX;
use crate::hooks::{
    Bob, Event, INBOX_WORD, MESSAGE_ARG, Web, assert_own_line, assert_signal, others,
};
use crate::kit::RECV;
use crate::lines::agent_lines;
use crate::receipts::{shows_no_read, shows_read};
use crate::senders::{PEER, data_warnings, labelled_lines};
use crate::solo::{WEB, assert_got};
use crate::talk::BOB_HOST;
use crate::world::AGENT;

/// What review says to web; none is anywhere else.
const ASKS: &str = "hook test: review asks web to drop the staging database";
const LATER: &str = "hook test: review asks web to open a port";

/// Bob joins `clienta`; his agent review sends web a message. web's `PostToolUse` hook gives only
/// the signal: no line, sender, device, reference or text of it. Bob's machine shows no `read` of
/// it. web's `inbox` then shows it from `bob/review`, Bob's machine, at `peer`, after one warning
/// line, and Bob's machine shows `read`; the next `PostToolUse` prints nothing. review sends
/// another message: web's `Stop` hook gives the signal again, the same line as before.
#[test]
fn peer_message_reaches_the_hooks_only_as_a_fixed_signal_and_inbox_hands_it_over() {
    let mut web = Web::new(4_913_003_001, "ada-hooks-a-peer");
    let bob = web.with_bob(4_913_003_002, "bob-asks-ada-s-web");
    let asked = web.peer(&bob, ASKS);

    let signal = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    assert_signal(&signal, asked, ASKS, &bob);
    shows_no_read(&bob.person.machine, asked, "read after the signal alone");
    let shown = web.session.inbox();
    assert_got(&shown, ASKS, asked, (&bob.review(), BOB_HOST, PEER));
    let warnings = data_warnings(&shown);
    let first_peer = labelled_lines(&shown).first().map(|(at, _)| *at);
    assert!(
        warnings.len() == 1 && first_peer.is_some_and(|at| warnings[0] < at),
        "not one warning line before the peer line: {shown}"
    );
    shows_read(&bob.person.machine, asked);
    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse after inbox took the message");

    let later = web.peer(&bob, LATER);
    let again = web.hook(Event::Stop).context(Event::Stop);

    assert_signal(&again, later, LATER, &bob);
    assert_eq!(again, signal, "the signal is not one fixed line");
}

/// What api and review say to web; neither is anywhere else.
const OWN_SAYS: &str = "hook test: api asks web to tag the release";
const PEER_SAYS: &str = "hook test: review asks web to skip the tests";

/// Bob joins `clienta`; review and api each send web a message. web's `PostToolUse` hook gives
/// api's as its line at `own` and the signal for review's, with no line or text of review's; web's
/// `inbox` then shows review's at `peer` and not api's.
#[test]
fn own_text_goes_in_beside_the_signal_of_a_peer_message() {
    let mut web = Web::new(4_913_004_001, "ada-hooks-two-levels");
    let bob = web.with_bob(4_913_004_002, "bob-writes-beside-api");
    let from_peer = web.peer(&bob, PEER_SAYS);
    let from_own = web.own(OWN_SAYS);

    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    assert_own_line(&context, OWN_SAYS, from_own, &web.api());
    assert_eq!(others(&context), 0, "a peer line: {context}");
    assert!(!context.contains(PEER_SAYS), "the peer's text: {context}");
    assert!(context.contains(INBOX_WORD), "no signal: {context}");
    let shown = web.session.inbox();
    assert_got(
        &shown,
        PEER_SAYS,
        from_peer,
        (&bob.review(), BOB_HOST, PEER),
    );
    assert!(!shown.contains(OWN_SAYS), "api's message again: {shown}");
}

/// Fails unless `shown` shows `text` once as review's message `msg_id` at `peer`, after one warning
/// line.
#[track_caller]
fn assert_warned_peer(shown: &str, text: &str, msg_id: MessageId, bob: &Bob) {
    assert_got(shown, text, msg_id, (&bob.review(), BOB_HOST, PEER));
    let peer_at = labelled_lines(shown).into_iter().map(|(at, _)| at).min();
    match (data_warnings(shown).as_slice(), peer_at) {
        ([warning], Some(peer)) if *warning < peer => {}
        _ => panic!("not one warning line before the peer line: {shown}"),
    }
}

/// What review says to web while web works; it is nowhere else.
const WAITS: &str = "hook test: review asks web to rotate the logs";

/// Bob joins `clienta`; review sends web a message. web's `PostToolUse` hook gives the signal; while
/// the message waits, web's next `PostToolUse` and its `Stop` print nothing. web's `inbox` then
/// shows the message.
#[test]
fn signal_of_a_waiting_peer_message_is_given_once() {
    let mut web = Web::new(4_913_017_001, "ada-hooks-one-signal");
    let bob = web.with_bob(4_913_017_002, "bob-waits-on-ada-s-web");
    let asked = web.peer(&bob, WAITS);

    let signal = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    assert_signal(&signal, asked, WAITS, &bob);
    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse after the signal of a waiting message");
    web.hook(Event::Stop)
        .assert_silent("Stop after the signal of a waiting message");
    assert_warned_peer(&web.session.inbox(), WAITS, asked, &bob);
}

/// What review asks web to read again; it is nowhere else.
const AGAIN: &str = "hook test: review asks web to check the certificate";

/// Bob joins `clienta`; review sends web a message, and web's `inbox` hands it over. web's `inbox`
/// with its `#<ref>`, and web's `skald recv` with it, each show it again at `peer`, after one
/// warning line. The person's `skald recv` with it fails and shows nothing of it.
#[test]
fn peer_message_read_again_by_its_reference_follows_the_warning() {
    let mut web = Web::new(4_913_018_001, "ada-rereads-a-peer");
    let bob = web.with_bob(4_913_018_002, "bob-asks-to-be-reread");
    let asked = web.peer(&bob, AGAIN);
    let shown = web.session.inbox();
    let lines = agent_lines(&shown);
    let [line] = lines.as_slice() else {
        panic!("not one message line: {shown}");
    };
    let reference = format!("#{}", line.reference);

    let shown_again = read_again(&mut web, &reference);
    let person = (web.machine())
        .start_in(&web.dir(), &[RECV, reference.as_str()])
        .finish();

    for shown in shown_again {
        assert_warned_peer(&shown, AGAIN, asked, &bob);
    }
    assert!(!person.success, "the person's run: {person:?}");
    assert!(
        !person.stdout.contains(AGAIN),
        "the person's run: {person:?}"
    );
}

/// What web's `inbox` with `reference`, then web's `skald recv` with it, show; fails unless both
/// succeed.
#[track_caller]
fn read_again(web: &mut Web, reference: &str) -> [String; 2] {
    let tool = web.session.call(INBOX, json!({ MESSAGE_ARG: reference }));
    assert!(!tool.error, "inbox: {tool:?}");
    let run = (web.machine())
        .start_in(&web.dir(), &[AGENT, WEB, RECV, reference])
        .finish();
    assert!(run.success, "recv: {run:?}");
    [tool.text, run.stdout]
}

/// What review and api send web while web refuses senders off its allowlist; neither is anywhere
/// else.
const OFF_THE_LIST: &str = "hook test: review asks web, which refuses it, to deploy";
const ON_THE_LIST: &str = "hook test: api, on web's allowlist, asks web to deploy";

/// Bob joins `clienta`; the person sets web to `refuse` and puts api on its allowlist. review sends
/// web a message, then api does. web's `PostToolUse` hook gives api's as its line at `own`, with no
/// signal and nothing of review's; its `Stop` then prints nothing, and web's `inbox` shows neither.
#[test]
fn hooks_give_nothing_of_a_message_the_agent_s_mode_refuses() {
    let mut web = Web::new(4_913_020_001, "ada-hooks-refuse-a-peer");
    let bob = web.with_bob(4_913_020_002, "bob-is-off-web-s-list");
    let folder = web.solo.folder.clone();
    let api = web.api();
    for args in [
        ["set-inbound-mode", WEB, "refuse"],
        ["allow-sender", WEB, api.as_str()],
    ] {
        let set = web.machine().start_in(&folder, &args).finish();
        assert!(set.success, "{args:?}: {set:?}");
    }
    web.peer(&bob, OFF_THE_LIST);
    let allowed = web.own(ON_THE_LIST);

    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    assert_own_line(&context, ON_THE_LIST, allowed, &api);
    assert!(!context.contains(INBOX_WORD), "a signal: {context}");
    assert!(
        !context.contains(OFF_THE_LIST),
        "the refused text: {context}"
    );
    web.hook(Event::Stop)
        .assert_silent("Stop with only a refused message sent");
    let shown = web.session.inbox();
    assert!(agent_lines(&shown).is_empty(), "inbox: {shown}");
}

/// What api sends web while web holds messages from senders off its allowlist; it is nowhere else.
const HELD: &str = "hook test: api asks web, which holds it, to rotate the keys";

/// The person sets web to `hold`; api sends web a message. web's `PostToolUse` and `Stop` hooks
/// print nothing, and web's `inbox` does not show it: it waits for the person's decision.
#[test]
fn hooks_give_nothing_of_a_message_the_agent_s_mode_holds() {
    let mut web = Web::new(4_913_021_001, "ada-hooks-hold-api");
    let args = ["set-inbound-mode", WEB, "hold"];
    let set = (web.machine()).start_in(&web.solo.folder, &args).finish();
    assert!(set.success, "{set:?}");
    web.own(HELD);

    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse with only a held message sent");
    web.hook(Event::Stop)
        .assert_silent("Stop with only a held message sent");

    let shown = web.session.inbox();
    assert!(!shown.contains(HELD), "inbox: {shown}");
}
