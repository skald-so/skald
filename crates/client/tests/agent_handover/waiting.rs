//! How long a message waits for an agent: until its envelope's `ttl` ends.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{Cbor as _, Envelope, RawEnvelope};

use crate::gaps::sends;
use crate::kit::received;
use crate::talk::{BOB_HOST, Line, assert_shows_once};
use crate::{Agents, CLAUDE, assert_handed_line, assert_not_handed};

/// The `ttl` of the message that waits shortly, in seconds.
const TTL: u64 = 10;

/// The texts Bob's device sends; none is anywhere else.
const KEPT: &str = "quarterly report draft is in the shared folder";
const SHORT: &str = "standup moved to the small room for today";

/// `envelope` with the `ttl` `ttl`.
fn with_ttl(envelope: &RawEnvelope, ttl: u64) -> RawEnvelope {
    let mut envelope: Envelope = envelope.envelope().clone();
    envelope.header.ttl = Some(ttl);
    RawEnvelope::from_bytes(envelope.to_cbor()).expect("an envelope")
}

/// Ada's device has run as claude in `~/work/acme`. Bob's device sends two messages to the
/// channel, the second with a `ttl` of [`TTL`] seconds. The person's `skald recv` shows both.
/// claude runs again only once that `ttl` has ended: it gets the first, but not the second.
#[test]
fn agent_that_has_not_run_since_a_message_came_gets_it_but_not_once_its_wait_ended() {
    let mut agents = Agents::new(
        [4_147_074_001, 4_147_074_002],
        ["ada-agent-runs-late", "bob-sends-short-lived"],
    );
    agents.run_as(CLAUDE);
    let bob = agents.bob();
    let kept = agents.bob_sends(&Line::new(&bob, 1, KEPT));
    let short = with_ttl(&agents.bob_says(&Line::new(&bob, 2, SHORT)), TTL);
    sends(&agents.talk, &short);
    let person = received(&agents.talk.ada.machine).shown();
    let came = Instant::now();
    let channel = agents.talk.channel;
    for text in [KEPT, SHORT] {
        assert_shows_once(&person, text, (channel, &bob, BOB_HOST));
    }
    thread::sleep((came + Duration::from_secs(TTL + 2)).saturating_duration_since(Instant::now()));

    let claude = agents.run_as(CLAUDE).shown();

    assert_handed_line(&claude, KEPT, kept);
    assert_not_handed(&claude, SHORT);
}
