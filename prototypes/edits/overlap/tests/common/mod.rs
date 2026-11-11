//! What the scenario tests share: the space key, the test repositories and a space of machines.
#![allow(dead_code, reason = "each test file uses some of the helpers")]

use std::collections::BTreeSet;
use std::ops::Range;

use edit_overlap::harness::{Machine, Upstream};
use edit_overlap::{Client, Edits, Fingerprint, Lines, MachineId, Messages, Server, SpaceKey};
use tempfile::TempDir;

/// The space key, a test fixture.
pub fn key() -> SpaceKey {
    SpaceKey::from_bytes(*b"a fixture space key, 32 bytes!!!")
}

/// `src/inventory.rs` in the test repository, line by line.
pub const INVENTORY: [&str; 14] = [
    "pub fn count() -> u32 {",
    "    let items = 0;",
    "    items",
    "}",
    "",
    "pub fn reserve() -> u32 {",
    "    let reserved = 1;",
    "    reserved",
    "}",
    "",
    "pub fn release() -> u32 {",
    "    let released = 2;",
    "    released",
    "}",
];

/// The test repository's files.
pub fn files() -> Vec<(&'static str, String)> {
    vec![
        (".gitignore", "/target\n".into()),
        ("src/inventory.rs", inventory(&[])),
        ("src/warehouse.rs", "pub struct Warehouse;\n".into()),
        ("docs/guide.md", "# Guide\n\nRead the code.\n".into()),
    ]
}

/// `src/inventory.rs` with `top` on top of it and each `(line, text)` of `edits`, 1-based, in
/// place of its line; a text with line breaks stands for several lines, an empty one for none.
pub fn inventory_with(top: &[&str], edits: &[(usize, &str)]) -> String {
    let mut lines: Vec<String> = INVENTORY.iter().map(|line| format!("{line}\n")).collect();
    for &(line, text) in edits {
        lines[line - 1] = text.lines().map(|line| format!("{line}\n")).collect();
    }
    let top: String = top.iter().map(|line| format!("{line}\n")).collect();
    top + &lines.concat()
}

pub fn inventory(edits: &[(usize, &str)]) -> String {
    inventory_with(&[], edits)
}

/// A test repository; each machine is a clone of it.
pub struct Repo {
    pub upstream: Upstream,
}

impl Repo {
    /// A repository with `files`, each a path and its contents, in the directory `name`.
    pub fn new(tmp: &TempDir, name: &str, files: &[(&str, &str)]) -> Self {
        let upstream = Upstream::create(&tmp.path().join(name), files).unwrap();
        Self { upstream }
    }

    /// The test repository.
    pub fn create(tmp: &TempDir) -> Self {
        Self::create_with(tmp, &[])
    }

    /// The test repository with `extra` files, each a path and its contents.
    pub fn create_with(tmp: &TempDir, extra: &[(&str, &str)]) -> Self {
        let files = files();
        let files: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, text)| (*path, text.as_str()))
            .chain(extra.iter().copied())
            .collect();
        Self::new(tmp, "repo", &files)
    }

    /// A clone of the repository, as the machine `name` of the space with the key [`key`].
    pub fn machine(&self, name: &str) -> (Machine, Client) {
        self.machine_with_key(name, key())
    }

    pub fn machine_with_key(&self, name: &str, key: SpaceKey) -> (Machine, Client) {
        (
            self.upstream.clone_as(name).unwrap(),
            Client::new(name, key),
        )
    }
}

/// A space: the server stand-in, the messages between machines, its machines, and their latest
/// edit sets.
pub struct Space<'a> {
    pub server: Server,
    pub messages: Messages,
    pub machines: Vec<(&'a Machine, &'a Client)>,
    pub edits: Vec<Edits>,
}

impl<'a> Space<'a> {
    pub fn new(machines: &[(&'a Machine, &'a Client)]) -> Self {
        Self {
            server: Server::default(),
            messages: Messages::default(),
            machines: machines.to_vec(),
            edits: Vec::new(),
        }
    }

    /// Each machine publishes its edit set, in turn; returns the edit sets.
    pub fn publish(&mut self) -> Vec<Edits> {
        self.edits = self
            .machines
            .iter()
            .map(|(machine, client)| client.edits(machine.dir()).unwrap())
            .collect();
        for edit in &self.edits {
            self.server.publish(edit.publication.clone());
        }
        self.edits.clone()
    }

    /// The latest edit set of the machine `name`.
    pub fn edits_of(&self, name: &str) -> &Edits {
        self.edits
            .iter()
            .find(|edits| edits.publication.machine == name)
            .unwrap()
    }

    pub fn find(&self, name: &str) -> (&'a Machine, &'a Client) {
        *self
            .machines
            .iter()
            .find(|(_, client)| client.machine == name)
            .unwrap()
    }
}

/// Paths with ranges of lines, `[(path, [lines])]`.
pub fn lines(files: &[(&str, &[Range<usize>])]) -> Lines {
    files
        .iter()
        .map(|(path, ranges)| (path.to_string(), ranges.to_vec()))
        .collect()
}

/// The bytes of `publications`, each a machine and its fingerprints, one after another.
pub fn bytes<'p>(
    publications: impl IntoIterator<Item = (&'p MachineId, &'p BTreeSet<Fingerprint>)>,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (machine, fingerprints) in publications {
        bytes.extend_from_slice(machine.as_bytes());
        for fingerprint in fingerprints {
            bytes.extend_from_slice(&fingerprint.0);
        }
    }
    bytes
}

/// Whether `bytes` holds `needle`.
pub fn holds(bytes: &[u8], needle: &str) -> bool {
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}
