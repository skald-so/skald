//! The measurement on real repositories: each merge of a repository replayed as two machines, each
//! with the changes of one parent since their merge base, and the warnings of each level compared
//! with the conflicts of `git merge-tree`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::noise::{self, Class, Signal};
use crate::{Client, Conflict, Edits, Result, Scope, Server, SpaceKey, check, git};

/// A level of fingerprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    File,
    Hunk,
    Symbol,
}

impl Level {
    pub const ALL: [Self; 3] = [Self::File, Self::Hunk, Self::Symbol];

    pub fn name(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Hunk => "hunk",
            Self::Symbol => "symbol",
        }
    }
}

/// A measured repository: the URL it is fetched from, the commit it is fetched at, and the
/// number of merges replayed.
pub struct Repository {
    pub name: &'static str,
    pub url: &'static str,
    pub commit: &'static str,
    pub merges: usize,
}

/// The two machines of a replay: the first parent's and the second parent's.
pub const MACHINES: [&str; 2] = ["first-parent", "second-parent"];

/// The space key of the replays. Any key gives the same counts.
pub fn key() -> SpaceKey {
    SpaceKey::from_bytes([0; 32])
}

/// The merges to replay: the `limit` first merges of two parents that `git rev-list` lists from
/// `commit` in the repository at `dir`, the newest first. Merges of more parents are left out.
pub fn merges(dir: &Path, commit: &str, limit: usize) -> Result<Vec<String>> {
    let limit = format!("--max-count={limit}");
    let merges = git::run(
        dir,
        &["rev-list", "--merges", "--max-parents=2", &limit, commit],
    )?;
    Ok(String::from_utf8(merges)?
        .lines()
        .map(str::to_owned)
        .collect())
}

/// One parent of a replayed merge, as a machine.
#[derive(Debug)]
pub struct Side {
    /// What the machine publishes: its changes from the merge base to the parent.
    pub edits: Edits,
    /// The seconds from the author date of the side's first commit after the merge base, along its
    /// first-parent history, to the merge's committer date; none when the parent is the merge
    /// base.
    pub lifetime: Option<i64>,
}

/// A merge replayed as two machines.
#[derive(Debug)]
pub struct Replay {
    pub merge: String,
    /// The merge's committer date, `YYYY-MM-DD`.
    pub date: String,
    /// The merge's first and second parents.
    pub parents: [String; 2],
    /// The parents' merge base: the first one `git merge-base --all` gives.
    pub base: String,
    /// The number of the parents' merge bases.
    pub bases: usize,
    /// The first parent's side, then the second's.
    pub sides: [Side; 2],
    /// Whether `git merge-tree` merges the parents cleanly.
    pub clean: bool,
    /// The conflicts `git merge-tree` reports.
    pub conflicts: Vec<Conflict>,
    /// For each level, in [`Level::ALL`] order, the paths it warns about: those both sides
    /// publish a fingerprint of that level of.
    pub warnings: [BTreeSet<String>; 3],
    /// The warned paths that are the same in both parents: both sides made the same change.
    pub same: BTreeSet<String>,
    /// The class of each file of a file-level warning or a conflict, and what recognized it, as
    /// [`noise::classify_all`] gives them for the parents.
    pub classes: BTreeMap<String, (Class, Option<Signal>)>,
    /// The migrations each side adds: the paths it adds since the merge base that are in a
    /// [`noise::migration`].
    pub migrations: [BTreeSet<String>; 2],
}

