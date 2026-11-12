//! A machine's side: it fingerprints its edit set, its hunks and its symbols with the space key,
//! maps the overlaps the server reports back to its own paths, lines and symbols, and checks them
//! with the other machine's changes.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use crate::messages::{Message, Messages};
use crate::symbols::{self, Language, Symbol};
use crate::{
    Check, Fingerprint, MachineId, Overlap, Publication, Result, Scope, SpaceKey, check,
    fingerprint, git, hunks,
};

/// A machine of a space.
#[derive(Clone)]
pub struct Client {
    pub machine: MachineId,
    pub key: SpaceKey,
}

/// A machine's edit set: what it publishes, the paths behind each of its path fingerprints, the
/// changed lines behind each of its hunk fingerprints, and the symbols behind each of its symbol
/// fingerprints. Paths that differ only in case or in Unicode normalization share fingerprints.
#[derive(Debug, Clone)]
pub struct Edits {
    pub publication: Publication,
    paths: BTreeMap<Fingerprint, BTreeSet<String>>,
    /// The changed lines behind each hunk fingerprint, each a path and a range; one fingerprint
    /// can stand for several.
    hunks: BTreeMap<Fingerprint, Vec<(String, Range<usize>)>>,
    /// The symbols behind each symbol fingerprint, each with its path.
    symbols: BTreeMap<Fingerprint, BTreeSet<(String, Symbol)>>,
    /// The paths whose changes delete or replace no line of the merge base.
    adding: BTreeSet<String>,
}

/// Paths with ranges of this machine's lines, 1-based and end-exclusive, as [`Check::files`].
pub type Lines = BTreeMap<String, Vec<Range<usize>>>;

/// Paths with symbols of this machine's files.
pub type Symbols = BTreeMap<String, BTreeSet<Symbol>>;

impl Client {
    pub fn new(machine: impl Into<MachineId>, key: SpaceKey) -> Self {
        Self {
            machine: machine.into(),
            key,
        }
    }

    /// The edit set of the working tree at `dir`, its root, with the fingerprint of each path, of
    /// each boundary its hunks touch, and of each symbol that is the innermost one enclosing a
    /// line the hunks change, in the merge base or in the snapshot. The hunks are the changes from
    /// the merge base with the upstream's default branch to a snapshot of the working tree, as
    /// `git merge-tree` sees them.
    pub fn edits(&self, dir: &Path) -> Result<Edits> {
        let scope = git::scope(dir)?;
        let paths = git::edit_set(dir)?;
        let base = git::merge_base(dir)?;
        let snapshot = git::snapshot(dir)?;
        self.edits_between(dir, &scope, &base, &snapshot, paths)
    }

    /// The edit set `paths` of the changes from the commit `base` to the commit `commit` in the
    /// repository at `dir`, fingerprinted in `scope` as [`Client::edits`] does.
    pub fn edits_between(
        &self,
        dir: &Path,
        scope: &Scope,
        base: &str,
        commit: &str,
        paths: BTreeSet<String>,
    ) -> Result<Edits> {
        let mut by_fingerprint = BTreeMap::<Fingerprint, BTreeSet<String>>::new();
        for path in &paths {
            let fingerprint = fingerprint::path(&self.key, scope, path);
            by_fingerprint
                .entry(fingerprint)
                .or_default()
                .insert(path.clone());
        }
        let mut changed = BTreeMap::<Fingerprint, Vec<(String, Range<usize>)>>::new();
        let mut symbols = BTreeMap::<Fingerprint, BTreeSet<(String, Symbol)>>::new();
        let mut adding = BTreeSet::new();
        for path in &paths {
            let changes = hunks::changes(dir, base, commit, path)?;
            if changes.iter().all(|change| change.old.is_empty()) {
                adding.insert(path.clone());
            }
            for change in &changes {
                for (before, after) in &change.boundaries {
                    let fingerprint = fingerprint::hunk(&self.key, scope, path, before, after);
                    let lines = (path.clone(), change.lines.clone());
                    changed.entry(fingerprint).or_default().push(lines);
                }
            }
            let Some(language) = Language::of(path) else {
                continue;
            };
            // The lines the changes delete or replace in the merge base, and the lines they add
            // in `commit`; a side without lines may have no file.
            let base_lines: Vec<usize> = changes.iter().flat_map(|c| c.old.clone()).collect();
            let new_lines: Vec<usize> = changes.iter().flat_map(|c| c.lines.clone()).collect();
            for (commit, lines) in [(base, base_lines), (commit, new_lines)] {
                if lines.is_empty() {
                    continue;
                }
                let source = git::blob(dir, commit, path)?;
                for symbol in symbols::enclosing(language, &source, lines)? {
                    let fingerprint = fingerprint::symbol(&self.key, scope, path, &symbol);
                    symbols
                        .entry(fingerprint)
                        .or_default()
                        .insert((path.clone(), symbol));
                }
            }
        }
        Ok(Edits {
            publication: Publication {
                machine: self.machine.clone(),
                fingerprints: by_fingerprint
                    .keys()
                    .chain(changed.keys())
                    .chain(symbols.keys())
                    .copied()
                    .collect(),
            },
            paths: by_fingerprint,
            hunks: changed,
            symbols,
            adding,
        })
    }

