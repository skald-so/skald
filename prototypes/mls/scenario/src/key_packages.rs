//! How a device keeps its KeyPackages, as `docs/protocol/membership.md`, "KeyPackages", says.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::wire::Lifetime;

const DAY: u64 = 24 * 60 * 60;

/// Below this many unused KeyPackages on the server, a device uploads more.
pub const LOW: usize = 10;
/// The unused KeyPackages a device has on the server after it uploads.
pub const FULL: usize = 20;
/// The longest lifetime of a KeyPackage.
pub const MAX_LIFETIME: u64 = 60 * DAY;
/// How long before it expires a device replaces a KeyPackage.
pub const RENEWAL: u64 = 7 * DAY;
/// How far into the past a lifetime starts, for receivers whose clocks are behind.
pub const SKEW: u64 = 60 * 60;

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs()
}

/// The lifetime of a KeyPackage made at `now`: the longest, starting [`SKEW`] before `now`.
pub fn lifetime(now: u64) -> Lifetime {
    let not_before = now - SKEW;
    Lifetime {
        not_before,
        not_after: not_before + MAX_LIFETIME,
    }
}

/// How many KeyPackages a device uploads when `unused` of its others remain on the server.
pub fn to_upload(unused: usize) -> usize {
    if unused < LOW { FULL - unused } else { 0 }
}

/// Whether a device replaces a KeyPackage with `lifetime` at `now`.
pub fn due(lifetime: &Lifetime, now: u64) -> bool {
    now + RENEWAL >= lifetime.not_after
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lifetime_is_60_days_from_an_hour_ago() {
        let now = 1_780_000_000;
        assert_eq!(
            lifetime(now),
            Lifetime {
                not_before: now - 3600,
                not_after: now - 3600 + 60 * 86400,
            }
        );
        assert!(lifetime(now).holds(now));
        // After this code was written, and not in the far future.
        assert!((1_770_000_000..4_000_000_000).contains(&super::now()));
    }

    #[test]
    fn uploads_up_to_20_below_10() {
        for (unused, want) in [(0, 20), (1, 19), (9, 11), (10, 0), (25, 0)] {
            assert_eq!(to_upload(unused), want, "{unused}");
        }
    }

    #[test]
    fn replaces_a_key_package_7_days_before_it_expires() {
        let lifetime = Lifetime {
            not_before: 0,
            not_after: 30 * 86400,
        };
        for (now, want) in [
            (0, false),
            (23 * 86400 - 1, false),
            (23 * 86400, true),
            (30 * 86400 + 1, true),
        ] {
            assert_eq!(due(&lifetime, now), want, "{now}");
        }
    }
}
