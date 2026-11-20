//! What every agent's executor takes and gives (`docs/protocol/queues.md`, "Running a task"): a
//! new agent session with no person, in a given folder, at a mode.

use std::fmt;
use std::str::FromStr;

/// What an executor session may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Reads its folder: no file written, no command run, no network, nothing read outside it.
    ReadOnly,
    /// Every tool, without prompts.
    Full,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Full => "full",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A mode other than `read-only` and `full`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownMode(pub String);

impl fmt::Display for UnknownMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown executor mode {:?}: only read-only and full",
            self.0
        )
    }
}

impl std::error::Error for UnknownMode {}

impl FromStr for Mode {
    type Err = UnknownMode;

    fn from_str(mode: &str) -> Result<Self, Self::Err> {
        match mode {
            "read-only" => Ok(Self::ReadOnly),
            "full" => Ok(Self::Full),
            _ => Err(UnknownMode(mode.to_owned())),
        }
    }
}

/// How an executor session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The session's final answer; when it did not end well, what it said of why.
    pub answer: String,
    /// Whether it ended well.
    pub finished: bool,
}

/// Why an executor did not run a session: a refusal, or a failure to start the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}