    /// Asks `other` for its changes, to check an overlap with it; the working tree at `dir` gives
    /// the base the answer leaves out.
    pub fn request_changes(&self, dir: &Path, other: &str, messages: &mut Messages) -> Result<()> {
        let base = git::merge_base(dir)?;
        messages.send(&self.machine, other, Message::ChangesRequest { base });
        Ok(())
    }

    /// Answers each request for changes in this machine's inbox with a bundle of a commit of the
    /// working tree at `dir`.
    pub fn answer_requests(&self, dir: &Path, messages: &mut Messages) -> Result<()> {
        let requests = messages.take_requests(&self.machine);
        if requests.is_empty() {
            return Ok(());
        }
        let snapshot = git::snapshot(dir)?;
        for (other, base) in requests {
            let bundle = git::bundle(dir, &snapshot, &base)?;
            messages.send(&self.machine, &other, Message::Changes { bundle });
        }
        Ok(())
    }

    /// Checks the changes in this machine's inbox against the working tree at `dir`: for each
    /// machine that sent them, what `git merge-tree` finds when it merges them into this one's.
    pub fn check(&self, dir: &Path, messages: &mut Messages) -> Result<BTreeMap<MachineId, Check>> {
        let changes = messages.take_changes(&self.machine);
        if changes.is_empty() {
            return Ok(BTreeMap::new());
        }
        let ours = git::snapshot(dir)?;
        changes
            .into_iter()
            .map(|(other, bundle)| {
                let theirs = git::unbundle(dir, &bundle)?;
                Ok((other, check::merge(dir, &ours, &theirs)?))
            })
            .collect()
    }
}

impl Edits {
    /// The paths of the edit set, sorted.
    pub fn paths(&self) -> BTreeSet<&str> {
        self.paths.values().flatten().map(String::as_str).collect()
    }

    /// For each overlapping machine, the paths of this edit set that it edits too.
    pub fn overlapping_paths(&self, overlaps: &[Overlap]) -> BTreeMap<MachineId, BTreeSet<String>> {
        overlaps
            .iter()
            .map(|overlap| {
                let paths = overlap
                    .fingerprints
                    .iter()
                    .filter_map(|fingerprint| self.paths.get(fingerprint))
                    .flatten()
                    .cloned()
                    .collect();
                (overlap.machine.clone(), paths)
            })
            .collect()
    }

    /// Whether the changes of `path` delete or replace no line of the merge base: they only add
    /// lines.
    pub fn only_adds(&self, path: &str) -> bool {
        self.adding.contains(path)
    }

    /// The number of fingerprints of paths, of hunk boundaries and of symbols.
    pub fn sizes(&self) -> [usize; 3] {
        [self.paths.len(), self.hunks.len(), self.symbols.len()]
    }

    /// The fingerprints of paths, of hunk boundaries and of symbols.
    pub fn levels(&self) -> [BTreeSet<Fingerprint>; 3] {
        [
            self.paths.keys().copied().collect(),
            self.hunks.keys().copied().collect(),
            self.symbols.keys().copied().collect(),
        ]
    }

    /// Each path of the edit set with the symbols its changed lines are in.
    pub fn symbols(&self) -> Symbols {
        by_path(self.symbols.values().flatten())
    }

    /// For each machine that overlaps at symbol level, this machine's symbols that it changes
    /// too, by path.
    pub fn overlapping_symbols(&self, overlaps: &[Overlap]) -> BTreeMap<MachineId, Symbols> {
        overlaps
            .iter()
            .filter_map(|overlap| {
                let shared = overlap.fingerprints.iter();
                let symbols = by_path(
                    shared
                        .filter_map(|fingerprint| self.symbols.get(fingerprint))
                        .flatten(),
                );
                (!symbols.is_empty()).then(|| (overlap.machine.clone(), symbols))
            })
            .collect()
    }

    /// For each machine that overlaps at hunk level, this machine's changed lines whose hunks
    /// touch a boundary that it touches too, by path; ranges sorted and without repeats.
    pub fn overlapping_hunks(&self, overlaps: &[Overlap]) -> BTreeMap<MachineId, Lines> {
        overlaps
            .iter()
            .filter_map(|overlap| {
                let mut lines = BTreeMap::<String, BTreeSet<(usize, usize)>>::new();
                for fingerprint in &overlap.fingerprints {
                    for (path, range) in self.hunks.get(fingerprint).into_iter().flatten() {
                        let ranges = lines.entry(path.clone()).or_default();
                        ranges.insert((range.start, range.end));
                    }
                }
                let lines: Lines = lines
                    .into_iter()
                    .map(|(path, ranges)| {
                        let ranges = ranges.into_iter().map(|(start, end)| start..end);
                        (path, ranges.collect())
                    })
                    .collect();
                (!lines.is_empty()).then(|| (overlap.machine.clone(), lines))
            })
            .collect()
    }
}

/// `symbols`, each a path and a symbol, by path.
fn by_path<'s>(symbols: impl Iterator<Item = &'s (String, Symbol)>) -> Symbols {
    let mut by_path = Symbols::new();
    for (path, symbol) in symbols {
        let path = by_path.entry(path.clone()).or_default();
        path.insert(symbol.clone());
    }
    by_path
}
