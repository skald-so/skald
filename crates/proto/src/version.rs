//! Protocol versions: `docs/protocol/README.md`, "Protocol version".

use std::fmt;
use std::num::NonZeroU64;
use std::str::FromStr;

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

/// The protocol versions from `oldest` to `latest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionRange {
    oldest: NonZeroU64,
    latest: NonZeroU64,
}

impl VersionRange {
    /// The oldest version in the range.
    pub fn oldest(self) -> NonZeroU64 {
        self.oldest
    }

    /// The newest version in the range.
    pub fn latest(self) -> NonZeroU64 {
        self.latest
    }

    /// Whether `version` is in the range.
    pub fn contains(self, version: u64) -> bool {
        (self.oldest.get()..=self.latest.get()).contains(&version)
    }

    /// The versions in the range, ascending.
    pub fn list(self) -> Vec<u64> {
        (self.oldest.get()..=self.latest.get()).collect()
    }
}

/// [`PROTOCOL_VERSION`] and the version before it; version 1 alone while it is the only one.
impl Default for VersionRange {
    fn default() -> Self {
        VersionRange {
            oldest: NonZeroU64::new(PROTOCOL_VERSION.get().saturating_sub(1))
                .unwrap_or(PROTOCOL_VERSION),
            latest: PROTOCOL_VERSION,
        }
    }
}

/// `<n>`, or `<oldest>-<latest>`, such as `2-3`; each version as [`parse_version`] reads it.
impl FromStr for VersionRange {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let version = |text: &str| {
            parse_version(text)
                .ok_or_else(|| format!("{text:?} is not a protocol version, an integer from 1"))
        };
        let (oldest, latest) = match text.split_once('-') {
            Some((oldest, latest)) => (version(oldest)?, version(latest)?),
            None => (version(text)?, version(text)?),
        };
        if oldest > latest {
            return Err(format!(
                "the oldest version, {oldest}, is above the latest, {latest}"
            ));
        }
        Ok(VersionRange { oldest, latest })
    }
}

/// As [`describe_versions`] describes the range's versions.
impl fmt::Display for VersionRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&describe_versions(&self.list()))
    }
}

/// `protocol version 3`, `protocol versions 2 and 3`, `protocol versions 1, 2 and 3`;
/// `no protocol version` for none.
pub fn describe_versions(versions: &[u64]) -> String {
    let text: Vec<String> = versions.iter().map(u64::to_string).collect();
    match text.as_slice() {
        [] => "no protocol version".to_owned(),
        [only] => format!("protocol version {only}"),
        [rest @ .., last] => format!("protocol versions {} and {last}", rest.join(", ")),
    }
}
