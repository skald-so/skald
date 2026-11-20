//! What `skald send` says of a message the server refuses over its device's send rate: the
//! message's ID, the problem `rate-limited` and what it means. Like any refusal, the envelope is
//! not sent again. The front gives the refusal in place of the server's `stored`; the server's own
//! rate is tested in `crates/server/tests/send_rate`.

use super::rate_said::{RATE_LIMITED, assert_says_rate_limited};
use super::{Answers, Talk, msg_id, one_message_sent, sent_after};

/// Ada's messages: the one the front refuses, and the next one, which it does not.
const REFUSED: &str = "send rate test: the third status report in a row";
const NEXT: &str = "send rate test: a status report once the rate allows";

/// Ada's `skald send`, the front refusing its envelope with `rate-limited`, fails, names the
/// message and says what the refusal means. Her next run, the front forwarding, does not send the
/// refused envelope again, and says nothing of the rate.
#[test]
fn a_message_over_the_send_rate_says_what_the_refusal_means() {
    let talk = Talk::new(
        &[4_216_061_001, 4_216_061_002],
        &["ada-over-send-rate-standin", "bob-under-send-rate-standin"],
        false,
    );
    talk.front.answer(Answers::Refuse(RATE_LIMITED.to_owned()));
    let from = talk.front.sent().len();

    let refused = talk.send(REFUSED);

    assert!(!refused.success, "{refused:?}");
    let id = one_message_sent(&talk.front, talk.channel, from);
    let shown = refused.shown();
    assert!(
        shown.contains(&id.to_string()),
        "no message ID: {refused:?}"
    );
    assert_says_rate_limited(&shown, "skald send");

    talk.front.answer(Answers::Forward);
    let from = talk.front.sent().len();
    let next = talk.send(NEXT);

    assert!(next.success, "{next:?}");
    let sent = sent_after(&talk.front, from);
    assert!(
        sent.iter().all(|sent| msg_id(sent) != id),
        "the refused envelope is sent again"
    );
    assert!(
        !next.shown().contains(RATE_LIMITED),
        "a stored message is said to be {RATE_LIMITED}: {next:?}"
    );
}
