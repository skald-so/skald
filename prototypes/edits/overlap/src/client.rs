//! A machine's side: it fingerprints its edit set and its hunks with the space key, maps the
//! overlaps the server reports back to its own paths and lines, and checks them with the other
//! machine's changes.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

use crate::messages::{Message, Messages};
use crate::{
    Check, Fingerprint, MachineId, Overlap, Publication, Result, SpaceKey, check, fingerprint, git,
    hunks,
};

/// A machine of a space.
#[derive(Clone)]
pub struct Client {
    pub machine: MachineId,
    pub key: SpaceKey,
}

/// A machine's edit set: what it publishes, the path behind each of its path fingerprints, and
/// the changed lines behind each of its hunk fingerprints.
#[derive(Debug, Clone)]
pub struct Edits {
    pub publication: Publication,
    paths: BTreeMap<Fingerprint, String>,
    /// The changed lines behind each hunk fingerprint, each a path and a range; one fingerprint
    /// can stand for several.
    hunks: BTreeMap<Fingerprint, Vec<(String, Range<usize>)>>,
}

/// Paths with ranges of this machine's lines, 1-based and end-exclusive, as [`Check::files`].
pub type Lines = BTreeMap<String, Vec<Range<usize>>>;

impl Client {
    pub fn new(machine: impl Into<MachineId>, key: SpaceKey) -> Self {
        Self {
            machine: machine.into(),
            key,
        }
    }

    /// The edit set of the working tree at `dir`, its root, with the fingerprint of each path and
    /// of each boundary its hunks touch. The hunks are the changes from the merge base with the
    /// upstream's default branch to a snapshot of the working tree, as `git merge-tree` sees them.
    pub fn edits(&self, dir: &Path) -> Result<Edits> {
        let scope = git::scope(dir)?;
        let paths: BTreeMap<_, _> = git::edit_set(dir)?
            .into_iter()
            .map(|path| (fingerprint::path(&self.key, &scope, &path), path))
            .collect();
        let base = git::merge_base(dir)?;
        let snapshot = git::snapshot(dir)?;
        let mut changed = BTreeMap::<Fingerprint, Vec<(String, Range<usize>)>>::new();
        for path in paths.values() {
            for change in hunks::changes(dir, &base, &snapshot, path)? {
                for (before, after) in &change.boundaries {
                    let fingerprint = fingerprint::hunk(&self.key, &scope, path, before, after);
                    let lines = (path.clone(), change.lines.clone());
                    changed.entry(fingerprint).or_default().push(lines);
                }
            }
        }
        Ok(Edits {
            publication: Publication {
                machine: self.machine.clone(),
                fingerprints: paths.keys().chain(changed.keys()).copied().collect(),
            },
            paths,
            hunks: changed,
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
        self.paths.values().map(String::as_str).collect()
    }

    /// For each overlapping machine, the paths of this edit set that it edits too.
    pub fn overlapping_paths(&self, overlaps: &[Overlap]) -> BTreeMap<MachineId, BTreeSet<String>> {
        overlaps
            .iter()
            .map(|overlap| {
                let paths = overlap
                    .fingerprints
                    .iter()
                    .filter_map(|fingerprint| self.paths.get(fingerprint).cloned())
                    .collect();
                (overlap.machine.clone(), paths)
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
