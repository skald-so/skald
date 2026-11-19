//! `--json` and the person's full lines carry each message's level.

use skald_proto::MessageId;

use crate::agents::CLAUDE;
use crate::kit::{RECV, received};
use crate::lines::{JSON_FLAG, json_objects};
use crate::senders::{OWN, PEER, Senders, holds_word};
use crate::world::AGENT;

/// What the senders say; none is anywhere else.
const LAPTOP_SAYS: &str = "json level test: ada's laptop reports the build";
const BOB_SAYS: &str = "json level test: bob reports the review";

/// The one message object of `objects` whose `text` is `text`; fails unless there is one.
#[track_caller]
fn message_of<'o>(objects: &'o [serde_json::Value], text: &str) -> &'o serde_json::Value {
    let found: Vec<&serde_json::Value> = (objects.iter())
        .filter(|object| object["text"].as_str() == Some(text))
        .collect();
    let [object] = found.as_slice() else {
        panic!("not one message object of {text:?}: {objects:#?}");
    };
    object
}

/// The lines of `shown`, the person's full lines, from the one that holds `msg_id` to the one that
/// holds `text`, joined; fails unless one line holds each, in that order.
#[track_caller]
fn message_lines(shown: &str, (msg_id, text): (MessageId, &str)) -> String {
    let lines: Vec<&str> = shown.lines().collect();
    let id = msg_id.to_string();
    let first: Vec<usize> = (0..lines.len())
        .filter(|&at| lines[at].contains(&id))
        .collect();
    let last: Vec<usize> = (0..lines.len())
        .filter(|&at| lines[at].contains(text))
        .collect();
    match (first.as_slice(), last.as_slice()) {
        ([first], [last]) if first <= last => lines[*first..=*last].join("\n"),
        _ => panic!("not one line of {msg_id} before one of {text:?}: {shown}"),
    }
}

/// Ada's device has run as claude in `~/work/acme`; her second machine is linked and in the
/// channel. Its person says a line, and Bob's device says one. claude's `skald recv --json` gives
/// the laptop's message with `level` `own` and Bob's with `peer`; the person's `skald recv` shows
/// the laptop's message with the word `own` and not `peer`, and Bob's with `peer`.
#[test]
fn json_and_the_person_s_full_lines_carry_each_level() {
    let mut senders = Senders::new(
        [4_152_003_001, 4_152_003_002],
        ["ada-reads-json-levels", "bob-sends-json-levels"],
    );
    let laptop = senders.laptop_person_says(LAPTOP_SAYS);
    let bob = senders.agents.bob();
    let from_bob = senders.bob_says_as(&bob, BOB_SAYS);
    let talk = &senders.agents.talk;

    let json = (talk.ada.machine)
        .start_in(&senders.agents.folder, &[AGENT, CLAUDE, RECV, JSON_FLAG])
        .finish();
    let person = received(&talk.ada.machine).shown();

    assert!(json.success, "{json:?}");
    let objects = json_objects(&json.stdout);
    for (text, level) in [(LAPTOP_SAYS, OWN), (BOB_SAYS, PEER)] {
        let object = message_of(&objects, text);
        assert_eq!(object["level"].as_str(), Some(level), "{object}");
    }
    let own = message_lines(&person, (laptop, LAPTOP_SAYS));
    assert!(holds_word(&own, OWN), "not {OWN}: {own}");
    assert!(!holds_word(&own, PEER), "{PEER}: {own}");
    let peer = message_lines(&person, (from_bob, BOB_SAYS));
    assert!(holds_word(&peer, PEER), "not {PEER}: {peer}");
}