/// Replays `merge`, a commit of two parents in the repository at `dir`, with fingerprints in
/// `scope`: each parent's changes since their merge base, as a machine, publish to a server
/// stand-in, and `git merge-tree` merges the parents.
pub fn replay(dir: &Path, scope: &Scope, merge: &str) -> Result<Replay> {
    let parent = |n: usize| git::line(dir, &["rev-parse", "--verify", &format!("{merge}^{n}")]);
    let parents = [parent(1)?, parent(2)?];
    let bases = git::line(dir, &["merge-base", "--all", &parents[0], &parents[1]])?;
    let base = bases
        .lines()
        .next()
        .ok_or("parents without a merge base")?
        .to_owned();
    let bases = bases.lines().count();
    let committed = git::line(dir, &["log", "-1", "--format=%ct %cs", merge])?;
    let (merged_at, date) = committed
        .split_once(' ')
        .ok_or_else(|| format!("a commit date without a day: {committed}"))?;
    let merged_at: i64 = merged_at.parse()?;
    let mut server = Server::default();
    let mut sides = Vec::new();
    for (machine, parent) in MACHINES.iter().zip(&parents) {
        let edits = edits(dir, scope, machine, &base, parent)?;
        server.publish(edits.publication.clone());
        // The author date of each commit along the first-parent history, the first one last.
        let range = format!("{base}..{parent}");
        let dates = git::run(dir, &["log", "--first-parent", "--format=%at", &range])?;
        let lifetime = match String::from_utf8(dates)?.lines().last() {
            Some(first) => Some(merged_at - first.parse::<i64>()?),
            None => None,
        };
        sides.push(Side { edits, lifetime });
    }
    let sides: [Side; 2] = sides.try_into().map_err(|_| "a merge without two sides")?;
    let overlaps = server.overlaps(MACHINES[0]);
    let ours = &sides[0].edits;
    let files = ours.overlapping_paths(&overlaps).into_values().flatten();
    let hunks = ours.overlapping_hunks(&overlaps).into_values();
    let symbols = ours.overlapping_symbols(&overlaps).into_values();
    let warnings: [BTreeSet<String>; 3] = [
        files.collect(),
        hunks.flat_map(|lines| lines.into_keys()).collect(),
        symbols.flat_map(|symbols| symbols.into_keys()).collect(),
    ];
    let (clean, conflicts) = check::conflicts(dir, &parents[0], &parents[1])?;
    let different = git::changed(dir, &parents[0], &parents[1])?;
    let same = warnings[Level::File as usize]
        .difference(&different)
        .cloned()
        .collect();
    let classed: BTreeSet<&str> = warnings[Level::File as usize]
        .iter()
        .chain(conflicts.iter().flat_map(|conflict| &conflict.paths))
        .map(String::as_str)
        .collect();
    let classes = noise::classify_all(dir, &parents, &classed)?;
    let mut migrations = [BTreeSet::new(), BTreeSet::new()];
    for (added, parent) in migrations.iter_mut().zip(&parents) {
        let paths = git::added(dir, &base, parent)?.into_iter();
        added.extend(paths.filter(|path| noise::migration(path).is_some()));
    }
    Ok(Replay {
        merge: merge.to_owned(),
        date: date.to_owned(),
        parents,
        base,
        bases,
        sides,
        clean,
        conflicts,
        warnings,
        same,
        classes,
        migrations,
    })
}

impl Replay {
    /// The conflicts none of whose files `level` warns about.
    pub fn unwarned(&self, level: Level) -> impl Iterator<Item = &Conflict> {
        let warned = &self.warnings[level as usize];
        self.conflicts
            .iter()
            .filter(|conflict| conflict.paths.iter().all(|path| !warned.contains(path)))
    }

    /// The files of the conflicts.
    pub fn conflicted(&self) -> BTreeSet<&String> {
        let paths = self.conflicts.iter().flat_map(|conflict| &conflict.paths);
        paths.collect()
    }
}

/// What `machine` publishes for the changes from `base` to `parent`, commits in the repository at
/// `dir`, with fingerprints in `scope`.
fn edits(dir: &Path, scope: &Scope, machine: &str, base: &str, parent: &str) -> Result<Edits> {
    let paths = git::changed(dir, base, parent)?;
    Client::new(machine, key()).edits_between(dir, scope, base, parent, paths)
}

/// Replays `merges` as [`replay`] does, on `jobs` threads, and calls `progress` with the number
/// replayed after each. The replays are in the order of `merges`.
pub fn replay_all(
    dir: &Path,
    scope: &Scope,
    merges: &[String],
    jobs: usize,
    progress: impl Fn(usize) + Sync,
) -> Result<Vec<Replay>> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let worker = || {
        let mut replays = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(merge) = merges.get(i) else {
                return replays;
            };
            let replayed = replay(dir, scope, merge);
            let failed = replayed.is_err();
            replays.push((i, replayed));
            progress(done.fetch_add(1, Ordering::Relaxed) + 1);
            if failed {
                // The other threads stop at their next merge.
                next.store(merges.len(), Ordering::Relaxed);
                return replays;
            }
        }
    };
    let mut replays: Vec<(usize, Result<Replay>)> = std::thread::scope(|threads| {
        let handles: Vec<_> = (0..jobs.max(1)).map(|_| threads.spawn(worker)).collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a replay thread panicked"))
            .collect()
    });
    replays.sort_by_key(|(i, _)| *i);
    replays.into_iter().map(|(_, replay)| replay).collect()
}

