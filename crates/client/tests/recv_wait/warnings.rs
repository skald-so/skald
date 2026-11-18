//! The warnings of what the helper takes in for agents' sessions alone: each session gets its
//! short lines, and the person's next `skald recv` shows each warning once, in full.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::MessageId;

use crate::agents::CLAUDE;
use crate::cloud::WAIT;
use crate::joins::{kept, removal_warnings};
use crate::kit::{RECV, received};
use crate::lines::{agent_notes, lines_referring_to};
use crate::machine::{Finished, Run};
use crate::talk::{BOB_HOST, Line};
use crate::waits::{TRACE_SETTING, Waits, assert_got, assert_keeps_waiting};
use crate::world::{WARNING, now, warnings};

/// What Bob's device says; none is anywhere else.
const FORGED: &str = "wait drop test: a message whose messageId is not its msg_id";
const AFTER_FORGED: &str = "wait drop test: the next one is fine";

/// What a session that says when the helper's steps ran shows each time the helper took something
/// in for it.
const TAKEN: &str = "skald: trace: handing over";

/// Starts claude's `skald recv --wait` in `~/work/acme` through a helper, with `settings`, once the
/// device has run as claude and the person's `skald recv` has shown what came so far; fails unless
/// it keeps waiting.
fn waiting(ids: [u64; 2], logins: [&str; 2], settings: &[(&str, &str)]) -> (Waits, Run) {
    let waits = Waits::new(ids, logins);
    waits.agents.run_as(CLAUDE);
    received(&waits.agents.talk.ada.machine);
    let mut session = waits.session_with(settings, CLAUDE);
    assert_keeps_waiting(&mut session);
    (waits, session)
}

/// The warnings of `person`'s output that name `msg_id`.
fn warned_of(person: &Finished, msg_id: MessageId) -> Vec<String> {
    (warnings(person).into_iter())
        .filter(|line| line.contains(&msg_id.to_string()))
        .collect()
}

/// claude waits through a helper. Bob's device sends a message whose `messageId` is not its
/// `msg_id`, then one that is fine: the session shows a line that the first was dropped, by its
/// reference and Bob's device's name, without the full warning, then the second, and exits. The
/// person's next `skald recv` warns of the dropped one once, in full; the one after, not again.
#[test]
fn waiting_agent_gets_a_dropped_message_s_short_line_and_the_person_its_warning() {
    let (mut waits, session) = waiting(
        [4_149_071_001, 4_149_071_002],
        ["ada-waits-for-a-forgery", "bob-forges-for-a-session"],
        &[],
    );
    let bob = waits.agents.bob();
    let mut forged = Line::new(&bob, 1, FORGED);
    forged.message_id = Some(MessageId::generate());
    let dropped = waits.agents.bob_sends(&forged);
    let id = waits.agents.bob_sends(&Line::new(&bob, 2, AFTER_FORGED));

    let claude = session.finish();

    assert_got(&claude, AFTER_FORGED, id);
    let shown = claude.shown();
    let notes = lines_referring_to(&shown, dropped);
    assert!(
        notes.len() == 1 && notes[0].contains(BOB_HOST),
        "{claude:?}"
    );
    assert_eq!(agent_notes(&shown), notes, "{claude:?}");
    assert!(!shown.to_lowercase().contains(WARNING), "{claude:?}");
    let person = waits.helped.run(&[RECV]);
    assert_eq!(warned_of(&person, dropped).len(), 1, "{person:?}");
    let again = waits.helped.run(&[RECV]);
    assert_eq!(
        warned_of(&again, dropped),
        Vec::<String>::new(),
        "{again:?}"
    );
}

/// claude waits through a helper, saying when it took something in. Bob's device commits the Remove
/// of Ada's device, which the roster admits: the helper takes it in, and the session keeps waiting.
/// The person's next `skald recv` warns once that a Commit removed the device from the channel.
#[test]
fn warning_of_taking_in_for_a_waiting_agent_is_kept_for_the_person() {
    let (mut waits, mut session) = waiting(
        [4_149_072_001, 4_149_072_002],
        ["ada-removed-while-waiting", "bob-removes-a-waiting-device"],
        &[(TRACE_SETTING, "1")],
    );
    let before = session.shown().matches(TAKEN).count();
    let talk = &mut waits.agents.talk;
    let (at, channel, ada) = (talk.at(), talk.channel, talk.ada.device);
    let bob = &mut talk.played[0];
    let commit = (bob.mls.remove_devices(channel, &[ada], b"", now())).expect("a Commit");
    kept(&talk.world.cloud, bob, at, &commit);

    let until = Instant::now() + WAIT;
    while session.shown().matches(TAKEN).count() == before {
        assert!(Instant::now() < until, "the helper took nothing in");
        thread::sleep(Duration::from_millis(100));
    }

    session.assert_running();
    let person = waits.helped.run(&[RECV]);
    assert_eq!(removal_warnings(&person, channel).len(), 1, "{person:?}");
}
