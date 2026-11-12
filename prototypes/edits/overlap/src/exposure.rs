//! What the server learns from the fingerprints of the measured merges without the space key, what
//! a holder of the key recovers from them, and what a change of the key costs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::dictionary::{self, Dictionary, Recovered};
use crate::fingerprint::Keying;
use crate::noise::table;
use crate::replay::{self, Replay, Repository, percentile};
use crate::{Fingerprint, Result, Scope};

/// A machine as the server sees it: the fingerprints it publishes, when it publishes, and when it
/// stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    pub fingerprints: BTreeSet<Fingerprint>,
    /// Its commits, each with its author date in seconds since the Unix epoch: it publishes at
    /// least once at each.
    pub commits: Vec<(String, i64)>,
    /// When its changes are merged, in seconds since the Unix epoch: it publishes nothing after.
    pub until: i64,
}

impl Machine {
    /// When it first publishes: its first commit.
    fn since(&self) -> i64 {
        let dates = self.commits.iter().map(|(_, date)| *date);
        dates.min().unwrap_or(self.until)
    }
}

/// The server's view of a repository's replayed merges: its machines, and for each merge the
/// machines of its sides, none for a side without fingerprints.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct View {
    pub machines: Vec<Machine>,
    pub merges: Vec<[Option<usize>; 2]>,
}

impl View {
    /// The view of `replays`: a machine for each side with fingerprints, one for the sides of
    /// several merges with the same parent and merge base, until the latest of those merges.
    pub fn of(replays: &[Replay]) -> Self {
        let mut view = Self::default();
        let mut machines = BTreeMap::<(&str, &str), usize>::new();
        for replay in replays {
            let mut sides = [None, None];
            for ((side, parent), machine) in
                replay.sides.iter().zip(&replay.parents).zip(&mut sides)
            {
                let fingerprints = &side.edits.publication.fingerprints;
                if fingerprints.is_empty() {
                    continue;
                }
                let next = view.machines.len();
                let i = *machines.entry((&replay.base, parent)).or_insert(next);
                if i == next {
                    view.machines.push(Machine {
                        fingerprints: fingerprints.clone(),
                        commits: side.commits.clone(),
                        until: replay.merged_at,
                    });
                }
                let until = &mut view.machines[i].until;
                *until = (*until).max(replay.merged_at);
                *machine = Some(i);
            }
            view.merges.push(sides);
        }
        view
    }

    /// For each merge whose two machines share fingerprints, how many.
    pub fn shared(&self) -> Vec<usize> {
        self.merges
            .iter()
            .filter_map(|sides| match sides {
                [Some(a), Some(b)] => {
                    let [a, b] = [*a, *b].map(|i| &self.machines[i].fingerprints);
                    Some(a.intersection(b).count())
                }
                _ => None,
            })
            .filter(|&shared| shared > 0)
            .collect()
    }

    /// The groups of machines that fingerprints they share link, largest first: two machines are in
    /// one group when a chain of machines, each sharing a fingerprint with the next, joins them.
    pub fn groups(&self) -> Vec<usize> {
        // Each machine links to another of its group, or to itself at the group's root.
        let mut group: Vec<usize> = (0..self.machines.len()).collect();
        fn root(group: &[usize], i: usize) -> usize {
            if group[i] == i {
                i
            } else {
                root(group, group[i])
            }
        }
        let mut first = BTreeMap::<Fingerprint, usize>::new();
        for (i, machine) in self.machines.iter().enumerate() {
            // The machines that first published one of its fingerprints, each once.
            let others: BTreeSet<usize> = (machine.fingerprints.iter())
                .map(|fingerprint| *first.entry(*fingerprint).or_insert(i))
                .collect();
            for other in others {
                let (a, b) = (root(&group, i), root(&group, other));
                group[a] = b;
            }
        }
        let mut sizes = BTreeMap::<usize, usize>::new();
        for i in 0..group.len() {
            *sizes.entry(root(&group, i)).or_default() += 1;
        }
        let mut sizes: Vec<usize> = sizes.into_values().collect();
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        sizes
    }

    /// How many machines publish each fingerprint.
    pub fn recurrence(&self) -> BTreeMap<Fingerprint, usize> {
        let mut machines = BTreeMap::new();
        for fingerprint in self.machines.iter().flat_map(|m| &m.fingerprints) {
            *machines.entry(*fingerprint).or_default() += 1;
        }
        machines
    }