/// The times to compute the fingerprints of the [`LARGEST`] sides of `replays`, as [`largest`]
/// orders them, one after another, in the repository at `dir`, with fingerprints in `scope`.
pub fn cost(dir: &Path, scope: &Scope, replays: &[Replay]) -> Result<Vec<Duration>> {
    largest(replays, LARGEST)
        .into_iter()
        .map(|(replay, parent)| {
            let start = Instant::now();
            let machine = MACHINES[parent - 1];
            edits(
                dir,
                scope,
                machine,
                &replay.base,
                &replay.parents[parent - 1],
            )?;
            Ok(start.elapsed())
        })
        .collect()
}

/// A measured repository with its replays and the [`cost`] of their largest sides.
pub struct Measured<'r> {
    pub repository: &'r Repository,
    pub replays: Vec<Replay>,
    pub times: Vec<Duration>,
}

/// Fetches `repository` at its commit into a bare repository in `tmp`, replays its merges on
/// `jobs` threads, calling `progress` as [`replay_all`] does, then times its largest sides one
/// after another. Fails when the repository has fewer merges than `repository` says.
pub fn measure<'r>(
    tmp: &Path,
    repository: &'r Repository,
    jobs: usize,
    progress: impl Fn(usize) + Sync,
) -> Result<Measured<'r>> {
    let dir = tmp.join(format!("{}.git", repository.name));
    let path = dir.to_str().ok_or("a path that is not UTF-8")?;
    git::run(tmp, &["init", "--quiet", "--bare", path])?;
    let (url, commit) = (repository.url, repository.commit);
    git::run(&dir, &["fetch", "--quiet", "--no-tags", url, commit])?;
    let scope = git::scope_of(&dir, commit)?;
    let merges = merges(&dir, commit, repository.merges)?;
    if merges.len() != repository.merges {
        return Err(format!(
            "{} has {} merges of two parents, not {}",
            repository.name,
            merges.len(),
            repository.merges
        )
        .into());
    }
    let replays = replay_all(&dir, &scope, &merges, jobs, progress)?;
    let times = cost(&dir, &scope, &replays)?;
    Ok(Measured {
        repository,
        replays,
        times,
    })
}

/// Where macOS keeps `sysctl`, which names its CPU; it is not in the default search path of a
/// command without a `PATH`.
const SYSCTL: &str = "/usr/sbin/sysctl";

/// The operating system, the architecture, the CPU where `sysctl` names it, as on macOS, and the
/// number of CPUs.
pub fn machine() -> String {
    let cpu = std::process::Command::new(SYSCTL)
        .args(["-n", "machdep.cpu.brand_string"])
        .env_clear()
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    let cpus = std::thread::available_parallelism().map_or(0, |cpus| cpus.get());
    format!(
        "{} {}, {}{cpus} CPUs",
        std::env::consts::OS,
        std::env::consts::ARCH,
        cpu.map(|cpu| format!("{cpu}, ")).unwrap_or_default()
    )
}

/// The lines [`report`] goes after: the [`machine`] and the version of `git`, run in `dir`.
pub fn header(dir: &Path) -> Result<String> {
    let git = git::line(dir, &["version"])?;
    Ok(format!("Machine: {}\nGit: {git}\n\n", machine()))
}

/// What a level finds on replayed merges, counted in files and in merges.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counts {
    /// Files a merge's sides publish a fingerprint of that level of in common, one per merge.
    pub warnings: usize,
    /// The conflicts `git merge-tree` reports.
    pub conflicts: usize,
    /// Warnings whose file is in no conflict of the merge.
    pub warnings_without_conflict: usize,
    /// Conflicts none of whose files the level warns about.
    pub conflicts_without_warning: usize,
    /// Warnings whose file neither side deletes or replaces a line of: both only add lines.
    pub adding: usize,
    /// Adding warnings whose file is in a conflict of the merge.
    pub adding_with_conflict: usize,
    /// Warnings whose file is the same in both parents.
    pub same: usize,
    /// Merges with a warning.
    pub merges_warned: usize,
    /// Merges `git merge-tree` does not merge cleanly.
    pub merges_conflicted: usize,
    /// Merges with a warning that merge cleanly.
    pub merges_warned_clean: usize,
    /// Merges without a warning that do not merge cleanly.
    pub merges_conflicted_unwarned: usize,
}

