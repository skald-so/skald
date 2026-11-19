//! `replied` at the sender: a reply in the channel of one of the device's messages, to it and from
//! its `to` agent, is handed to the agent it is to, or shown to the person, and gives the message
//! `replied`; the person's `skald recv` shows it, an agent's only `--json`.

use skald_proto::MessageId;

use crate::agents::{Agents, CLAUDE, CODEX, assert_not_handed};
use crate::fixtures::{ALL, TO};
use crate::kit::{RECV, received};
use crate::lines::{JSON_FLAG, json_objects, lines_referring_to, objects_of, refers_to};
use crate::replying::{
    BOB_CLAUDE, BOB_CODEX, REPLIED, Writes, ada_sends, agent_of, assert_reply_line, bob_reads,
    bob_writes, handles, replied_lines, run_in, the_one,
};
use crate::world::ACME;

/// What Ada's machine sends and Bob's device answers; none is anywhere else.
const ASKED: &str = "replied test: claude asks bob's claude to review the migration";
const ANSWERED: &str = "replied test: bob's claude approved the migration";
const ASKED_JSON: &str = "replied test: claude asks bob's claude for the release date";
const ANSWERED_JSON: &str = "replied test: bob's claude says the release is on friday";
const PERSON_ASKED: &str = "replied test: ada asks bob's claude for the invoice";
const PERSON_ANSWERED: &str = "replied test: bob's claude sent ada the invoice";

/// Ada's claude sends each of these with `--to` Bob's claude, but `TO_ALL`, which it sends with
/// `--all`; Bob's device answers each with the reply of the same index.
const TO_ALL: &str = "replied test: claude tells everyone the build is red";
const TO_BOBS_CLAUDE: [&str; 3] = [
    "replied test: claude asks bob's claude, codex answers",
    "replied test: claude asks bob's claude, the answer is in the default channel",
    "replied test: claude asks bob's claude, which answers",
];
const ANSWERS: [&str; 4] = [
    "replied test: bob's claude answers the message to everyone",
    "replied test: bob's codex answers what was not asked of it",
    "replied test: bob's claude answers in the default channel",
    "replied test: bob's claude answers where it was asked",
];

/// Ada's claude sends `text` with `--to` Bob's claude in the talk's channel, which is not `acme`'s
/// default one; gives its `msg_id`, as Bob's device reads it.
#[track_caller]
fn claude_asks_bobs_claude(agents: &mut Agents, text: &str) -> MessageId {
    let (bob, _) = handles(&agents.talk);
    let to = agent_of(&bob, ACME, BOB_CLAUDE);
    ada_sends(agents, Some(CLAUDE), &[TO, &to], text);
    let channel = agents.talk.channel;
    the_one(&bob_reads(&mut agents.talk, channel), text).0
}

/// Bob's device answers `asked` in the talk's channel as Bob's claude, `to` Ada's claude, with
/// `hops` 1 and the first counter of Bob's device there; gives the answer's `msg_id`.
#[track_caller]
fn bobs_claude_answers(agents: &mut Agents, asked: MessageId, text: &str) -> MessageId {
    let (bob, ada) = handles(&agents.talk);
    let from = agent_of(&bob, ACME, BOB_CLAUDE);
    let reply = Writes::new(&from, Some(&agent_of(&ada, ACME, CLAUDE)), 1, text);
    let at = agents.talk.at();
    bob_writes(&mut agents.talk, at, &reply.hops(1).replying_to(asked))
}

/// Ada's device has run as codex in `~/work/acme`. claude sends a message there with `--to` Bob's
/// claude in the talk's channel, not the default one, and Bob's device answers it in that channel as
/// Bob's claude, `to` Ada's claude. codex's next `skald recv` does not show the answer. claude's
/// shows it once, on a line that names the message it answers by its reference, and holds no other
/// line of that message: no `replied`. The person's next `skald recv` says `replied` of claude's
/// message.
#[test]
fn reply_in_another_channel_is_handed_to_the_asking_agent_only_and_the_person_sees_replied() {
    let mut agents = Agents::new(
        [4_656_001_001, 4_656_001_002],
        ["ada-asks-for-a-review", "bob-reviews-from-his-agent"],
    );
    agents.run_as(CODEX);
    let asked = claude_asks_bobs_claude(&mut agents, ASKED);
    let answered = bobs_claude_answers(&mut agents, asked, ANSWERED);

    let codex = agents.run_as(CODEX).shown();
    let claude = agents.run_as(CLAUDE).shown();

    assert_not_handed(&codex, ANSWERED);
    assert_reply_line(&claude, ANSWERED, (answered, asked));
    let of_asked = lines_referring_to(&claude, asked);
    assert_eq!(of_asked.len(), 1, "other lines of the message: {claude}");
    // The answer's own text holds the word.
    let mut others = claude.lines().filter(|line| !line.contains(ANSWERED));
    assert!(
        others.all(|line| !line.to_lowercase().contains(REPLIED)),
        "{claude}"
    );

    let person = received(&agents.talk.ada.machine).shown();

    let replied = replied_lines(&person, asked, &[ANSWERED]);
    assert_eq!(replied.len(), 1, "not {REPLIED} once: {person}");
}

