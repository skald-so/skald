//! What `skald send` says of a refusal over a cloud limit (`docs/protocol/delivery.md`, "Send"):
//! for `too-large`, that the message is `too large`; for `quota-exceeded`, that it is over a
//! `limit` of its space. Like any refusal, the envelope is not sent again. The front gives the
//! refusals in place of the server's `stored`; the server's own limits are tested in
//! `crates/server/tests/cloud_limits`.

use super::{
    Answers, FIRST, Finished, Pair, SECOND, Sent, THIRD, msg_id, only_envelope_to, sent_after,
};

/// Each refusal over a limit, by its problem, and what the run says of it, in lowercase.
const SAID: [(&str, &str); 2] = [("too-large", "too large"), ("quota-exceeded", "limit")];

/// Runs Ada's `skald send` of `text`, the front refusing its envelopes with `problem`; gives what
/// the run showed and the `send` frames the front saw.
fn refused_as(pair: &Pair, problem: &str, text: &str) -> (Finished, Vec<Sent>) {
    let before = pair.front.sent().len();
    pair.front.answer(Answers::Refuse(problem.into()));
    let finished = pair.send(text);
    (finished, sent_after(&pair.front, before))
}

/// A message refused as `too-large`, then one refused as `quota-exceeded`: each run names the
/// message and says what the refusal means for it; neither is sent again by the next run.
#[test]
fn refusals_over_a_limit_say_what_they_mean_for_the_message() {
    let pair = Pair::start(
        [4_122_101_001, 4_122_101_002],
        ["ada-over-limit-standin", "bob-under-limit-standin"],
    );
    let mut refused = Vec::new();
    for ((problem, said), text) in SAID.into_iter().zip([FIRST, SECOND]) {
        let (finished, sent) = refused_as(&pair, problem, text);

        let id = msg_id(&only_envelope_to(&sent, pair.bob.device));
        let shown = finished.shown();
        assert!(
            shown.contains(&id.to_string()),
            "no message ID: {finished:?}"
        );
        assert!(
            shown.to_lowercase().contains(said),
            "{problem}: it does not say {said:?}: {finished:?}"
        );
        assert!(
            sent.iter().all(|sent| !refused.contains(&msg_id(sent))),
            "a refused envelope is sent again"
        );
        refused.push(id);
    }
    pair.front.answer(Answers::Forward);
    let from = pair.front.sent().len();

    let finished = pair.send(THIRD);

    assert!(finished.success, "{finished:?}");
    let sent = sent_after(&pair.front, from);
    assert!(
        sent.iter().all(|sent| !refused.contains(&msg_id(sent))),
        "a refused envelope is sent again"
    );
    for (_, said) in SAID {
        assert!(
            !finished.shown().to_lowercase().contains(said),
            "a stored message is said to be {said:?}: {finished:?}"
        );
    }
}
