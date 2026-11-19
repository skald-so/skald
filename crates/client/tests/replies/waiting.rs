//! How long a message can be replied to: for its wait, its envelope's `ttl` from when the device
//! wrote it, at most 7 days.

use std::thread;
use std::time::{Duration, Instant};

use crate::agents::{Agents, CLAUDE};
use crate::kit::received;
use crate::lines::agent_line_of;
use crate::replying::{
    BOB_CLAUDE, Writes, agent_of, assert_bob_never_reads, assert_refused, bob_reads, bob_writes,
    handles, of_text, replies,
};
use crate::world::ACME;

/// The `ttl` of the message, in seconds: room for the runs before the first reply.
const TTL: u64 = 30;

/// What Bob's device says, and what Ada's claude answers; none is anywhere else.
const SHORT_LIVED: &str = "wait test: bob's claude asks for the standup room, for today only";
const IN_TIME: &str = "wait test: claude answers while the message waits";
const TOO_LATE: &str = "wait test: claude answers after the message's wait ended";

/// Ada's device has run as claude in `~/work/acme`. Bob's device says something to Ada's claude in
/// the talk's channel, in an envelope with a `ttl` of [`TTL`] seconds; the person's `skald recv`
/// shows it and claude's takes it. claude's `skald reply` to it by its reference succeeds; once the
/// `ttl` has ended since the person's run, another fails, says why and sends nothing.
#[test]
fn message_can_be_replied_to_until_its_shortened_wait_ends_not_after() {
    let mut agents = Agents::new(
        [4_656_041_001, 4_656_041_002],
        ["ada-replies-before-it-expires", "bob-sends-for-today-only"],
    );
    agents.run_as(CLAUDE);
    let (bob, ada) = handles(&agents.talk);
    let mut asked = Writes::new(
        &agent_of(&bob, ACME, BOB_CLAUDE),
        Some(&agent_of(&ada, ACME, CLAUDE)),
        1,
        SHORT_LIVED,
    );
    asked.ttl = Some(TTL);
    let at = agents.talk.at();
    bob_writes(&mut agents.talk, at, &asked.hops(1));
    let person = received(&agents.talk.ada.machine).shown();
    let written = Instant::now();
    assert!(person.contains(SHORT_LIVED), "{person}");
    let claude = agents.run_as(CLAUDE).shown();
    let short = format!("#{}", agent_line_of(&claude, SHORT_LIVED).reference);

    let in_time = replies(&agents, Some(CLAUDE), &short, IN_TIME);

    assert!(in_time.success, "{in_time:?}");
    assert!(
        written.elapsed() < Duration::from_secs(TTL),
        "the reply took longer than the ttl; nothing is checked"
    );
    let read = bob_reads(&mut agents.talk, at.1);
    assert_eq!(of_text(&read, IN_TIME).len(), 1, "not sent once: {read:?}");
    let end = written + Duration::from_secs(TTL + 2);
    thread::sleep(end.saturating_duration_since(Instant::now()));

    let too_late = replies(&agents, Some(CLAUDE), &short, TOO_LATE);

    assert_refused(&too_late, &[], "a message whose wait ended");
    let channels = [agents.talk.channel, agents.talk.space.default];
    assert_bob_never_reads(&mut agents.talk, &channels, &[TOO_LATE]);
}