impl Counts {
    /// The counts of `level` on `replays`.
    pub fn of(replays: &[Replay], level: Level) -> Self {
        let mut counts = Self::default();
        for replay in replays {
            let warned = &replay.warnings[level as usize];
            let conflicted = replay.conflicted();
            let adding: Vec<&String> = warned
                .iter()
                .filter(|path| replay.sides.iter().all(|side| side.edits.only_adds(path)))
                .collect();
            counts.warnings += warned.len();
            counts.conflicts += replay.conflicts.len();
            counts.warnings_without_conflict += warned
                .iter()
                .filter(|path| !conflicted.contains(path))
                .count();
            counts.conflicts_without_warning += replay.unwarned(level).count();
            counts.adding += adding.len();
            counts.same += warned.intersection(&replay.same).count();
            counts.adding_with_conflict += adding
                .iter()
                .filter(|path| conflicted.contains(*path))
                .count();
            let (is_warned, conflicts) = (!warned.is_empty(), !replay.clean);
            counts.merges_warned += usize::from(is_warned);
            counts.merges_conflicted += usize::from(conflicts);
            counts.merges_warned_clean += usize::from(is_warned && !conflicts);
            counts.merges_conflicted_unwarned += usize::from(!is_warned && conflicts);
        }
        counts
    }
}

/// The lifetimes of the sides of `replays` that have one, sorted.
pub fn lifetimes(replays: &[Replay]) -> Vec<i64> {
    let mut lifetimes: Vec<i64> = replays
        .iter()
        .flat_map(|replay| replay.sides.iter().filter_map(|side| side.lifetime))
        .collect();
    lifetimes.sort_unstable();
    lifetimes
}