    /// The hour of the day, UTC, that starts the [`HOURS`] hours with the most commits of the
    /// machines, each commit once, and how many commits they hold; the earliest such hour.
    pub fn busiest_hours(&self) -> (usize, usize) {
        let commits: BTreeMap<&str, i64> = self
            .machines
            .iter()
            .flat_map(|machine| &machine.commits)
            .map(|(commit, date)| (commit.as_str(), *date))
            .collect();
        let mut hours = [0; 24];
        for date in commits.values() {
            hours[(date.rem_euclid(86_400) / 3600) as usize] += 1;
        }
        let window = |start: usize| (start..start + HOURS).map(|h| hours[h % 24]).sum::<usize>();
        let start = (0..24).max_by_key(|&start| (window(start), std::cmp::Reverse(start)));
        let start = start.expect("a day has hours");
        (start, window(start))
    }

    /// A change of the space key at `at`, in seconds since the Unix epoch: the machines that
    /// publish before it and after it, which publish their fingerprints again under the new key,
    /// and the merges whose two machines share fingerprints and are both among them, which overlap
    /// under neither key until both have.
    pub fn change(&self, at: i64) -> Change {
        let alive: BTreeSet<usize> = (0..self.machines.len())
            .filter(|&i| {
                let machine = &self.machines[i];
                machine.since() <= at && at < machine.until
            })
            .collect();
        let bytes = alive.iter().map(|&i| self.machines[i].fingerprints.len());
        let bytes = bytes.sum::<usize>() * FINGERPRINT_BYTES;
        let broken = self.merges.iter().filter(|sides| match sides {
            [Some(a), Some(b)] => {
                let shared = (self.machines[*a].fingerprints)
                    .intersection(&self.machines[*b].fingerprints)
                    .next()
                    .is_some();
                shared && alive.contains(a) && alive.contains(b)
            }
            _ => false,
        });
        Change {
            machines: alive.len(),
            bytes,
            overlaps: broken.count(),
        }
    }
}

/// The number of consecutive hours [`View::busiest_hours`] counts.
pub const HOURS: usize = 8;

/// The bytes of a fingerprint.
const FINGERPRINT_BYTES: usize = 32;

/// What a change of the space key costs, as [`View::change`] gives it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    /// The machines that publish again.
    pub machines: usize,
    /// The bytes of their fingerprints, at the size of their last publication.
    pub bytes: usize,
    /// The merges whose machines share fingerprints and publish again.
    pub overlaps: usize,
}

/// A measured repository with its replays, and what a holder of the space key recovers of their
/// fingerprints: with the fixed dictionary, then with a clone at the oldest merge replayed.
pub struct Exposed<'r> {
    pub repository: &'r Repository,
    pub replays: Vec<Replay>,
    pub recovered: [Recovered; 2],
}

/// The fingerprints of each level of `replays`, each once.
pub fn published(replays: &[Replay]) -> [BTreeSet<Fingerprint>; 3] {
    let mut levels: [BTreeSet<Fingerprint>; 3] = Default::default();
    for side in replays.iter().flat_map(|replay| &replay.sides) {
        for (level, fingerprints) in levels.iter_mut().zip(side.edits.levels()) {
            level.extend(fingerprints);
        }
    }
    levels
}

/// What the holder of the replays' space key recovers of the fingerprints of `replays`, merges of
/// the repository at `dir` in `scope`, the newest first: with the fixed [`Dictionary`], then with
/// a clone at the oldest merge.
pub fn recover(dir: &Path, scope: &Scope, replays: &[Replay]) -> Result<[Recovered; 2]> {
    let key = replay::key();
    let keying = Keying::Key(&key);
    let all: BTreeSet<Fingerprint> = published(replays).into_iter().flatten().collect();
    let with_dictionary = dictionary::recover(keying, scope, &Dictionary::fixed(), &all);
    let with_clone = match replays.last() {
        Some(oldest) => dictionary::recover_with_clone(keying, scope, dir, &oldest.merge, &all)?,
        None => Recovered::default(),
    };
    Ok([with_dictionary, with_clone])
}

