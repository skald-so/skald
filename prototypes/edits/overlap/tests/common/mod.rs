//! What the scenario tests share: the space key, the test repositories and a space of machines.
#![allow(dead_code, reason = "each test file uses some of the helpers")]

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use edit_overlap::harness::{Machine, Upstream};
use edit_overlap::{
    Check, Client, Edits, Fingerprint, Lines, MachineId, Messages, Server, SpaceKey, Symbols,
};
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

    /// `name` asks each machine the server says it overlaps for its changes, they answer, and it
    /// checks them: what it finds, by machine.
    pub fn check(&mut self, name: &str) -> BTreeMap<String, Check> {
        let (machine, client) = self.find(name);
        for overlap in self.server.overlaps(name) {
            client
                .request_changes(machine.dir(), &overlap.machine, &mut self.messages)
                .unwrap();
            let (other, other_client) = self.find(&overlap.machine);
            other_client
                .answer_requests(other.dir(), &mut self.messages)
                .unwrap();
        }
        let checks = client.check(machine.dir(), &mut self.messages).unwrap();
        assert!(
            self.messages.waiting().values().all(Vec::is_empty),
            "every message is answered"
        );
        checks
    }
}

/// Two machines, each with the other.
pub const PAIRS: [(&str, &str); 2] = [
    ("alice-laptop", "bob-desktop"),
    ("bob-desktop", "alice-laptop"),
];

impl Space<'_> {
    /// The bytes of everything the server has received, as [`bytes`] gives them.
    pub fn received_bytes(&self) -> Vec<u8> {
        let received = self.server.received().iter();
        bytes(received.map(|publication| (&publication.machine, &publication.fingerprints)))
    }
}

/// What a machine learns from the server, by the other machine.
#[derive(Debug, PartialEq, Eq)]
pub struct Seen {
    /// Its paths that the other machine edits too.
    pub paths: BTreeMap<String, BTreeSet<String>>,
    /// Its changed lines whose hunks overlap the other machine's.
    pub hunks: BTreeMap<String, Lines>,
    /// Its changed symbols that the other machine changes too, each as `<kind> <name>` of it and
    /// the symbols that enclose it, joined by ` > `.
    pub symbols: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
}

impl Space<'_> {
    /// What the machine `name` learns from the server.
    pub fn seen(&self, name: &str) -> Seen {
        let edits = self.edits_of(name);
        let overlaps = self.server.overlaps(name);
        Seen {
            paths: edits.overlapping_paths(&overlaps),
            hunks: edits.overlapping_hunks(&overlaps),
            symbols: edits
                .overlapping_symbols(&overlaps)
                .into_iter()
                .map(|(machine, symbols)| (machine, names(&symbols)))
                .collect(),
        }
    }
}

/// Each path of `symbols` with its symbols as text, as [`Seen::symbols`].
pub fn names(symbols: &Symbols) -> BTreeMap<String, BTreeSet<String>> {
    symbols
        .iter()
        .map(|(path, symbols)| {
            let names = symbols.iter().map(ToString::to_string).collect();
            (path.clone(), names)
        })
        .collect()
}

/// Each other machine with the same paths, `[(machine, [path])]`.
pub fn paths(others: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    others
        .iter()
        .map(|(machine, paths)| {
            let paths = paths.iter().map(|path| path.to_string()).collect();
            (machine.to_string(), paths)
        })
        .collect()
}

/// Each other machine with the changed lines, `[(machine, lines)]`.
pub fn hunks(others: &[(&str, Lines)]) -> BTreeMap<String, Lines> {
    others
        .iter()
        .map(|(machine, lines)| (machine.to_string(), lines.clone()))
        .collect()
}

/// Each other machine with the changed symbols of a path, `[(machine, path, [symbol])]`.
pub fn symbols(
    others: &[(&str, &str, &[&str])],
) -> BTreeMap<String, BTreeMap<String, BTreeSet<String>>> {
    let mut seen = BTreeMap::<String, BTreeMap<String, BTreeSet<String>>>::new();
    for (machine, path, symbols) in others {
        let names = symbols.iter().map(|symbol| symbol.to_string()).collect();
        seen.entry(machine.to_string())
            .or_default()
            .insert(path.to_string(), names);
    }
    seen
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

/// The lines of `texts` without the spaces around them, each long enough not to occur in random
/// bytes by chance.
pub fn long_lines<'t>(texts: impl IntoIterator<Item = &'t str>) -> BTreeSet<&'t str> {
    texts
        .into_iter()
        .flat_map(str::lines)
        .map(str::trim)
        .filter(|line| line.len() >= 4)
        .collect()
}

/// Panics if `bytes`, which stand for `what`, hold any of `needles`.
pub fn assert_holds_none<'n>(bytes: &[u8], needles: impl IntoIterator<Item = &'n str>, what: &str) {
    for needle in needles {
        let found = bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes());
        assert!(!found, "{needle} in {what}");
    }
}
