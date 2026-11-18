//! Commands through the helper act as in one-shot mode: an agent's space is that of its folder,
//! and a refused send or a send with the server down shows what one-shot mode shows.

use crate::fixtures::{SEND, person_address, sent_after};
use crate::front::Answers;
use crate::helped::{Helped, ONE_SHOT, bob_reads, timeout_of, with_client_b};
use crate::kit::{RECV, bind_client_a};
use crate::machine::Finished;
use crate::talk::{
    BOB_HOST, Line, Talk, assert_read, assert_shows_once, encrypted, one_message_sent, stored,
};
use crate::world::{AGENT, CLIENT_A, CLIENT_B, HOST};

/// The agent the tests run as.
const CLAUDE: &str = "claude-h3lp9r";

/// What the agent and Bob's device say.
const AGENT_ASKS: &str = "helper test: which invoices are still open?";
const AGENT_LEAKS: &str = "helper test: this must not reach the other client";
const BOB_IN_A: &str = "helper test: clienta's invoices 7 and 9 are open";
const BOB_IN_B: &str = "helper test: the other client's audit starts monday";

/// What the one-shot and helper runs send.
const FIRST: &str = "helper test: release notes are drafted";
const SECOND: &str = "helper test: release notes are reviewed";
const THIRD: &str = "helper test: release notes are published";

/// The problem the front refuses a message with, in place of the server.
const REFUSAL: &str = "not-a-member";

/// The request timeout of a run with the server down, in seconds.
const DOWN_TIMEOUT: &str = "4";

/// Ada's machine follows `clienta`, whose channel holds Bob's device, and `clientb`, with a
/// channel that holds Bob's device too; `~/work/clientA` is bound to `clienta`. Through one helper,
/// as the agent in `~/work/clientA`: `skald recv` runs first; `skald send` to clienta's channel
/// stores a message Bob's device reads with `from` the agent's address in clienta; `skald send`
/// to clientb's channel fails, sends nothing and names nothing of clientb. Bob's device then says
/// something in each channel: the agent's `skald recv` shows clienta's once and names nothing of
/// clientb; the person's `skald recv` shows both, each once. The same helper served every run.
#[test]
fn through_the_helper_an_agent_sends_and_receives_only_in_the_space_of_its_folder() {
    let mut talk = Talk::in_space(
        CLIENT_A,
        &[4_146_021_001, 4_146_021_002],
        &["ada-agent-via-helper", "bob-client-via-helper"],
        false,
    );
    let (other, elsewhere) = with_client_b(&mut talk);
    let folder = bind_client_a(&talk.ada.machine);
    let helped = Helped::new(&talk.ada.machine);
    let agent = |args: &[&str]| {
        let mut all = vec![AGENT, CLAUDE];
        all.extend_from_slice(args);
        helped.run_in(&folder, &all)
    };
    let first = agent(&[RECV]);
    assert!(first.success, "{first:?}");
    let helper = helped.one_helper();
    let agent_address = format!("{CLIENT_A}/{}/{CLAUDE}@{HOST}", talk.ada.handle);
    let from = talk.front.sent().len();

    let own = agent(&[SEND, &talk.channel.to_string(), AGENT_ASKS]);

    assert!(own.success, "{own:?}");
    let read = [(
        one_message_sent(&talk.front, talk.channel, from),
        1,
        AGENT_ASKS,
    )];
    talk.played[0].take_in(&talk.world.cloud);
    let sender = (talk.ada.device, agent_address.as_str());
    assert_read(&talk.played[0], talk.channel, sender, &read);

    let from = talk.front.sent().len();
    let refused = agent(&[SEND, &elsewhere.to_string(), AGENT_LEAKS]);

    assert!(!refused.success, "{refused:?}");
    assert!(
        sent_after(&talk.front, from).is_empty(),
        "sent to clientb's channel"
    );
    let clientb = [
        CLIENT_B.to_owned(),
        other.id.to_string(),
        elsewhere.to_string(),
    ];
    assert_names_none(&refused, &clientb);

    let from_bob = [
        person_address(CLIENT_A, &talk.played[0].person),
        person_address(CLIENT_B, &talk.played[0].person),
    ];
    let said = [
        ((talk.space.id, talk.channel), &from_bob[0], BOB_IN_A),
        ((other.id, elsewhere), &from_bob[1], BOB_IN_B),
    ];
    for (at, from, text) in said {
        let bob = &mut talk.played[0];
        let envelope = encrypted(bob, at, at.1, &Line::new(from, 1, text));
        stored(&talk.world.cloud, bob, &envelope);
    }

    let as_agent = agent(&[RECV]);

    assert!(as_agent.success, "{as_agent:?}");
    assert_shows_once(
        &as_agent.shown(),
        BOB_IN_A,
        (talk.channel, &from_bob[0], BOB_HOST),
    );
    let mut unseen = clientb.to_vec();
    unseen.push(BOB_IN_B.to_owned());
    assert_names_none(&as_agent, &unseen);

    let person = helped.received();

    let shown = person.shown();
    assert_shows_once(&shown, BOB_IN_B, (elsewhere, &from_bob[1], BOB_HOST));
    assert_shows_once(&shown, BOB_IN_A, (talk.channel, &from_bob[0], BOB_HOST));
    assert_eq!(helped.helpers(), [helper], "not the same helper");
}

