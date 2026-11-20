//! `read` follows a handover by the helper's intake to a session that waits through it, stored in
//! `counter` order after what the device's outbox held before it.

use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{Cbor as _, Content, MessageId};

use crate::agents::CLAUDE;
use crate::cloud::WAIT;
use crate::fixtures::{ALL, CHANNEL, SEND, TIMEOUT, TIMEOUT_SETTING};
use crate::front::Front;
use crate::helped::Helped;
use crate::kit::RECV;
use crate::receipts::{Roles, assert_shows_once, shows_read_once};
use crate::talk::messages_sent;
use crate::waits::{TRACE_SETTING, TRACED, WAIT_FLAG, assert_keeps_waiting};
use crate::world::AGENT;

/// What A's agent sends, and what B's person sends while the server gets no `send`; none is
/// anywhere else.
const TEXT: &str = "receipt session test: claude, compact the event log";
const LEFT: &str = "receipt session test: the nightly build is green";

/// Through a helper, claude's `skald recv --wait` waits in B's `~/work/acme`. A's agent sends a
/// message `--to` claude, which the helper's intake hands to the session; A's runs then show
/// `read` of it once, with no other run on B's machine.
#[test]
fn read_shows_once_a_session_waiting_through_the_helper_has_the_message() {
    let roles = Roles::new(
        [4_155_010_001, 4_155_010_002],
        ["ada-agent-waits-helped", "bob-beside-the-helper"],
    );
    // Dropped first: it kills the helper before the machine's directory goes.
    let helped = Helped::new(roles.b());
    let args = [AGENT, CLAUDE, RECV, WAIT_FLAG];
    let traced = [(TRACE_SETTING, "1")];
    let mut session = helped.start_in(&roles.senders.agents.folder, &traced, &args);
    assert_keeps_waiting(&mut session);

    let msg_id = roles.a_sends_to_claude(TEXT);

    let got = session.finish();
    assert!(got.success, "{got:?}");
    assert_shows_once(&got.stdout, TEXT);
    assert!(got.stderr.contains(TRACED), "not the intake's: {got:?}");
    shows_read_once(roles.a(), msg_id);
}

/// With the front swallowing every `send`, B's person sends a message `--all` in the channel: it
/// waits in the outbox. Then, through a helper, claude's `skald recv --wait` waits in B's
/// `~/work/acme`, each request with the timeout the tests give a run that retries, and its first
/// run of `skald recv` tries the message too. B's person runs before the helper, so no run of the
/// helper keeps it waiting past that timeout. A's agent then sends a message `--to` claude, which
/// the helper's intake hands to the session. C reads B's waiting message, then its receipt, their
/// counters one after the other.
#[test]
fn receipt_of_the_helper_goes_after_the_message_the_outbox_held_before_it() {
    let mut roles = Roles::new(
        [4_155_012_001, 4_155_012_002],
        ["ada-agent-waits-behind", "bob-counts-in-order"],
    );
    let helped = Helped::new(roles.b());
    let timeout = TIMEOUT.as_secs().to_string();
    let (channel, b_device) = (roles.channel(), roles.b_device());
    let front = &roles.senders.agents.talk.front;
    let from = front.sent().len();
    front.swallow(true);
    let id = channel.to_string();
    let sent = (roles.b())
        .command_in(
            &roles.senders.agents.folder,
            &[SEND, ALL, CHANNEL, &id, LEFT],
        )
        .env(TIMEOUT_SETTING, &timeout)
        .stdin(Stdio::null())
        .output()
        .expect("run skald");
    assert!(!sent.status.success(), "stored while swallowed: {sent:?}");
    let args = [AGENT, CLAUDE, RECV, WAIT_FLAG];
    let settings = [(TRACE_SETTING, "1"), (TIMEOUT_SETTING, timeout.as_str())];
    let mut session = helped.start_in(&roles.senders.agents.folder, &settings, &args);
    assert_keeps_waiting(&mut session);
    await_no_send_for(front, TIMEOUT);
    front.swallow(false);
    let left = match messages_sent(front, channel, from).first() {
        Some(sent) => sent.envelope.envelope().header.msg_id,
        None => panic!("B's person sent nothing: {sent:?}"),
    };

    let msg_id = roles.a_sends_to_claude(TEXT);

    let got = session.finish();
    assert!(got.success, "{got:?}");
    assert!(got.stderr.contains(TRACED), "not the intake's: {got:?}");
    let receipts = roles.c_gets_receipt_of(msg_id);
    let receipt = receipts.last().expect("a receipt").msg_id;
    let played = &roles.senders.agents.talk.played[0];
    let of_b: Vec<(MessageId, u64)> = (played.read_in(channel).into_iter())
        .filter(|(_, decrypted)| decrypted.sender == b_device)
        .map(|(msg_id, decrypted)| {
            let content = Content::from_cbor(&decrypted.plaintext).expect("content");
            (msg_id, content.counter.expect("a counter").get())
        })
        .collect();
    let [(first, counter), (second, next)] = of_b.as_slice() else {
        panic!("not B's message and its receipt: {of_b:?}");
    };
    assert_eq!((*first, *second), (left, receipt), "{of_b:?}");
    assert_eq!(*next, counter + 1, "{of_b:?}");
}

/// Waits, for up to [`WAIT`], until `front` has swallowed no `send` for `quiet`: longer than a run
/// with that request timeout waits between two attempts, so its attempts are over.
#[track_caller]
fn await_no_send_for(front: &Front, quiet: Duration) {
    let deadline = Instant::now() + WAIT;
    loop {
        let swallowed = front.swallowed();
        thread::sleep(quiet);
        if front.swallowed() == swallowed {
            return;
        }
        assert!(Instant::now() < deadline, "the sends never ended");
    }
}
