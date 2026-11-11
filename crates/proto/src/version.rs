//! Protocol versions: `docs/protocol/README.md`, "Protocol version".

use std::num::NonZeroU64;

/// The protocol version this implementation speaks.
pub const PROTOCOL_VERSION: NonZeroU64 = NonZeroU64::new(1).unwrap();

/// The HTTPS request header that states the protocol version.
pub const VERSION_HEADER: &str = "Skald-Protocol-Version";

/// The response header that carries the server's current protocol version.
pub const LATEST_HEADER: &str = "Skald-Protocol-Latest";

/// The prefix of the WebSocket subprotocol `skald.<n>` that states protocol version `n`.
pub const SUBPROTOCOL_PREFIX: &str = "skald.";

/// Why the server refuses the protocol version a request states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VersionProblem {
    /// The version is below the oldest one the server accepts: the client must update.
    TooOld,
    /// The version is above the server's current one.
    TooNew,
    /// The request states no version, or not a positive integer.
    Invalid,
}

impl VersionProblem {
    /// The problem's name, such as `protocol-too-old`.
    pub fn name(self) -> &'static str {
        match self {
            Self::TooOld => "protocol-too-old",
            Self::TooNew => "protocol-too-new",
            Self::Invalid => "protocol-version-invalid",
        }
    }

    /// The `type` of the problem details object, such as
    /// `https://skald.so/problems/protocol-too-old`.
    pub fn type_uri(self) -> String {
        format!("https://skald.so/problems/{}", self.name())
    }
}

/// Parses a stated version: decimal digits without a leading zero. A version above `u64::MAX`
/// becomes `u64::MAX`: still above every version. `None` for any other text.
pub fn parse_version(text: &str) -> Option<NonZeroU64> {
    let first = *text.as_bytes().first()?;
    if first == b'0' || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // Only digits without a leading zero: parsing fails only on overflow.
    Some(text.parse().unwrap_or(NonZeroU64::MAX))
}

/// The WebSocket subprotocol that states `version`: `skald.<n>`.
pub fn subprotocol(version: NonZeroU64) -> String {
    format!("{SUBPROTOCOL_PREFIX}{version}")
}
