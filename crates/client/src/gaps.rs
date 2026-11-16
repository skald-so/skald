//! Gap detection (`docs/protocol/delivery.md`, "Gap detection"): per channel, sending device and
//! scheme, the highest `counter` the device accepted and the counters it is missing. The baseline:
//! 0 under `hpke`; under `mls`, the first message from the sending device since the device last
//! joined the channel. A counter above the highest is accepted, the counters between becoming
//! missing; so is a missing one; any other is a duplicate or a replay.

use std::num::NonZeroU64;

use skald_proto::{ChannelId, DeviceId, Scheme};

/// The most runs of missing counters kept per sending device, channel and scheme: past it, the
/// lowest run is forgotten, and a late message of it is dropped as a duplicate.
pub const MAX_RUNS: usize = 1024;

/// Whose counters: a sending device's in a channel under a scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub channel: ChannelId,
    pub device: DeviceId,
    pub scheme: Scheme,
}

/// What the device keeps of a [`Source`]'s counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counters {
    /// The highest counter accepted.
    pub highest: u64,
    /// The missing counters, as runs `first..=last`, ascending and apart, all below the highest.
    pub missing: Vec<(u64, u64)>,
}

/// What accepting a counter gives.
#[derive(Debug, PartialEq, Eq)]
pub struct Accepted {
    /// The counters with it accepted.
    pub counters: Counters,
    /// The counters it made missing, as a run `first..=last`: those between the highest and it.
    pub gap: Option<(u64, u64)>,
}

impl Counters {
    /// The highest, then `first` and `last` of each run, each as 8 bytes, big-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let runs = self.missing.iter().flat_map(|&(first, last)| [first, last]);
        std::iter::once(self.highest)
            .chain(runs)
            .flat_map(u64::to_be_bytes)
            .collect()
    }

    /// The counters `bytes` encode as [`Counters::to_bytes`] does; none if they do not.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (chunks, rest) = bytes.as_chunks::<8>();
        if !rest.is_empty() || chunks.len() % 2 == 0 {
            return None;
        }
        let mut numbers = chunks.iter().map(|chunk| u64::from_be_bytes(*chunk));
        let highest = numbers.next()?;
        let mut missing = Vec::new();
        while let (Some(first), Some(last)) = (numbers.next(), numbers.next()) {
            missing.push((first, last));
        }
        Some(Counters { highest, missing })
    }
}

