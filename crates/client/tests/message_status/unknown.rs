//! What an agent may not ask of: another agent's message, one past its wait, and a reference two
//! kept messages share. Each gives nothing, says why and names nothing else.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::MessageId;

use crate::agents::{Agents, CLAUDE, CODEX};
use crate::fixtures::{CHANNEL, SEND, TO};
use crate::kit::received;
use crate::lines::{SHORTEST, agent_line_of, reference};
use crate::replying::{BOB_CLAUDE, agent_of, assert_refused, handles, run_in};
use crate::statuses::{Asked, assert_holds, bob_asks_claude, json_status, status_in};
use crate::talk::{BOB_HOST, one_message_sent};
use crate::world::ACME;

/// What claude sends Bob's claude, and what Bob's device says to Ada's agents; none is anywhere
/// else.
const CLAUDE_SENDS: &str = "unknown status test: claude asks bob's claude for the invoice";
const TO_CLAUDE: &str = "unknown status test: bob's claude sends ada's claude the invoice";
const SHORT_LIVED: &str = "unknown status test: bob's claude says the room is free today only";
const TWINS: [&str; 2] = [
    "unknown status test: the first of two messages whose references start alike",
    "unknown status test: the second of two messages whose references start alike",
];

/// The `ttl` of the short-lived message, in seconds: room for the runs before the first status.
const TTL: u64 = 30;

/// Ada's device has run as claude and codex in `~/work/acme`. claude sends a message `--to` Bob's
/// claude; Bob's device says something to Ada's claude, which claude's `skald recv` shows. As
/// codex, `skald message-status` of either by its `msg_id` fails, says why and names neither text,
/// claude, Bob's device, Bob's agent nor the channel. As claude, and as the person, each gives the
/// message's status.
#[test]
fn another_agent_s_message_is_unknown_to_an_agent_and_names_nothing() {
    let mut agents = Agents::new(
        [4_380_004_001, 4_380_004_002],
        ["ada-keeps-agents-apart", "bob-writes-to-one-agent"],
    );
    agents.run_as(CLAUDE);
    agents.run_as(CODEX);
    let bob_claude = agent_of(&handles(&agents.talk).0, ACME, BOB_CLAUDE);
    let channel = agents.talk.channel;
    let mark = agents.talk.front.sent().len();
    let args = [TO, &bob_claude, CHANNEL, &channel.to_string(), CLAUDE_SENDS];
    let sent = run_in(
        &agents.talk.ada.machine,
        (&agents.folder, Some(CLAUDE)),
        SEND,
        &args,
    );
    assert!(sent.success, "{sent:?}");
    let own = one_message_sent(&agents.talk.front, channel, mark);
    let at = (ACME, agents.talk.at());
    let handed = bob_asks_claude(&mut agents.talk, at, (1, TO_CLAUDE), Asked::default());
    assert!(agents.run_as(CLAUDE).shown().contains(TO_CLAUDE));
    assert!(!agents.run_as(CODEX).shown().contains(TO_CLAUDE));
    let machine = &agents.talk.ada.machine;
    let bob_device = agents.talk.played[0].device().to_string();
    let names: [&str; 7] = [
        CLAUDE_SENDS,
        TO_CLAUDE,
        CLAUDE,
        BOB_CLAUDE,
        BOB_HOST,
        &bob_device,
        &channel.to_string(),
    ];

    for (msg_id, what) in [
        (own, "claude's message"),
        (handed, "a message handed to claude"),
    ] {
        let id = msg_id.to_string();
        let codex = status_in(machine, (&agents.folder, Some(CODEX)), &[], &id);

        assert_refused(&codex, &names, &format!("{what}, as codex"));
        for agent in [Some(CLAUDE), None] {
            let object = json_status(machine, (&agents.folder, agent), &id);
            assert_eq!(
                object["msg_id"].as_str(),
                Some(id.as_str()),
                "{what}: {object}"
            );
        }
    }
}

