//! What the tests share beside the scene of the acceptance tests of task results
//! (`task_results_acceptance/returns.rs`): the lines and results Bob's device writes in the talk's
//! channel, the results and `read` receipts it reads, and what a run shows of a result and of a
//! dropped message.

use std::num::NonZeroU64;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::{
    Body, Cbor as _, ClaimId, Content, DeviceId, MessageId, Outcome, ReceiptStatus, TaskId,
};

use crate::cloud::WAIT;
use crate::lines::{json_objects, refers_to};
use crate::machine::Finished;
use crate::queuing::{references, refers_to_task};
use crate::returns::{Answer, Returns};
use crate::states::Held;
use crate::talk::Line;

/// How a result's line names its outcome.
pub fn outcome_mark(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "(task done)",
        Outcome::Failed => "(task failed)",
    }
}

/// The word of a dropped message's warning.
pub const DROPPED: &str = "dropped";

/// A claim as a result names it: its `claim_id` and generation.
pub type Named = (ClaimId, u64);

impl Returns {
    /// Bob's device writes, in the talk's channel as its next line there, the content `content`
    /// makes of the envelope's `msg_id` and that line's counter, as [`Returns::bob_sends`] does.
    /// Gives its `msg_id`.
    #[track_caller]
    pub fn bob_writes(
        &mut self,
        content: impl FnOnce(MessageId, NonZeroU64) -> Content,
    ) -> MessageId {
        let channel = self.channel();
        let header = self.header_in(channel);
        let counter = NonZeroU64::new(self.next_counter(channel)).expect("a counter");
        let content = content(header.msg_id, counter);
        self.bob_sends(header, &content)
    }

    /// Bob's device writes a result of `task` from Bob's agent to `to`, naming the claim `named`,
    /// with `outcome` and `text`, as [`Returns::bob_answers`] does. Gives its `msg_id`.
    #[track_caller]
    pub fn bob_results(
        &mut self,
        (task, named): (TaskId, Named),
        to: &str,
        (outcome, text): (Outcome, &str),
    ) -> MessageId {
        let (claim_id, generation) = named;
        let answer = Answer {
            task,
            claim_id,
            generation,
            outcome,
            from: self.bob_agent(),
            to: to.to_owned(),
            text: text.to_owned(),
        };
        self.bob_answers(&answer)
    }

    /// Bob's device says `text` to claude from Bob's agent, as [`Returns::bob_writes`] does.
    /// Gives its `msg_id`.
    #[track_caller]
    pub fn bob_says_to_claude(&mut self, text: &str) -> MessageId {
        let (from, to) = (self.bob_agent(), self.claims.claude());
        self.bob_writes(|msg_id, counter| {
            let mut line = Line::new(&from, counter.get(), text);
            line.to = Some(to);
            line.content(msg_id)
        })
    }

    /// As [`Returns::bob_results`], to claude, naming `held`.
    #[track_caller]
    pub fn bob_results_to_claude(
        &mut self,
        held: &Held,
        (outcome, text): (Outcome, &str),
    ) -> MessageId {
        let claude = self.claims.claude();
        self.bob_results((held.task, named(held)), &claude, (outcome, text))
    }

    /// Waits until Bob's device read `count` results of `task` in the channel, taking in again
    /// every half second for up to [`WAIT`]; fails if they do not come.
    #[track_caller]
    pub fn bob_reads_results(&mut self, task: TaskId, count: usize) {
        let deadline = Instant::now() + WAIT;
        while self.claims.results(task).len() < count {
            assert!(
                Instant::now() < deadline,
                "not {count} results of {task} read"
            );
            thread::sleep(Duration::from_millis(500));
        }
    }

    /// The `read` receipts of `msg_id` Bob's device reads in the channel, each with its sending
    /// device and content, once it reads one; takes in again every half second for up to
    /// [`WAIT`]. Fails if none comes.
    #[track_caller]
    pub fn read_receipts_of(&mut self, msg_id: MessageId) -> Vec<(DeviceId, Content)> {
        let deadline = Instant::now() + WAIT;
        loop {
            let talk = &mut self.claims.senders.agents.talk;
            talk.played[0].take_in(&talk.world.cloud);
            let read: Vec<_> = (talk.played[0].read_in(talk.channel).into_iter())
                .filter_map(|(_, decrypted)| {
                    let content = Content::from_cbor(&decrypted.plaintext).ok()?;
                    let Some(Body::Receipt(receipt)) = &content.body else {
                        return None;
                    };
                    (receipt.status == ReceiptStatus::Read && receipt.messages.contains(&msg_id))
                        .then_some((decrypted.sender, content))
                })
                .collect();
            if !read.is_empty() {
                return read;
            }
            assert!(Instant::now() < deadline, "no read of {msg_id}");
            thread::sleep(Duration::from_millis(500));
        }
    }
}

/// The claim `held` names, as a result names it.
#[track_caller]
pub fn named(held: &Held) -> Named {
    let claim_id = (held.claim_id.parse()).unwrap_or_else(|_| panic!("no claim ID in {held:?}"));
    (claim_id, held.generation)
}

/// Fails unless `shown`, an agent's output, has one line holding `text`, and it names `task` by
/// its reference and `outcome` as [`outcome_mark`] does, before its text. Gives the line.
#[track_caller]
pub fn assert_result_line(shown: &str, task: TaskId, (outcome, text): (Outcome, &str)) -> String {
    let lines: Vec<&str> = shown.lines().filter(|line| line.contains(text)).collect();
    let [line] = lines.as_slice() else {
        panic!("not one line holds {text:?}: {shown}");
    };
    let (head, _) = line.split_once(text).expect("the text");
    assert!(
        head.ends_with(&format!(" {}: ", outcome_mark(outcome))),
        "not a {outcome:?} result line: {line:?}"
    );
    assert!(
        refers_to_task(head, task),
        "not the task's reference: {line:?}"
    );
    (*line).to_owned()
}

/// Fails unless `finished` says, on its standard error, that it dropped the message `msg_id`,
/// naming it by its `msg_id` or reference, and shows nothing of `text`.
#[track_caller]
pub fn assert_dropped(finished: &Finished, msg_id: MessageId, text: &str) {
    let id = msg_id.to_string();
    let names = |line: &str| {
        line.contains(&id) || (references(line).iter()).any(|short| refers_to(short, msg_id))
    };
    let warned =
        (finished.stderr.lines()).any(|line| line.to_lowercase().contains(DROPPED) && names(line));
    assert!(warned, "not said that {msg_id} was dropped: {finished:?}");
    assert!(
        !finished.shown().contains(text),
        "{text:?} shown: {finished:?}"
    );
}

/// The JSON objects of the type `result` of `stdout`, the output of `skald recv --json`.
pub fn result_objects(stdout: &str) -> Vec<Value> {
    (json_objects(stdout).into_iter())
        .filter(|object| object["type"] == "result")
        .collect()
}