/// `part` of `whole` as `<part> (<percent>%)`, the percent rounded down to a tenth; `-` for an
/// empty whole.
fn share(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "-".into();
    }
    let tenths = part * 1000 / whole;
    format!("{part} ({}.{}%)", tenths / 10, tenths % 10)
}

/// The median, the 90th percentile and the largest of `values`, sorted in place; `-` for none.
fn spread(values: &mut [usize]) -> [String; 3] {
    values.sort_unstable();
    if values.is_empty() {
        return ["-".into(), "-".into(), "-".into()];
    }
    let values: Vec<i64> = values.iter().map(|&value| value as i64).collect();
    [50, 90, 100].map(|percent| percentile(&values, percent).to_string())
}

/// The findings on `exposed` as Markdown tables.
pub fn report(exposed: &[Exposed]) -> String {
    let mut out = String::new();
    let views: Vec<(&str, View)> = exposed
        .iter()
        .map(|e| (e.repository.name, View::of(&e.replays)))
        .collect();

    let title = "What the server sees of each machine";
    let header = [
        "Repository",
        "Machines",
        "Publications, at least",
        "Publications per machine, median",
        "Fingerprints per machine, median",
        "90th percentile",
        "Most",
    ];
    table(&mut out, title, &header.map(str::to_owned), |row| {
        for (name, view) in &views {
            let mut publications: Vec<usize> =
                view.machines.iter().map(|m| m.commits.len()).collect();
            let total = publications.iter().sum::<usize>();
            let [median_publications, ..] = spread(&mut publications);
            let mut sizes: Vec<usize> =
                view.machines.iter().map(|m| m.fingerprints.len()).collect();
            let [median, p90, most] = spread(&mut sizes);
            row(vec![
                (*name).into(),
                view.machines.len().to_string(),
                total.to_string(),
                median_publications,
                median,
                p90,
                most,
            ]);
        }
    });

    let title = "Machines that share fingerprints";
    let header = [
        "Repository",
        "Merges",
        "Whose two machines share fingerprints",
        "Shared, median",
        "90th percentile",
        "Most",
        "Groups of machines linked by shared fingerprints",
        "Machines in the largest",
        "Machines that share none",
    ];
    table(&mut out, title, &header.map(str::to_owned), |row| {
        for (name, view) in &views {
            let mut shared = view.shared();
            let merges = share(shared.len(), view.merges.len());
            let [median, p90, most] = spread(&mut shared);
            let groups = view.groups();
            let alone = groups.iter().filter(|&&size| size == 1).count();
            let machines = view.machines.len();
            row(vec![
                (*name).into(),
                view.merges.len().to_string(),
                merges,
                median,
                p90,
                most,
                groups.len().to_string(),
                share(groups.first().copied().unwrap_or(0), machines),
                share(alone, machines),
            ]);
        }
    });

    let title = "How many machines publish each fingerprint";
    let header = [
        "Repository",
        "Fingerprints",
        "1 machine",
        "2 to 9",
        "10 or more",
        "Most",
        "Published in another repository too",
    ];
    table(&mut out, title, &header.map(str::to_owned), |row| {
        let recurrences: Vec<BTreeMap<Fingerprint, usize>> =
            views.iter().map(|(_, view)| view.recurrence()).collect();
        for (i, ((name, _), recurrence)) in views.iter().zip(&recurrences).enumerate() {
            let count = |range: std::ops::RangeInclusive<usize>| {
                let machines = recurrence.values().filter(|n| range.contains(n));
                share(machines.count(), recurrence.len())
            };
            let elsewhere = recurrence
                .keys()
                .filter(|fingerprint| {
                    let others = recurrences.iter().enumerate().filter(|(j, _)| *j != i);
                    others
                        .into_iter()
                        .any(|(_, other)| other.contains_key(fingerprint))
                })
                .count();
            let most = recurrence
                .values()
                .max()
                .map_or("-".into(), usize::to_string);
            row(vec![
                (*name).into(),
                recurrence.len().to_string(),
                count(1..=1),
                count(2..=9),
                count(10..=usize::MAX),
                most,
                elsewhere.to_string(),
            ]);
        }
    });

    let title = "When the machines publish: their commits, each once, by hour of the day, UTC";
    let header = ["Repository", "Commits", "Busiest hours", "Commits in them"];
    table(&mut out, title, &header.map(str::to_owned), |row| {
        for (name, view) in &views {
            let commits: BTreeSet<&str> = view
                .machines
                .iter()
                .flat_map(|machine| &machine.commits)
                .map(|(commit, _)| commit.as_str())
                .collect();
            let (start, busiest) = view.busiest_hours();
            let hours = format!("{start:02}:00 to {:02}:00", (start + HOURS) % 24);
            row(vec![
                (*name).into(),
                commits.len().to_string(),
                hours,
                share(busiest, commits.len()),
            ]);
        }
    });

    let title = "What a holder of the space key recovers: with the fixed dictionary, and with a clone \
        at the oldest merge replayed";
    let header = [
        "Repository",
        "Path fingerprints",
        "Recovered with the dictionary",
        "With the clone",
        "Symbol fingerprints",
        "Recovered with the dictionary",
        "With the clone",
    ];
    table(&mut out, title, &header.map(str::to_owned), |row| {
        for exposed in exposed {
            let [paths, _, symbols] = published(&exposed.replays);
            let [dictionary, clone] = &exposed.recovered;
            row(vec![
                exposed.repository.name.into(),
                paths.len().to_string(),
                share(dictionary.paths.len(), paths.len()),
                share(clone.paths.len(), paths.len()),
                symbols.len().to_string(),
                share(dictionary.symbols.len(), symbols.len()),
                share(clone.symbols.len(), symbols.len()),
            ]);
        }
    });

    let title = "What the fixed dictionary recovers";
    let header = ["Repository", "Path", "Symbol", "Machines that publish it"];
    table(&mut out, title, &header.map(str::to_owned), |row| {
        for (exposed, (_, view)) in exposed.iter().zip(&views) {
            let recurrence = view.recurrence();
            let machines = |fingerprint| recurrence.get(fingerprint).copied().unwrap_or(0);
            let [dictionary, _] = &exposed.recovered;
            let paths = (dictionary.paths.iter())
                .map(|(fingerprint, path)| (path.clone(), "-".to_owned(), fingerprint));
            let symbols = (dictionary.symbols.iter()).map(|(fingerprint, (path, symbol))| {
                (path.clone(), format!("`{symbol}`"), fingerprint)
            });
            let mut rows: Vec<_> = paths.chain(symbols).collect();
            rows.sort();
            for (path, symbol, fingerprint) in rows {
                let name = exposed.repository.name.to_owned();
                row(vec![
                    name,
                    format!("`{path}`"),
                    symbol,
                    machines(fingerprint).to_string(),
                ]);
            }
        }
    });

    let title = "A change of the space key at the time of each merge replayed";
    let header = [
        "Repository",
        "Changes",
        "Machines that publish again, median",
        "Most",
        "Their bytes, median",
        "Most",
        "Overlaps of merges lost until both machines publish again, median",
        "Most",
    ];
    table(&mut out, title, &header.map(str::to_owned), |row| {
        for (exposed, (name, view)) in exposed.iter().zip(&views) {
            let changes: Vec<Change> = exposed
                .replays
                .iter()
                .map(|replay| view.change(replay.merged_at))
                .collect();
            let column = |value: fn(&Change) -> usize| {
                let mut values: Vec<usize> = changes.iter().map(value).collect();
                let [median, _, most] = spread(&mut values);
                [median, most]
            };
            let [machines, machines_most] = column(|c| c.machines);
            let [bytes, bytes_most] = column(|c| c.bytes);
            let [overlaps, overlaps_most] = column(|c| c.overlaps);
            row(vec![
                (*name).into(),
                changes.len().to_string(),
                machines,
                machines_most,
                bytes,
                bytes_most,
                overlaps,
                overlaps_most,
            ]);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2023-11-14 00:00 UTC, in seconds since the Unix epoch.
    const DAY: i64 = 1_699_920_000;
    const HOUR: i64 = 3600;

    /// A machine with the fingerprints `[n; 32]` of `fingerprints`, `commits` at their hours after
    /// [`DAY`], until `until` hours after it.
    fn machine(fingerprints: &[u8], commits: &[(&str, i64)], until: i64) -> Machine {
        Machine {
            fingerprints: fingerprints.iter().map(|&n| Fingerprint([n; 32])).collect(),
            commits: commits
                .iter()
                .map(|&(commit, hours)| (commit.to_owned(), DAY + hours * HOUR))
                .collect(),
            until: DAY + until * HOUR,
        }
    }

    /// Four machines: the first two share a fingerprint, the second and the fourth another, the
    /// third shares none. Merges: of the first two, of the last two, and of a side without
    /// fingerprints and the first.
    fn view() -> View {
        View {
            machines: vec![
                machine(&[1, 2, 3], &[("a", 23), ("b", 25)], 30),
                machine(&[3, 4], &[("c", 29), ("a", 23)], 40),
                machine(&[5], &[("d", 5)], 6),
                machine(&[4, 6], &[("e", 12)], 13),
            ],
            merges: vec![[Some(0), Some(1)], [Some(2), Some(3)], [None, Some(0)]],
        }
    }

    #[test]
    fn shared_gives_the_fingerprints_the_machines_of_a_merge_share() {
        assert_eq!(view().shared(), [1]);
        let mut view = view();
        view.merges[1] = [Some(1), Some(3)];
        assert_eq!(view.shared(), [1, 1]);
        view.machines[3].fingerprints.extend([Fingerprint([3; 32])]);
        assert_eq!(view.shared(), [1, 2]);
    }

    #[test]
    fn groups_are_the_machines_shared_fingerprints_link() {
        assert_eq!(view().groups(), [3, 1]);
        let mut apart = view();
        apart.machines[3].fingerprints = [Fingerprint([7; 32])].into();
        assert_eq!(apart.groups(), [2, 1, 1]);
        assert_eq!(View::default().groups(), Vec::<usize>::new());
    }

    #[test]
    fn recurrence_counts_the_machines_of_each_fingerprint() {
        let recurrence: Vec<(u8, usize)> = view()
            .recurrence()
            .into_iter()
            .map(|(fingerprint, machines)| (fingerprint.0[0], machines))
            .collect();
        assert_eq!(recurrence, [(1, 1), (2, 1), (3, 2), (4, 2), (5, 1), (6, 1)]);
    }

    #[test]
    fn busiest_hours_count_each_commit_once_across_midnight() {
        // Commits at 23:00, 01:00 the next day, 05:00, 12:00 and 05:00 the next day; `a` twice.
        assert_eq!(view().busiest_hours(), (22, 4));
        let mut noon = view();
        noon.machines[2].commits = vec![("d".into(), DAY + 12 * HOUR)];
        // 12:00 twice, 23:00 and 05:00 the next day: from 05:00 to 13:00.
        assert_eq!(noon.busiest_hours(), (5, 3));
        assert_eq!(View::default().busiest_hours(), (0, 0));
    }

    #[test]
    fn a_key_change_is_paid_by_the_machines_that_publish_across_it() {
        let view = view();
        let change = |hours: i64| view.change(DAY + hours * HOUR);
        // The first machine from 23 to 30 hours, the second from 23 to 40.
        assert_eq!(
            change(23),
            Change {
                machines: 2,
                bytes: 5 * 32,
                overlaps: 1
            }
        );
        assert_eq!(
            change(30),
            Change {
                machines: 1,
                bytes: 2 * 32,
                overlaps: 0
            }
        );
        // The third from 5 to 6 hours, the fourth from 12 to 13: they share nothing.
        assert_eq!(
            change(5),
            Change {
                machines: 1,
                bytes: 32,
                overlaps: 0
            }
        );
        assert_eq!(change(22), Change::default());
        assert_eq!(change(40), Change::default());
    }

    #[test]
    fn share_and_spread_give_counts_percents_and_percentiles() {
        assert_eq!(share(1, 3), "1 (33.3%)");
        assert_eq!(share(2, 3), "2 (66.6%)");
        assert_eq!(share(3, 3), "3 (100.0%)");
        assert_eq!(share(14, 1496), "14 (0.9%)");
        assert_eq!(share(0, 0), "-");
        let mut values: Vec<usize> = (1..=20).rev().collect();
        assert_eq!(spread(&mut values), ["10", "18", "20"]);
        assert_eq!(spread(&mut []), ["-", "-", "-"]);
    }
}