/// Fails if `finished` names one of `names`, in any case.
#[track_caller]
fn assert_names_none(finished: &Finished, names: &[String]) {
    let shown = finished.shown().to_lowercase();
    for name in names {
        assert!(
            !shown.contains(&name.to_lowercase()),
            "it names {name}: {finished:?}"
        );
    }
}

/// The front refuses each message in place of the server, which stores it. Ada's `skald send` in
/// one-shot mode fails; the next `skald send`, through the helper it starts, fails with the same
/// exit code and the same output, but for the message's ID. Once the front forwards again, a
/// `skald send` through that helper succeeds, and Bob's device reads the three messages, with the
/// counters 1, 2 and 3.
#[test]
fn refused_send_through_the_helper_shows_what_one_shot_mode_shows() {
    let mut talk = Talk::new(
        &[4_146_022_001, 4_146_022_002],
        &["ada-refused-via-helper", "bob-refusal-via-helper"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let channel = talk.channel.to_string();
    talk.front.answer(Answers::Refuse(REFUSAL.to_owned()));
    let one_shot = helped.run(&[ONE_SHOT, SEND, &channel, FIRST]);
    assert!(!one_shot.success, "{one_shot:?}");
    assert!(one_shot.shown().contains(REFUSAL), "{one_shot:?}");
    helped.assert_none();

    let through = helped.send_with(&talk, &[], SECOND);

    let helper = helped.one_helper();
    assert_same_output(&one_shot, &through);
    talk.front.answer(Answers::Forward);
    helped.sent(&talk, THIRD);
    assert_eq!(helped.helpers(), [helper], "not the same helper");
    bob_reads(&mut talk, &[FIRST, SECOND, THIRD]);
}

/// With the server down for delivery, Ada's `skald send` in one-shot mode, with a timeout of
/// [`DOWN_TIMEOUT`], fails, leaving its message in the outbox; once the server is back, a
/// `skald recv` in one-shot mode sends it. With the server down again, a `skald send` without the
/// one-shot flag, with the same timeout, fails with the same exit code and the same output, but for
/// the message's ID. Once the server is back, the next `skald send` succeeds, and Bob's device
/// reads the three messages, each once, with the counters 1, 2 and 3.
#[test]
fn send_through_the_helper_with_the_server_down_shows_what_one_shot_mode_shows() {
    let mut talk = Talk::new(
        &[4_146_023_001, 4_146_023_002],
        &["ada-offline-via-helper", "bob-offline-via-helper"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let channel = talk.channel.to_string();
    let timeout = timeout_of(DOWN_TIMEOUT);
    talk.front.down(true);
    let one_shot = helped.run_with(&timeout, &[ONE_SHOT, SEND, &channel, FIRST]);
    assert!(!one_shot.success, "{one_shot:?}");
    talk.front.down(false);
    let drained = helped.run(&[ONE_SHOT, RECV]);
    assert!(drained.success, "{drained:?}");
    helped.assert_none();
    talk.front.down(true);

    let through = helped.send_with(&talk, &timeout, SECOND);

    assert_same_output(&one_shot, &through);
    talk.front.down(false);
    helped.sent(&talk, THIRD);
    bob_reads(&mut talk, &[FIRST, SECOND, THIRD]);
}

/// Fails unless `through`, a run through the helper, exited as `one_shot` did and showed the same
/// on each output, IDs and what is in parentheses aside: an error's cause, which a network may word
/// otherwise from one run to the next.
#[track_caller]
fn assert_same_output(one_shot: &Finished, through: &Finished) {
    assert_eq!(
        through.success, one_shot.success,
        "not the same exit: {through:?}, one-shot {one_shot:?}"
    );
    for (what, one_shot_text, through_text) in [
        ("stdout", &one_shot.stdout, &through.stdout),
        ("stderr", &one_shot.stderr, &through.stderr),
    ] {
        assert_eq!(
            comparable(through_text),
            comparable(one_shot_text),
            "not the same {what}: {through:?}, one-shot {one_shot:?}"
        );
    }
}

/// `text` with each ID, 36 characters of hex digits and hyphens, as `<id>`, and each part in
/// parentheses as `(…)`.
fn comparable(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '('
            && let Some(close) = chars[at..].iter().position(|char| *char == ')')
        {
            out.push_str("(…)");
            at += close + 1;
            continue;
        }
        let id = chars.get(at..at + 36).is_some_and(|candidate| {
            candidate
                .iter()
                .enumerate()
                .all(|(index, char)| match index {
                    8 | 13 | 18 | 23 => *char == '-',
                    _ => char.is_ascii_hexdigit(),
                })
        });
        if id {
            out.push_str("<id>");
            at += 36;
        } else {
            out.push(chars[at]);
            at += 1;
        }
    }
    out
}
