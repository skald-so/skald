//! The scenarios of `docs/dev/edit-prototype.md`, "Scenarios": machines editing clones of a test
//! repository publish fingerprints of their edit sets to the server stand-in.

use std::collections::{BTreeMap, BTreeSet};

use edit_overlap::harness::{Machine, Upstream};
use edit_overlap::{Client, Edits, Publication, Server, SpaceKey};
use tempfile::TempDir;

/// The space key, a test fixture.
fn key() -> SpaceKey {
    SpaceKey::from_bytes(*b"a fixture space key, 32 bytes!!!")
}

/// The key of another space.
fn other_key() -> SpaceKey {
    SpaceKey::from_bytes(*b"another space's key, 32 bytes!!!")
}

/// The test repository's files.
const FILES: &[(&str, &str)] = &[
    (".gitignore", "/target\n"),
    ("README.md", "# Inventory service\n"),
    ("src/inventory.rs", "pub fn count() -> u32 {\n    0\n}\n"),
    ("src/warehouse.rs", "pub struct Warehouse;\n"),
    ("docs/onboarding-guide.md", "Read the code.\n"),
];

/// A test repository; each machine is a clone of it.
struct Repo {
    upstream: Upstream,
}

impl Repo {
    fn create(tmp: &TempDir, name: &str, files: &[(&str, &str)]) -> Self {
        let upstream = Upstream::create(&tmp.path().join(name), files).unwrap();
        Self { upstream }
    }

    fn machine(&self, name: &str, key: SpaceKey) -> (Machine, Client) {
        (
            self.upstream.clone_as(name).unwrap(),
            Client::new(name, key),
        )
    }
}

/// Each machine's paths that other machines edit too, by those machines: `[(machine, [(other
/// machine, [path])])]`, machines without overlaps left out.
type Seen = BTreeMap<String, BTreeMap<String, BTreeSet<String>>>;

/// The machines publish their edit sets in turn; returns what each learns, and their edit sets.
fn publish(server: &mut Server, machines: &[(&Machine, &Client)]) -> (Seen, Vec<Edits>) {
    let edits: Vec<Edits> = machines
        .iter()
        .map(|(machine, client)| client.edits(machine.dir()).unwrap())
        .collect();
    for edit in &edits {
        server.publish(edit.publication.clone());
    }
    let seen = edits
        .iter()
        .map(|edit| {
            let overlaps = server.overlaps(&edit.publication.machine);
            (
                edit.publication.machine.clone(),
                edit.overlapping_paths(&overlaps),
            )
        })
        .filter(|(_, overlaps)| !overlaps.is_empty())
        .collect();
    (seen, edits)
}

/// What machines that overlap in pairs see: in each pair, `(machine, other machine, paths)`, each
/// sees the other with the paths.
fn pairs(pairs: &[(&str, &str, &[&str])]) -> Seen {
    let mut seen = Seen::new();
    for (a, b, paths) in pairs {
        let paths: BTreeSet<String> = paths.iter().map(|path| path.to_string()).collect();
        for (machine, other) in [(a, b), (b, a)] {
            let overlaps = seen.entry(machine.to_string()).or_default();
            overlaps.insert(other.to_string(), paths.clone());
        }
    }
    seen
}

#[test]
fn edit_set_holds_committed_uncommitted_and_untracked_changes() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp, "repo", FILES);
    let (machine, client) = repo.machine("alice-laptop", key());
    let (other, _) = repo.machine("bob-desktop", key());
    let (maintainer, _) = repo.machine("maintainer", key());
    // Another machine's change merged upstream and fetched, but not merged here, is not an edit.
    other.write("README.md", "# Inventory\n").unwrap();
    other.commit("rename the service").unwrap();
    other.push("readme").unwrap();
    maintainer.merge("readme").unwrap();
    machine.git(&["fetch", "--quiet"]).unwrap();

    machine
        .write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
        .unwrap();
    machine.commit("count one").unwrap();
    machine
        .write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();
    machine.git(&["add", "src/warehouse.rs"]).unwrap();
    machine
        .write("src/warehouse.rs", "pub struct Warehouse(u64);\n")
        .unwrap();
    machine
        .rename("docs/onboarding-guide.md", "docs/guide.md")
        .unwrap();
    machine.write("notes/todo-list.md", "- ship\n").unwrap();
    machine
        .write("target/build-output.bin", "ignored\n")
        .unwrap();

    let edits = client.edits(machine.dir()).unwrap();
    assert_eq!(
        edits.paths(),
        BTreeSet::from([
            "docs/guide.md",
            "docs/onboarding-guide.md",
            "notes/todo-list.md",
            "src/inventory.rs",
            "src/warehouse.rs",
        ])
    );
    assert_eq!(edits.publication.fingerprints.len(), 5);
    assert_eq!(edits.publication.machine, "alice-laptop");
}

#[test]
fn machines_editing_one_file_overlap() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop", key());
    let (b, b_client) = repo.machine("bob-desktop", key());
    a.write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
        .unwrap();
    a.write("README.md", "# Inventory\n").unwrap();
    a.commit("count one").unwrap();
    b.write("src/inventory.rs", "pub fn count() -> u32 {\n    2\n}\n")
        .unwrap();
    b.write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();

    let (seen_now, _) = publish(&mut Server::default(), &[(&a, &a_client), (&b, &b_client)]);
    assert_eq!(
        seen_now,
        pairs(&[("alice-laptop", "bob-desktop", &["src/inventory.rs"])])
    );
}

#[test]
fn machines_editing_different_files_do_not_overlap() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop", key());
    let (b, b_client) = repo.machine("bob-desktop", key());
    a.write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
        .unwrap();
    b.write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();
    b.write("src/shipping.rs", "pub struct Shipment;\n")
        .unwrap();

    let (seen_now, _) = publish(&mut Server::default(), &[(&a, &a_client), (&b, &b_client)]);
    assert_eq!(seen_now, Seen::new());
}

