//! Trust levels: `docs/protocol/membership.md`, "Trust levels".

use std::fmt;
use std::str::FromStr;

/// The level a recipient device gives a sending device. Ordered: `Own` is above `Peer`, which is
/// above `Service`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrustLevel {
    /// A `service` device.
    Service,
    /// A `client` or `browser` device of another account the recipient's space roster lists.
    Peer,
    /// A `client` or `browser` device of the recipient's account.
    Own,
}

impl TrustLevel {
    /// Its name in the spec: `own`, `peer` or `service`.
    pub fn name(self) -> &'static str {
        match self {
            TrustLevel::Own => "own",
            TrustLevel::Peer => "peer",
            TrustLevel::Service => "service",
        }
    }
}

impl fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A text that names no trust level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustLevelError;

impl fmt::Display for TrustLevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not own, peer or service")
    }
}

impl std::error::Error for TrustLevelError {}

impl FromStr for TrustLevel {
    type Err = TrustLevelError;

    /// A level by its [`TrustLevel::name`].
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        [TrustLevel::Own, TrustLevel::Peer, TrustLevel::Service]
            .into_iter()
            .find(|level| level.name() == text)
            .ok_or(TrustLevelError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each level reads back from its name; no other text is a level.
    #[test]
    fn level_reads_back_from_its_name_only() {
        for level in [TrustLevel::Own, TrustLevel::Peer, TrustLevel::Service] {
            assert_eq!(level.to_string().parse(), Ok(level));
        }
        assert_eq!(TrustLevel::Own.to_string(), "own");
        assert_eq!(TrustLevel::Peer.to_string(), "peer");
        assert_eq!(TrustLevel::Service.to_string(), "service");
        for text in ["", "Own", "user", "own "] {
            assert_eq!(text.parse::<TrustLevel>(), Err(TrustLevelError));
        }
    }
}
