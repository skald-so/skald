//! A result counts only in the channel of its task: check 2 of "Results".

use crate::returns::{DONE, Returns};

/// What the test enqueues and answers; no text is anywhere else.
const IN_DEFAULT: &str = "channel test: claude asks in acme's default channel for the roadmap";
const ROADMAP: &str = "channel test: the roadmap is in the wiki";

/// As claude, `skald enqueue --channel <acme's default channel>` puts a task there; Bob's device
/// claims and completes it in that channel's queue `default`. Its `done` result, sent in the
/// talk's channel, is not handed over to claude, and the person's next run warns of it; the same
/// result sent in the default channel is handed over.
#[test]
fn result_for_a_task_of_another_channel_is_dropped() {
    let mut returns = Returns::new(
        [5_873_021_001, 5_873_021_002],
        [
            "ada-asks-in-the-default-channel",
            "bob-answers-in-the-wrong-one",
        ],
    );
    let default = returns.default_channel();
    let task = returns.claude_enqueues_in(default, &[], IN_DEFAULT);
    let answer = returns.bob_runs_in(default, ROADMAP);
    assert_eq!(answer.task, task, "Bob's claim is not of claude's task");

    returns.bob_answers_in(returns.channel(), &answer);

    returns.assert_dropped(task, "a result in another channel");
    returns.bob_answers_in(default, &answer);
    returns.assert_claude_gets_bob_s(task, DONE);
}
