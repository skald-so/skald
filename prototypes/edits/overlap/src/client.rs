//! A machine's side: it fingerprints its edit set with the space key, maps the overlaps the
//! server reports back to its own paths, and checks them with the other machine's changes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::messages::{Message, Messages};
use crate::{
    Check, Fingerprint, MachineId, Overlap, Publication, Result, SpaceKey, check, fingerprint, git,
};

/// A machine of a space.
#[derive(Clone)]
pub struct Client {
    pub machine: MachineId,
    pub key: SpaceKey,
}

/// A machine's edit set: what it publishes, and the path behind each of its fingerprints.
#[derive(Debug, Clone)]
pub struct Edits {
    pub publication: Publication,
    paths: BTreeMap<Fingerprint, String>,
}

impl Client {
    pub fn new(machine: impl Into<MachineId>, key: SpaceKey) -> Self {
        Self {
            machine: machine.into(),
            key,
        }
    }

    /// The edit set of the working tree at `dir`, its root, with the fingerprint of each path.
    pub fn edits(&self, dir: &Path) -> Result<Edits> {
        let scope = git::scope(dir)?;
        let paths: BTreeMap<_, _> = git::edit_set(dir)?
            .into_iter()
            .map(|path| (fingerprint::path(&self.key, &scope, &path), path))
            .collect();
        Ok(Edits {
            publication: Publication {
                machine: self.machine.clone(),
                fingerprints: paths.keys().copied().collect(),
            },
            paths,
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
}