#[test]
fn rename_overlaps_an_edit_of_either_path() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop", key());
    let (b, b_client) = repo.machine("bob-desktop", key());
    let (c, c_client) = repo.machine("carol-server", key());
    a.rename("src/warehouse.rs", "src/storage.rs").unwrap();
    a.commit("rename the warehouse").unwrap();
    b.write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();
    c.write("src/storage.rs", "pub struct Storage;\n").unwrap();

    let (seen_now, _) = publish(
        &mut Server::default(),
        &[(&a, &a_client), (&b, &b_client), (&c, &c_client)],
    );
    assert_eq!(
        seen_now,
        pairs(&[
            ("alice-laptop", "bob-desktop", &["src/warehouse.rs"]),
            ("alice-laptop", "carol-server", &["src/storage.rs"]),
        ])
    );
}

#[test]
fn deletion_overlaps_an_edit_of_its_path() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop", key());
    let (b, b_client) = repo.machine("bob-desktop", key());
    a.delete("docs/onboarding-guide.md").unwrap();
    b.write("docs/onboarding-guide.md", "Read the docs.\n")
        .unwrap();

    let (seen_now, _) = publish(&mut Server::default(), &[(&a, &a_client), (&b, &b_client)]);
    assert_eq!(
        seen_now,
        pairs(&[("alice-laptop", "bob-desktop", &["docs/onboarding-guide.md"])])
    );
}

#[test]
fn machine_whose_changes_are_merged_leaves_the_overlap_once_it_pulls() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop", key());
    let (b, b_client) = repo.machine("bob-desktop", key());
    let (maintainer, _) = repo.machine("maintainer", key());
    let mut server = Server::default();
    a.write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
        .unwrap();
    a.commit("count one").unwrap();
    a.push("count-one").unwrap();
    b.write("src/inventory.rs", "pub fn count() -> u32 {\n    2\n}\n")
        .unwrap();
    let overlap = pairs(&[("alice-laptop", "bob-desktop", &["src/inventory.rs"])]);
    let machines = [(&a, &a_client), (&b, &b_client)];
    assert_eq!(publish(&mut server, &machines).0, overlap);

    // Merged upstream, but the machine does not know it yet.
    maintainer.merge("count-one").unwrap();
    assert_eq!(publish(&mut server, &machines).0, overlap);

    a.pull().unwrap();
    assert_eq!(a_client.edits(a.dir()).unwrap().paths(), BTreeSet::new());
    assert_eq!(publish(&mut server, &machines).0, Seen::new());
}

#[test]
fn same_path_in_another_repository_or_under_another_key_does_not_overlap() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp, "repo", FILES);
    // The same paths; another root commit.
    let mut other_files = FILES.to_vec();
    other_files[1] = ("README.md", "# Billing service\n");
    let other_repo = Repo::create(&tmp, "other-repo", &other_files);
    let (a, a_client) = repo.machine("alice-laptop", key());
    let (b, b_client) = repo.machine("bob-desktop", key());
    let (c, c_client) = other_repo.machine("carol-server", key());
    let (d, d_client) = repo.machine("dave-laptop", other_key());
    for machine in [&a, &b, &c, &d] {
        machine
            .write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
            .unwrap();
    }

    let (seen_now, _) = publish(
        &mut Server::default(),
        &[
            (&a, &a_client),
            (&b, &b_client),
            (&c, &c_client),
            (&d, &d_client),
        ],
    );
    // Only the clones of one repository under one key overlap.
    assert_eq!(
        seen_now,
        pairs(&[("alice-laptop", "bob-desktop", &["src/inventory.rs"])])
    );
}

#[test]
fn server_receives_and_keeps_no_path() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop", key());
    let (b, b_client) = repo.machine("bob-desktop", key());
    a.write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
        .unwrap();
    a.rename("src/warehouse.rs", "src/storage.rs").unwrap();
    a.commit("count one").unwrap();
    a.delete("docs/onboarding-guide.md").unwrap();
    b.write("src/inventory.rs", "pub fn count() -> u32 {\n    2\n}\n")
        .unwrap();
    b.write("notes/todo-list.md", "- ship\n").unwrap();
    let mut server = Server::default();

    let (seen_now, edits) = publish(&mut server, &[(&a, &a_client), (&b, &b_client)]);
    assert_eq!(
        seen_now,
        pairs(&[("alice-laptop", "bob-desktop", &["src/inventory.rs"])])
    );
    let kept: Vec<Publication> = server
        .kept()
        .iter()
        .map(|(machine, fingerprints)| Publication {
            machine: machine.clone(),
            fingerprints: fingerprints.clone(),
        })
        .collect();
    let sent: Vec<Publication> = edits.iter().map(|edit| edit.publication.clone()).collect();
    assert_eq!(kept, sent);

    // Every path of the scenario, and each of its file and directory names.
    let mut paths: BTreeSet<&str> = FILES.iter().map(|(path, _)| *path).collect();
    paths.extend(edits.iter().flat_map(Edits::paths));
    assert!(paths.contains("src/storage.rs") && paths.contains("notes/todo-list.md"));
    let names: BTreeSet<&str> = paths.iter().flat_map(|path| path.split('/')).collect();
    for publication in &sent {
        let mut bytes = publication.machine.as_bytes().to_vec();
        for fingerprint in &publication.fingerprints {
            bytes.extend_from_slice(&fingerprint.0);
        }
        for path in paths.union(&names) {
            let found = bytes
                .windows(path.len())
                .any(|window| window == path.as_bytes());
            assert!(!found, "{path} in what {} sent", publication.machine);
        }
    }
}
