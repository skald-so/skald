//! A held message whose wait ends before anyone decides gets `refused`, saying no one decided.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{Cbor as _, Envelope, RawEnvelope, ReceiptStatus};

use crate::agents::{CLAUDE, assert_not_handed};
use crate::gaps::sends;
use crate::held::{
    NO_ONE_DECIDED, c_reads, holding, nothing_to_decide, reason_of, says_none_waiting, says_waiting,
};
use crate::talk::Line;

/// The `ttl` of the message, in seconds: how long it waits for a decision.
const TTL: u64 = 10;

/// What Bob's agent sends; it is nowhere else.
const TEXT: &str = "hold expiry test: claude, renew the wildcard certificate";

/// B's person sets claude to `hold`. Bob's device sends, as his agent, a message `--to` claude
/// with a `ttl` of [`TTL`] seconds; B's person's `skald recv` takes it in and says one message
/// waits, and C reads B's `held` receipt of it. Once the `ttl` has ended, B's person's next
/// `skald recv` says none waits; C reads B's `refused` receipt of it, whose reason says no one
/// decided. claude's `skald recv` gets nothing, and the person's `skald held` has nothing to decide.
#[test]
fn held_message_undecided_past_its_wait_is_refused_saying_no_one_decided() {
    let mut roles = holding(
        [4_739_006_001, 4_739_006_002],
        ["ada-agent-left-waiting", "bob-waits-for-a-decision"],
    );
    let from = roles.senders.bob_agent();
    let mut line = Line::new(&from, 1, TEXT);
    line.to = Some(roles.full(CLAUDE));
    let said = roles.senders.agents.bob_says(&line);
    let mut short: Envelope = said.envelope().clone();
    short.header.ttl = Some(TTL);
    let short = RawEnvelope::from_bytes(short.to_cbor()).expect("an envelope");
    let msg_id = sends(&roles.senders.agents.talk, &short);
    says_waiting(roles.b(), 1);
    let taken = Instant::now();
    c_reads(&mut roles, msg_id, ReceiptStatus::Held, &from);

    thread::sleep((taken + Duration::from_secs(TTL + 2)).saturating_duration_since(Instant::now()));

    says_none_waiting(roles.b(), "past the message's wait");
    let refused = c_reads(&mut roles, msg_id, ReceiptStatus::Refused, &from);
    let reason = reason_of(&refused);
    assert!(
        reason.to_lowercase().contains(NO_ONE_DECIDED),
        "{reason:?} does not say {NO_ONE_DECIDED:?}"
    );
    assert_not_handed(&roles.claude_reads(false).shown(), TEXT);
    let folder = roles.senders.agents.folder.clone();
    nothing_to_decide(roles.b(), &folder, &[TEXT]);
}