/// claude sends a message with `--to` Bob's claude in the talk's channel, and Bob's device answers
/// it there as Bob's claude. claude's `skald recv --json` gives the answer with `reply_to` the
/// `msg_id` of claude's message, and a status of that message, `replied`.
#[test]
fn agent_s_json_gives_the_reply_s_reply_to_and_replied() {
    let mut agents = Agents::new(
        [4_656_002_001, 4_656_002_002],
        ["ada-reads-replies-in-json", "bob-answers-in-json"],
    );
    let asked = claude_asks_bobs_claude(&mut agents, ASKED_JSON);
    let answered = bobs_claude_answers(&mut agents, asked, ANSWERED_JSON);
    let at = (agents.folder.as_path(), Some(CLAUDE));

    let got = run_in(&agents.talk.ada.machine, at, RECV, &[JSON_FLAG]);

    let objects = json_objects(&got.stdout);
    assert!(got.success && !objects.is_empty(), "{got:?}");
    let message = objects_of(&objects, answered, "text");
    let [message] = message.as_slice() else {
        panic!("not one message object of the answer: {objects:#?}");
    };
    assert_eq!(
        message["reply_to"].as_str(),
        Some(asked.to_string().as_str()),
        "{message}"
    );
    let statuses = objects_of(&objects, asked, "status");
    let replied: Vec<_> = (statuses.iter())
        .filter(|status| status["status"].as_str() == Some(REPLIED))
        .collect();
    let [replied] = replied.as_slice() else {
        panic!("not one {REPLIED} of claude's message: {objects:#?}");
    };
    let short = replied["ref"].as_str().unwrap_or_default();
    assert!(refers_to(short, asked), "its ref: {replied}");
}

/// Ada's device has run as claude and codex in `~/work/acme`. The person sends a message there with
/// `--to` Bob's claude in the talk's channel, and Bob's device answers it as Bob's claude, `to`
/// Ada's person address. Neither claude's nor codex's next `skald recv` shows the answer; the
/// person's next `skald recv` shows it and says `replied` of the person's message.
#[test]
fn reply_to_the_person_s_message_reaches_the_person_and_no_agent() {
    let mut agents = Agents::new(
        [4_656_003_001, 4_656_003_002],
        ["ada-asks-in-person", "bob-answers-ada-herself"],
    );
    agents.run_as(CLAUDE);
    agents.run_as(CODEX);
    let (bob, _) = handles(&agents.talk);
    ada_sends(
        &agents,
        None,
        &[TO, &agent_of(&bob, ACME, BOB_CLAUDE)],
        PERSON_ASKED,
    );
    let channel = agents.talk.channel;
    let asked = the_one(&bob_reads(&mut agents.talk, channel), PERSON_ASKED).0;
    let ada = agents.talk.address(&agents.talk.ada);
    let reply = Writes::new(
        &agent_of(&bob, ACME, BOB_CLAUDE),
        Some(&ada),
        1,
        PERSON_ANSWERED,
    );
    let at = agents.talk.at();
    bob_writes(&mut agents.talk, at, &reply.hops(1).replying_to(asked));

    for agent in [CLAUDE, CODEX] {
        assert_not_handed(&agents.run_as(agent).shown(), PERSON_ANSWERED);
    }
    let person = received(&agents.talk.ada.machine).shown();

    assert_eq!(person.matches(PERSON_ANSWERED).count(), 1, "{person}");
    let replied = replied_lines(&person, asked, &[PERSON_ANSWERED]);
    assert_eq!(replied.len(), 1, "not {REPLIED} once: {person}");
}

/// claude sends a message with `--all` and three with `--to` Bob's claude in the talk's channel.
/// Bob's device answers them, each `to` Ada's claude with the message as `reply_to`: the first as
/// Bob's claude, the second as Bob's codex, the third as Bob's claude in `acme`'s default channel,
/// the fourth as Bob's claude in the talk's channel. The person's next `skald recv` says `replied`
/// of the fourth only.
#[test]
fn no_replied_for_a_message_to_everyone_from_another_agent_or_in_another_channel() {
    let mut agents = Agents::new(
        [4_656_004_001, 4_656_004_002],
        [
            "ada-counts-only-real-replies",
            "bob-answers-every-which-way",
        ],
    );
    ada_sends(&agents, Some(CLAUDE), &[ALL], TO_ALL);
    let channel = agents.talk.channel;
    let to_all = the_one(&bob_reads(&mut agents.talk, channel), TO_ALL).0;
    let mut asked = vec![to_all];
    for text in TO_BOBS_CLAUDE {
        asked.push(claude_asks_bobs_claude(&mut agents, text));
    }
    let (bob, ada) = handles(&agents.talk);
    let to = agent_of(&ada, ACME, CLAUDE);
    let (bob_claude, bob_codex) = (
        agent_of(&bob, ACME, BOB_CLAUDE),
        agent_of(&bob, ACME, BOB_CODEX),
    );
    let (space, default) = (agents.talk.space.id, agents.talk.space.default);
    let replies = [
        (&bob_claude, (space, channel), 1),
        (&bob_codex, (space, channel), 2),
        (&bob_claude, (space, default), 1),
        (&bob_claude, (space, channel), 3),
    ];
    for (index, (from, at, counter)) in replies.into_iter().enumerate() {
        let reply = Writes::new(from, Some(&to), counter, ANSWERS[index]);
        bob_writes(
            &mut agents.talk,
            at,
            &reply.hops(1).replying_to(asked[index]),
        );
    }

    let person = received(&agents.talk.ada.machine).shown();

    for (index, msg_id) in asked.iter().enumerate() {
        let replied = replied_lines(&person, *msg_id, &ANSWERS);
        let expected = usize::from(index == 3);
        assert_eq!(
            replied.len(),
            expected,
            "{REPLIED} of message {index}: {replied:?}: {person}"
        );
    }
}
