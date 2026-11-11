//! A machine's side: it fingerprints its edit set with the space key and maps the overlaps the
//! server reports back to its own paths.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::{Fingerprint, MachineId, Overlap, Publication, Result, SpaceKey, fingerprint, git};

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
