//! The person's lines of recorded prompts in a run's output.

/// The word of the person's line of a recorded prompt.
pub const PERMISSION_WORD: &str = "permission";

/// The lines of `shown`, a run's output, that hold the word [`PERMISSION_WORD`], in any case.
pub fn permission_lines(shown: &str) -> Vec<String> {
    (shown.lines())
        .filter(|line| line.to_lowercase().contains(PERMISSION_WORD))
        .map(str::to_owned)
        .collect()
}
