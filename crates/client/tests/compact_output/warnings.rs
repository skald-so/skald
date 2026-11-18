//! An agent's warnings and refusals: each names a message by its reference and a device by its
//! name; the person's `skald recv` shows in full what an agent's run warned of.

use skald_proto::MessageId;

use crate::agents::{Agents, CLAUDE, CODEX};
use crate::front::Answers;
use crate::gaps::{MISSING, msg_id_of, reports_of, second_dropped};
use crate::kit::received;
use crate::lines::{agent_line, agent_notes, assert_no_uuid, lines_referring_to};
use crate::said::send_as;
use crate::talk::{BOB_HOST, Line, Talk};
use crate::world::WARNING;

/// What Bob's device and claude say; none is anywhere else.
const SAYS: [&str; 3] = [
    "warning test: the first of three",
    "warning test: the second, which never arrives",
    "warning test: the third of three",
];
const FORGED: &str = "warning test: a message whose messageId is not its msg_id";
const REFUSED: &str = "warning test: claude's message the server refuses later";

/// The problem the front refuses a message with, in place of the server.
const REFUSAL: &str = "not-a-member";

/// The one line of `shown`, an agent's output, that refers to `msg_id` and is no message; fails
/// unless there is one.
#[track_caller]
fn note_of(shown: &str, msg_id: MessageId) -> String {
    let notes: Vec<String> = (lines_referring_to(shown, msg_id).into_iter())
        .filter(|line| agent_line(line).is_none())
        .collect();
    let [note] = notes.as_slice() else {
        panic!("not one note of {msg_id}: {shown}");
    };
    note.clone()
}

/// Ada's device has run as claude in `~/work/acme`. A proxy puts into Ada's catch-up Bob's content
/// whose `messageId` is not its `msg_id`. claude's next `skald recv` warns of it on its one line,
/// by the reference of the envelope's `msg_id`.
#[test]
fn agent_warning_of_a_dropped_message_names_it_by_its_reference() {
    let mut agents = Agents::new(
        [4_149_051_001, 4_149_051_002],
        ["ada-agent-told-what-dropped", "bob-forged-once-more"],
    );
    agents.run_as(CLAUDE);
    let forged = {
        let mut line = Line::new(&agents.bob(), 1, FORGED);
        line.message_id = Some(MessageId::generate());
        agents.bob_says(&line)
    };
    let msg_id = msg_id_of(&forged);
    (agents.talk.front).inject(agents.talk.ada.device, vec![forged]);

    let claude = agents.run_as(CLAUDE).shown();

    let note = note_of(&claude, msg_id);
    assert!(note.contains(BOB_HOST), "{note}");
    assert_eq!(agent_notes(&claude), [note]);
}

/// Ada's device has run as claude in `~/work/acme`. Bob's device sends three messages to the
/// channel; the second never reaches the server. claude's next `skald recv`, the first to get
/// them, warns on one line, without IDs, that a message from Bob's device is missing before the
/// third, by its reference. The person's next `skald recv` reports the missing message once, in
/// full, with the channel and Bob's device.
#[test]
fn agent_is_warned_of_missing_messages_by_the_next_ones_reference_and_the_person_in_full() {
    let mut agents = Agents::new(
        [4_149_052_001, 4_149_052_002],
        ["ada-agent-first-sees-gap", "bob-loses-one-midway"],
    );
    agents.run_as(CLAUDE);
    let (_, third) = second_dropped(&mut agents.talk, SAYS);

    let claude = agents.run_as(CLAUDE).shown();

    let note = note_of(&claude, third);
    assert!(
        note.contains(BOB_HOST) && note.to_lowercase().contains(MISSING),
        "{note}"
    );
    assert_eq!(agent_notes(&claude), [note]);
    assert_no_uuid(&claude);
    let person = received(&agents.talk.ada.machine).shown();
    let bob = agents.talk.played[0].device();
    let reports = reports_of(&person, agents.talk.channel, bob);
    assert_eq!(reports.len(), 1, "{person}");
}

/// The server's answer to claude's message from `~/work/acme` is lost, so the message waits in the
/// outbox. The server then refuses each message, and codex's `skald recv` sends it: codex's output
/// says nothing of it. claude's next `skald recv` says, on one line, that it was refused, by its
/// reference, with the problem; the one after says nothing of it. The person's next `skald recv`
/// warns in full that the server refused it.
#[test]
fn agent_is_told_of_the_refusal_of_its_message_whichever_run_got_it() {
    let talk = Talk::new(
        &[4_149_053_001, 4_149_053_002],
        &["ada-agent-refused-later", "bob-never-reads-it"],
        true,
    );
    let agents = Agents::bound(talk);
    agents.run_as(CODEX);
    agents.talk.front.answer(Answers::Lose);
    let (lost, sent) = send_as(&agents, (CLAUDE, &[]), REFUSED);
    assert!(!lost.success, "{lost:?}");
    let msg_id = sent[0].envelope().header.msg_id;
    agents
        .talk
        .front
        .answer(Answers::Refuse(REFUSAL.to_owned()));

    let codex = agents.run_as(CODEX).shown();

    let lines = lines_referring_to(&codex, msg_id);
    assert!(lines.is_empty(), "codex is told: {lines:?}");
    assert_eq!(agent_notes(&codex), Vec::<String>::new());
    let claude = agents.run_as(CLAUDE).shown();
    let note = note_of(&claude, msg_id);
    assert!(note.contains("refused") && note.contains(REFUSAL), "{note}");
    assert_eq!(agent_notes(&claude), [note]);
    let again = agents.run_as(CLAUDE).shown();
    assert_eq!(agent_notes(&again), Vec::<String>::new());
    let person = received(&agents.talk.ada.machine).shown();
    let warned: Vec<&str> = (person.lines())
        .filter(|line| line.to_lowercase().contains(WARNING))
        .filter(|line| line.contains(&msg_id.to_string()) && line.contains(REFUSAL))
        .collect();
    assert_eq!(warned.len(), 1, "{person}");
}
