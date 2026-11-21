//! `hops` and tasks (`docs/protocol/README.md`, "Content"): a claimed task counts as a message
//! handed to the agent for its next send, and a result goes out even over the limit on replies
//! without a human, as nothing answers a result.

use skald_proto::{Body, Cbor as _, Content, Outcome};

use crate::claiming::{COMPLETE, Claims, Craft};
use crate::senders::LAPTOP_AGENT;
use crate::talk::a2a;

/// What the tests write, say and answer; no text is anywhere else.
const RELAYED: &str = "hops test: a task three hops from a person asks codex to summarise";
const SAID_AFTER: &str = "hops test: codex tells the channel what the summary found";
const AT_THE_LIMIT: &str = "hops test: a task ten hops from a person asks codex to check";
const LIMIT_DONE: &str = "hops test: codex checked it all the same";

/// Ada's first device writes a task with `hops` 3; codex claims it, then says something in the
/// channel with `skald send` on the second machine. Bob's device reads that message with `hops` 4.
#[test]
fn claimed_task_counts_for_the_hops_of_the_agent_s_next_send() {
    let mut claims = Claims::new(
        [4_267_051_001, 4_267_051_002],
        ["ada-relays-a-summary", "bob-counts-the-hops"],
    );
    let person = claims.person();
    let mut craft = Craft::new(&person, claims.queue, RELAYED);
    craft.hops = 3;
    claims.ada_writes(&craft);
    claims.codex_claimed(&[]);

    claims.senders.laptop_agent_says(LAPTOP_AGENT, SAID_AFTER);

    let talk = &mut claims.senders.agents.talk;
    talk.played[0].take_in(&talk.world.cloud);
    let read: Vec<Content> = (talk.played[0].read_in(talk.channel).into_iter())
        .filter_map(|(_, decrypted)| Content::from_cbor(&decrypted.plaintext).ok())
        .filter(|content| {
            matches!(content.body, Some(Body::A2a(_))) && a2a(content).1 == [SAID_AFTER]
        })
        .collect();
    let [said] = read.as_slice() else {
        panic!("not one message of codex read: {read:?}");
    };
    assert_eq!(said.hops, 4, "{said:?}");
}

/// Ada's first device writes a task with `hops` 10, the limit; codex claims and completes it.
/// Bob's device reads its result, with `hops` 11.
#[test]
fn result_goes_out_over_the_limit_on_hops() {
    let mut claims = Claims::new(
        [4_267_052_001, 4_267_052_002],
        ["ada-sends-at-the-limit", "bob-reads-past-the-limit"],
    );
    let person = claims.person();
    let mut craft = Craft::new(&person, claims.queue, AT_THE_LIMIT);
    craft.hops = 10;
    let task = claims.ada_writes(&craft).task;
    claims.codex_claimed(&[]);
    let held = claims.read(task).held().clone();

    claims.codex_ran(COMPLETE, &[&task.to_string(), LIMIT_DONE]);

    claims.assert_codex_result((task, &held), (&person, 11), (Outcome::Done, LIMIT_DONE));
}