/// What accepting `counter` from a source under `scheme` gives, `previous` its counters, if the
/// device keeps any since its baseline; none for a duplicate or a replay.
pub fn accept(scheme: Scheme, previous: Option<Counters>, counter: NonZeroU64) -> Option<Accepted> {
    let counter = counter.get();
    let Some(previous) = previous.or_else(|| (scheme == Scheme::Hpke).then(Counters::default))
    else {
        return Some(Accepted {
            counters: Counters {
                highest: counter,
                missing: Vec::new(),
            },
            gap: None,
        });
    };
    let mut missing = previous.missing;
    let gap = if counter > previous.highest {
        let gap = (counter - previous.highest > 1).then(|| (previous.highest + 1, counter - 1));
        missing.extend(gap);
        gap
    } else {
        let at = missing
            .iter()
            .position(|&(first, last)| (first..=last).contains(&counter))?;
        let (first, last) = missing[at];
        let before = (counter > first).then(|| (first, counter - 1));
        let after = (counter < last).then(|| (counter + 1, last));
        missing.splice(at..=at, before.into_iter().chain(after));
        None
    };
    let excess = missing.len().saturating_sub(MAX_RUNS);
    missing.drain(..excess);
    Some(Accepted {
        counters: Counters {
            highest: previous.highest.max(counter),
            missing,
        },
        gap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter(counter: u64) -> NonZeroU64 {
        NonZeroU64::new(counter).unwrap()
    }

    fn counters(highest: u64, missing: &[(u64, u64)]) -> Counters {
        Counters {
            highest,
            missing: missing.to_vec(),
        }
    }

    /// Accepts `counter` after `previous` under `scheme`; fails if it is dropped.
    #[track_caller]
    fn accepted(scheme: Scheme, previous: Option<Counters>, at: u64) -> Accepted {
        accept(scheme, previous, counter(at)).expect("accepted")
    }

    /// Under `hpke`, counting starts at 0: a first counter above 1 makes those below it missing.
    #[test]
    fn hpke_counts_from_zero() {
        assert_eq!(
            accepted(Scheme::Hpke, None, 1),
            Accepted {
                counters: counters(1, &[]),
                gap: None,
            }
        );
        assert_eq!(
            accepted(Scheme::Hpke, None, 4),
            Accepted {
                counters: counters(4, &[(1, 3)]),
                gap: Some((1, 3)),
            }
        );
    }

    /// Under `mls`, the first counter sets the highest, with nothing missing.
    #[test]
    fn mls_counts_from_the_first_counter() {
        assert_eq!(
            accepted(Scheme::Mls, None, 7),
            Accepted {
                counters: counters(7, &[]),
                gap: None,
            }
        );
        assert_eq!(
            accepted(Scheme::Mls, Some(counters(7, &[])), 9).gap,
            Some((8, 8))
        );
    }

    /// The next counter makes nothing missing; one further on, those between it and the highest.
    #[test]
    fn counter_above_the_highest_makes_those_between_missing() {
        for scheme in [Scheme::Mls, Scheme::Hpke] {
            assert_eq!(
                accepted(scheme, Some(counters(3, &[(1, 1)])), 4),
                Accepted {
                    counters: counters(4, &[(1, 1)]),
                    gap: None,
                }
            );
            assert_eq!(
                accepted(scheme, Some(counters(3, &[(1, 1)])), 7),
                Accepted {
                    counters: counters(7, &[(1, 1), (4, 6)]),
                    gap: Some((4, 6)),
                }
            );
        }
    }

    /// A missing counter is accepted and no longer missing: at either end of its run or inside it,
    /// which splits the run.
    #[test]
    fn missing_counter_is_accepted_once() {
        let previous = || Some(counters(9, &[(2, 2), (4, 7)]));
        for (at, missing) in [
            (2, &[(4, 7)][..]),
            (4, &[(2, 2), (5, 7)]),
            (7, &[(2, 2), (4, 6)]),
            (5, &[(2, 2), (4, 4), (6, 7)]),
        ] {
            let accepted = accepted(Scheme::Mls, previous(), at);
            assert_eq!(
                accepted,
                Accepted {
                    counters: counters(9, missing),
                    gap: None,
                },
                "{at}"
            );
            assert_eq!(
                accept(Scheme::Mls, Some(accepted.counters), counter(at)),
                None,
                "{at} again"
            );
        }
    }

    /// The highest, a counter below it that is not missing, and one accepted already are dropped.
    #[test]
    fn counter_neither_above_nor_missing_is_dropped() {
        for scheme in [Scheme::Mls, Scheme::Hpke] {
            for at in [1, 3, 8, 9] {
                assert_eq!(
                    accept(scheme, Some(counters(9, &[(2, 2), (4, 7)])), counter(at)),
                    None,
                    "{at}"
                );
            }
        }
    }

    /// Past the most runs kept, the lowest is forgotten: its counters are dropped from then on.
    #[test]
    fn lowest_run_is_forgotten_past_the_most_kept() {
        let runs: Vec<(u64, u64)> = (1..=MAX_RUNS as u64)
            .map(|run| (2 * run, 2 * run))
            .collect();
        let highest = 2 * MAX_RUNS as u64 + 1;

        let accepted = accepted(Scheme::Hpke, Some(counters(highest, &runs)), highest + 2);

        assert_eq!(accepted.counters.missing.len(), MAX_RUNS);
        assert_eq!(accepted.counters.missing[0], runs[1]);
        assert_eq!(
            accepted.counters.missing.last(),
            Some(&(highest + 1, highest + 1))
        );
        assert_eq!(
            accept(Scheme::Hpke, Some(accepted.counters), counter(2)),
            None
        );
    }

    /// The counters, the largest included, come back from their bytes; bytes of another length do
    /// not decode.
    #[test]
    fn counters_come_back_from_their_bytes() {
        for kept in [
            counters(0, &[]),
            counters(u64::MAX, &[(1, 3), (5, u64::MAX - 1)]),
        ] {
            assert_eq!(Counters::from_bytes(&kept.to_bytes()), Some(kept));
        }
        for length in [0, 7, 9, 16, 31] {
            assert_eq!(Counters::from_bytes(&vec![0; length]), None, "{length}");
        }
    }
}
