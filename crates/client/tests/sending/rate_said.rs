//! What a run or a tool says of a message the server refuses with `rate-limited`, its device over
//! the server's send rate (`docs/protocol/delivery.md`, "Send"): the problem, and, apart from it,
//! what it means for the message: that its device sends too many messages, too fast or over a
//! limit per minute.

/// The problem of a message over its device's send rate.
pub const RATE_LIMITED: &str = "rate-limited";

/// Words, any one of which says what the refusal means; the problem's name holds none of them.
const MEANS: [&str; 5] = ["minute", "too many", "too fast", "too often", "rate limit"];

/// Fails unless `shown` names the problem [`RATE_LIMITED`] and, apart from it, says what it means,
/// in any case; `what` names the case.
#[track_caller]
pub fn assert_says_rate_limited(shown: &str, what: &str) {
    assert!(
        shown.contains(RATE_LIMITED),
        "{what}: no {RATE_LIMITED}: {shown}"
    );
    let rest = shown.to_lowercase().replace(RATE_LIMITED, "");
    assert!(
        MEANS.iter().any(|words| rest.contains(words)),
        "{what}: it does not say what {RATE_LIMITED} means, with any of {MEANS:?}: {shown}"
    );
}
