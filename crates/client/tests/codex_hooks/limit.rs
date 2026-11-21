//! Every hook output stays within Codex's hook output limit, however many messages wait.

use std::thread;
use std::time::{Duration, Instant};

use crate::agents::{Agents, CODEX as AGENT_NAME};
use crate::codex::{Event, Thread, given, hook_command};
use crate::gaps::bob_says;
use crate::helped::Helped;
use crate::hooks::run_hook;
use crate::kit::RECV;
use crate::world::run;

/// How many messages Bob's device sends.
const MANY: u64 = 1_000;

/// The text of Bob's message `counter`; none is anywhere else.
fn note(counter: u64) -> String {
    format!("codex limit test: Bob's note {counter:04} of {MANY}")
}

/// How many messages Bob's device sends between two times the person's `skald recv` on Ada's
/// machine shows the last: no run takes in so many that it lasts past the test kit's limit on a
/// loaded machine.
const BATCH: u64 = 100;

/// How long Ada's machine may take to take in [`MANY`] messages: a slow machine, as Windows', takes
/// over a minute.
const TAKE_IN: Duration = Duration::from_secs(300);

/// What a run says when the helper stays busy, as taking messages in, past its request timeout.
const BUSY: &str = "the helper of the local state is busy";

/// Waits for up to [`TAKE_IN`] until the person's `skald recv` on Ada's machine has shown `text`;
/// a run that gave up on a busy helper runs again.
#[track_caller]
fn person_has_seen(agents: &Agents, text: &str) {
    let deadline = Instant::now() + TAKE_IN;
    loop {
        let finished = run(&agents.talk.ada.machine, &[RECV]);
        let busy = !finished.success && finished.shown().contains(BUSY);
        assert!(finished.success || busy, "{finished:?}");
        if finished.shown().contains(text) {
            return;
        }
        assert!(Instant::now() < deadline, "{text:?} not taken in");
        thread::sleep(Duration::from_millis(500));
    }
}

/// Ada's device has run as an agent in `~/work/acme`, whose Codex session starts there with
/// `--agent`, and its turn starts. Bob's device sends the channel [`MANY`] messages, without `to`;
/// after each [`BATCH`], the person's `skald recv` on Ada's machine shows the last sent. Each hook of the session, three times
/// over, exits with success and prints at most 2,500 bytes; `PostToolUse` gives the signal first.
#[test]
fn hook_outputs_stay_within_the_limit_with_a_thousand_messages_waiting() {
    let mut agents = Agents::new(
        [4_927_014_001, 4_927_014_002],
        ["ada-codex-many-notes", "bob-codex-many-notes"],
    );
    agents.run_as(AGENT_NAME);
    let helped = Helped::new(&agents.talk.ada.machine);
    let session = Thread::new(&agents.talk.ada.machine, &agents.folder, Some(AGENT_NAME));
    let hook = |event: Event| {
        run_hook(
            &mut hook_command(&helped, &session, &[]),
            &session.input(event),
        )
    };
    hook(Event::SessionStart).assert_silent("SessionStart");
    hook(Event::UserPromptSubmit).assert_silent("the turn's start");

    let mut bob = agents.talk.played[0].connect(agents.talk.cloud());
    for counter in 1..=MANY {
        let envelope = bob_says(&mut agents.talk, counter, &note(counter));
        bob.stored(&envelope);
        if counter % BATCH == 0 {
            person_has_seen(&agents, &note(counter));
        }
    }

    let first = hook(Event::PostToolUse);
    assert!(!first.stdout.is_empty(), "no signal: {first:?}");
    given(&first, Event::PostToolUse);
    for _ in 0..3 {
        for event in Event::ALL {
            let hooked = hook(event);
            if hooked.stdout.is_empty() {
                hooked.assert_silent(event.name());
            } else {
                given(&hooked, event);
            }
        }
    }
}
