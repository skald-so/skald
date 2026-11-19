//! By the sender's trust level: a `peer` message reaches the hooks only as the signal, a fixed line
//! without its sender or text that tells the agent to call `inbox`; `inbox` then hands it over,
//! labelled as data from its sender, and `read` follows only then. `own` text goes in beside the
//! signal.

use crate::hooks::{Event, INBOX_WORD, Web, assert_own_line, assert_signal, others};
use crate::receipts::{shows_no_read, shows_read};
use crate::senders::{PEER, data_warnings, labelled_lines};
use crate::solo::assert_got;
use crate::talk::BOB_HOST;

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
