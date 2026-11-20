//! Without the setting, a device stores any number of messages; its environment variable sets it.

use crate::rate::{
    RATE_ENV, RATE_LIMITED, WINDOW_ENV, answer_to, message_to, stores_messages, with_alice,
};
use crate::wire::{Answer, Socket, envelope, envelopes_of};

/// More than the cloud's 120 messages per minute (`docs/dev/architecture.md`, "Cloud limits").
const MANY: usize = 121;

/// Without the setting, alice's laptop stores [`MANY`] messages at once. Started again on the same
/// database with the setting at 3 and a window of a minute, by their environment variables, the
/// server stores at most 3 more of her messages within the minute, then refuses each next one with
/// `rate-limited`.
#[test]
fn without_the_setting_a_device_stores_any_number_and_with_it_no_more_than_the_rate() {
    let (mut fixture, mut alice) = with_alice(3_216_060_501, &[]);
    let bob = fixture.team.bob.laptop.id;
    let stored = stores_messages(&fixture, &mut alice, MANY, &[bob]);
    assert_eq!(envelopes_of(fixture.db(), bob), stored, "bob's mailbox");
    drop(alice);

    let server = (fixture.team.flow.stand).restart(&[], &[(RATE_ENV, "3"), (WINDOW_ENV, "60")]);
    let t = &fixture.team;
    let mut again = Socket::to(&server, &t.flow, &t.alice, &t.alice.laptop);
    let answers: Vec<Answer> = (0..5)
        .map(|_| {
            let header = &message_to(&fixture, &[bob])[0];
            answer_to(&mut again, header, &envelope(header))
        })
        .collect();
    let taken = (answers.iter())
        .position(|answer| !matches!(answer, Answer::Stored(_)))
        .unwrap_or(answers.len());
    assert!(taken <= 3, "stored {taken} past a rate of 3: {answers:?}");
    let refused =
        |answer: &Answer| matches!(answer, Answer::Refused(_, problem) if problem == RATE_LIMITED);
    assert!(
        answers[taken..].iter().all(refused),
        "{RATE_LIMITED} past the rate, not {answers:?}"
    );
}