/// Two new message IDs whose references share their first [`SHORTEST`] characters, and that
/// prefix.
fn alike() -> ([MessageId; 2], String) {
    let mut earlier: Vec<MessageId> = Vec::new();
    loop {
        let next = MessageId::generate();
        let start = reference(next)[..SHORTEST].to_owned();
        if let Some(twin) = earlier
            .iter()
            .find(|one| reference(**one).starts_with(&start))
        {
            return ([*twin, next], start);
        }
        earlier.push(next);
    }
}

/// Ada's device has run as claude in `~/work/acme`. Bob's device says two things to Ada's claude,
/// in envelopes whose `msg_id`s have references that start alike; claude's `skald recv` shows both.
/// As claude, `skald message-status` by the characters both references share fails, says why and
/// gives neither text; by the longer reference claude was shown for the first, it gives the
/// first's status.
#[test]
fn reference_two_kept_messages_share_gives_nothing_and_says_so() {
    let mut agents = Agents::new(
        [4_380_005_001, 4_380_005_002],
        ["ada-agent-asks-alike-refs", "bob-sends-alike-refs-again"],
    );
    agents.run_as(CLAUDE);
    let (ids, shared) = alike();
    let at = (ACME, agents.talk.at());
    for (index, text) in TWINS.into_iter().enumerate() {
        let asked = Asked {
            msg_id: Some(ids[index]),
            ..Asked::default()
        };
        bob_asks_claude(&mut agents.talk, at, (index as u64 + 1, text), asked);
    }
    let shown = agents.run_as(CLAUDE).shown();
    let (machine, folder) = (&agents.talk.ada.machine, agents.folder.as_path());
    let first = agent_line_of(&shown, TWINS[0]).reference;

    let refused = status_in(machine, (folder, Some(CLAUDE)), &[], &format!("#{shared}"));

    assert_refused(&refused, &TWINS, "a shared reference");
    assert!(first.len() > SHORTEST, "#{first} is no longer: {shown}");
    let object = json_status(machine, (folder, Some(CLAUDE)), &format!("#{first}"));
    let id = ids[0].to_string();
    assert_eq!(object["msg_id"].as_str(), Some(id.as_str()), "{object}");
}

/// Ada's device has run as claude in `~/work/acme`. Bob's device says something to Ada's claude in
/// an envelope with a `ttl` of [`TTL`] seconds; the person's `skald recv` shows it and claude's
/// takes it. claude's status of it by its reference gives it; once the `ttl` has ended since the
/// person's run, it fails, says why and names neither the text nor the `msg_id`.
#[test]
fn message_past_its_wait_is_unknown() {
    let mut agents = Agents::new(
        [4_380_006_001, 4_380_006_002],
        ["ada-asks-after-it-expires", "bob-sends-for-today-again"],
    );
    agents.run_as(CLAUDE);
    let asked = Asked {
        ttl: Some(TTL),
        ..Asked::default()
    };
    let at = (ACME, agents.talk.at());
    let msg_id = bob_asks_claude(&mut agents.talk, at, (1, SHORT_LIVED), asked);
    let machine = &agents.talk.ada.machine;
    let written = {
        assert_holds(&received(machine).shown(), &[SHORT_LIVED]);
        Instant::now()
    };
    let claude = agents.run_as(CLAUDE).shown();
    let at = (agents.folder.as_path(), Some(CLAUDE));
    let given = format!("#{}", agent_line_of(&claude, SHORT_LIVED).reference);

    let in_time = json_status(machine, at, &given);

    assert!(
        written.elapsed() < Duration::from_secs(TTL),
        "the status took longer than the ttl; nothing is checked"
    );
    let id = msg_id.to_string();
    assert_eq!(in_time["msg_id"].as_str(), Some(id.as_str()), "{in_time}");
    thread::sleep(
        (written + Duration::from_secs(TTL + 2)).saturating_duration_since(Instant::now()),
    );

    let too_late = status_in(machine, at, &[], &given);

    assert_refused(&too_late, &[SHORT_LIVED, &id], "a message whose wait ended");
}