/// The nearest-rank `percent` percentile, 1 to 100, of `sorted`, which is not empty.
pub fn percentile(sorted: &[i64], percent: usize) -> i64 {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

/// The `count` sides of `replays` with the most fingerprints, each with its merge and its parent,
/// 1 or 2; sides with as many in the order of `replays`, first parent first.
pub fn largest(replays: &[Replay], count: usize) -> Vec<(&Replay, usize)> {
    let mut sides: Vec<(&Replay, usize)> = replays
        .iter()
        .flat_map(|replay| [(replay, 1), (replay, 2)])
        .collect();
    sides.sort_by_key(|(replay, parent)| {
        let fingerprints = &replay.sides[parent - 1].edits.publication.fingerprints;
        std::cmp::Reverse(fingerprints.len())
    });
    sides.truncate(count);
    sides
}

/// The number of sides [`report`] gives the cost of, per repository.
pub const LARGEST: usize = 3;

/// The bytes of a fingerprint.
const FINGERPRINT_BYTES: usize = 32;

/// The findings on `measured` as Markdown tables, those of [`noise::report`] last. All but the
/// times are the same on every run.
pub fn report(measured: &[Measured]) -> String {
    let mut out = String::new();
    let hours = |seconds: i64| format!("{:.1}", seconds as f64 / 3600.0);
    let short = |merge: &str| merge[..12].to_owned();

    out += "Repositories\n\n";
    out += "| Repository | URL | Commit | Merges | Newest | Oldest | Both sides changed \
        | Several merge bases |\n";
    out += "|---|---|---|---|---|---|---|---|\n";
    for Measured {
        repository,
        replays,
        ..
    } in measured
    {
        let ends = |replay: Option<&Replay>| match replay {
            Some(replay) => format!("`{}` {}", short(&replay.merge), replay.date),
            None => "-".into(),
        };
        let both = replays
            .iter()
            .filter(|replay| {
                let sizes = replay.sides.each_ref().map(|side| side.edits.sizes()[0]);
                sizes.iter().all(|&paths| paths > 0)
            })
            .count();
        writeln!(
            out,
            "| {} | {} | `{}` | {} | {} | {} | {both} | {} |",
            repository.name,
            repository.url,
            repository.commit,
            replays.len(),
            ends(replays.first()),
            ends(replays.last()),
            replays.iter().filter(|replay| replay.bases > 1).count()
        )
        .expect("a String takes any write");
    }

    out += "\nWarnings and conflicts, in files\n\n";
    out += "| Repository | Level | Warnings | Conflicts | Warnings without a conflict \
        | Conflicts without a warning | Warnings where both sides only add | Of them with a conflict \
        | Warnings where both sides made the same change |\n";
    out += "|---|---|---|---|---|---|---|---|---|\n";
    for Measured {
        repository,
        replays,
        ..
    } in measured
    {
        for level in Level::ALL {
            let c = Counts::of(replays, level);
            writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                repository.name,
                level.name(),
                c.warnings,
                c.conflicts,
                c.warnings_without_conflict,
                c.conflicts_without_warning,
                c.adding,
                c.adding_with_conflict,
                c.same
            )
            .expect("a String takes any write");
        }
    }

    out += "\nConflicts without a file-level warning\n\n";
    out += "| Repository | Merge | Merge bases | Conflict | Files |\n";
    out += "|---|---|---|---|---|\n";
    for Measured {
        repository,
        replays,
        ..
    } in measured
    {
        for replay in replays.iter() {
            for conflict in replay.unwarned(Level::File) {
                let paths: Vec<String> = conflict
                    .paths
                    .iter()
                    .map(|path| format!("`{path}`"))
                    .collect();
                writeln!(
                    out,
                    "| {} | `{}` | {} | {} | {} |",
                    repository.name,
                    short(&replay.merge),
                    replay.bases,
                    conflict.kind,
                    paths.join(", ")
                )
                .expect("a String takes any write");
            }
        }
    }

    out += "\nWarnings and conflicts, in merges\n\n";
    out += "| Repository | Level | With a warning | With a conflict | Warned without a conflict \
        | Conflicted without a warning |\n";
    out += "|---|---|---|---|---|---|\n";
    for Measured {
        repository,
        replays,
        ..
    } in measured
    {
        for level in Level::ALL {
            let c = Counts::of(replays, level);
            writeln!(
                out,
                "| {} | {} | {} | {} | {} | {} |",
                repository.name,
                level.name(),
                c.merges_warned,
                c.merges_conflicted,
                c.merges_warned_clean,
                c.merges_conflicted_unwarned
            )
            .expect("a String takes any write");
        }
    }

    out += "\nSide lifetimes, in hours\n\n";
    out += "| Repository | Sides | Median | 90th percentile | Longest |\n";
    out += "|---|---|---|---|---|\n";
    for Measured {
        repository,
        replays,
        ..
    } in measured
    {
        let lifetimes = lifetimes(replays);
        let [median, p90, longest] = if lifetimes.is_empty() {
            ["-".into(), "-".into(), "-".into()]
        } else {
            [50, 90, 100].map(|percent| hours(percentile(&lifetimes, percent)))
        };
        writeln!(
            out,
            "| {} | {} | {median} | {p90} | {longest} |",
            repository.name,
            lifetimes.len()
        )
        .expect("a String takes any write");
    }

    out += "\nCost on the largest sides; times are not compared on a rerun\n\n";
    out += "| Repository | Merge | Parent | File | Hunk | Symbol | Bytes | Time, ms |\n";
    out += "|---|---|---|---|---|---|---|---|\n";
    for Measured {
        repository,
        replays,
        times,
    } in measured
    {
        for ((replay, parent), time) in largest(replays, LARGEST).into_iter().zip(times) {
            let side = &replay.sides[parent - 1];
            let [paths, hunks, symbols] = side.edits.sizes();
            let bytes = side.edits.publication.fingerprints.len() * FINGERPRINT_BYTES;
            writeln!(
                out,
                "| {} | `{}` | {parent} | {paths} | {hunks} | {symbols} | {bytes} | {} |",
                repository.name,
                short(&replay.merge),
                time.as_millis()
            )
            .expect("a String takes any write");
        }
    }
    out += &noise::report(measured);
    out
}
