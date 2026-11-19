//! A message as an agent gets it: one line by its reference, and longer references only where two
//! kept messages would share theirs.

use std::collections::HashMap;

use skald_proto::MessageId;

use crate::agents::{Agents, CLAUDE};
use crate::gaps::sends;
use crate::kit::received;
use crate::lines::{DATA_WARNING, SHORTEST, assert_agent_got, reference, uuids};
use crate::talk::{BOB_HOST, Line, assert_shows_once, encrypted_as};

/// What Bob's device says; none is anywhere else.
const PLAIN: &str = "compact test: the staging deploy is green";
const CONTROL: &str = "compact test: \u{1b}[31mred\u{1b}[0m alert\u{7}";
/// [`CONTROL`] as the CLI shows it, its control characters escaped as in the person's lines.
const CONTROL_SHOWN: &str = r"compact test: \u{1b}[31mred\u{1b}[0m alert\u{7}";
const ALIKE: [&str; 2] = [
    "compact test: the first of two alike references",
    "compact test: the second of two alike references",
];
const APART: &str = "compact test: a reference of its own";

/// The first [`SHORTEST`] characters of `msg_id`'s reference.
fn leading(msg_id: MessageId) -> String {
    reference(msg_id)[..SHORTEST].to_owned()
}

/// Two new message IDs whose references share their first [`SHORTEST`] characters.
fn alike() -> (MessageId, MessageId) {
    let mut seen = HashMap::new();
    loop {
        let id = MessageId::generate();
        if let Some(earlier) = seen.insert(leading(id), id) {
            return (earlier, id);
        }
    }
}

/// A new message ID whose reference shares no first [`SHORTEST`] characters with `other`'s.
fn apart_from(other: MessageId) -> MessageId {
    loop {
        let id = MessageId::generate();
        if leading(id) != leading(other) {
            return id;
        }
    }
}

/// Ada's device has run as claude in `~/work/acme`. Bob's device sends two messages to the
/// channel, the second with control characters. The person's `skald recv` shows each with its
/// `msg_id`. claude's next run shows each as one line,
/// `[<from> · <device> · <level>] #<ref>: <text>`, from Bob's person address and his device's
/// name at the level `peer`, the text escaped, unmarked, its reference the first 4 characters of
/// the reference of that `msg_id`, after one warning line that the person's run does not show; its
/// output holds no UUID and no control character.
#[test]
fn agent_gets_a_message_as_one_line_by_its_reference_and_no_id() {
    let mut agents = Agents::new(
        [4_149_011_001, 4_149_011_002],
        ["ada-reads-short-lines", "bob-writes-short-lines"],
    );
    agents.run_as(CLAUDE);
    let bob = agents.bob();
    let plain = agents.bob_sends(&Line::new(&bob, 1, PLAIN));
    let control = agents.bob_sends(&Line::new(&bob, 2, CONTROL));
    let person = received(&agents.talk.ada.machine).shown();
    let channel = agents.talk.channel;
    for (text, id) in [(PLAIN, plain), (CONTROL_SHOWN, control)] {
        assert_shows_once(&person, text, (channel, &bob, BOB_HOST));
        assert!(person.contains(&id.to_string()), "no {id}: {person}");
    }
    assert!(!person.contains(DATA_WARNING), "{person}");

    let claude = agents.run_as(CLAUDE).shown();

    for (text, id) in [(PLAIN, plain), (CONTROL_SHOWN, control)] {
        let got = assert_agent_got(&claude, text, (id, &bob, BOB_HOST));
        assert!(!got.after_missing, "marked: {got:?}");
        assert_eq!(got.level, "peer", "{got:?}");
        if leading(plain) != leading(control) {
            assert_eq!(got.reference.len(), SHORTEST, "{got:?}");
        }
    }
    let warned: Vec<&str> = claude
        .lines()
        .filter(|line| *line == DATA_WARNING)
        .collect();
    assert_eq!(warned.len(), 1, "not one warning line: {claude}");
    assert!(
        claude.starts_with(DATA_WARNING),
        "the warning line after a message: {claude}"
    );
    assert!(
        !claude.contains(['\u{1b}', '\u{7}']),
        "a control character reaches the agent: {claude:?}"
    );
    assert_eq!(uuids(&claude), Vec::<String>::new(), "{claude}");
}

/// Ada's device has run as claude in `~/work/acme`. Bob's device sends three messages to the
/// channel: two whose `msg_id`s have references that share their first 4 characters, and one
/// whose reference shares them with neither. claude's next run shows the first two with longer,
/// distinct references, each of its own `msg_id`, and the third with 4 characters.
#[test]
fn kept_messages_whose_references_share_four_characters_get_longer_distinct_ones() {
    let mut agents = Agents::new(
        [4_149_012_001, 4_149_012_002],
        ["ada-tells-alike-refs-apart", "bob-sends-alike-refs"],
    );
    agents.run_as(CLAUDE);
    let bob = agents.bob();
    let (first, second) = alike();
    let apart = apart_from(first);
    let said = [(first, ALIKE[0]), (second, ALIKE[1]), (apart, APART)];
    let at = agents.talk.at();
    for (counter, (id, text)) in (1..).zip(said) {
        let line = Line::new(&bob, counter, text);
        let envelope = encrypted_as(&mut agents.talk.played[0], at, id, &line);
        sends(&agents.talk, &envelope);
    }

    let claude = agents.run_as(CLAUDE).shown();

    let [first, second, apart] =
        said.map(|(id, text)| assert_agent_got(&claude, text, (id, &bob, BOB_HOST)).reference);
    for longer in [&first, &second] {
        assert!(longer.len() > SHORTEST, "#{longer} is not longer: {claude}");
    }
    assert_ne!(
        first.to_lowercase(),
        second.to_lowercase(),
        "the same reference: {claude}"
    );
    assert_eq!(apart.len(), SHORTEST, "#{apart}: {claude}");
}
