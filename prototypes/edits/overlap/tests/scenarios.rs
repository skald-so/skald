//! The scenarios of `docs/dev/edit-prototype.md`, "Scenarios": machines editing clones of a test
//! repository publish fingerprints of their edit sets to the server stand-in.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{Repo, Space, bytes, holds};
use edit_overlap::{Edits, Publication, SpaceKey};
use tempfile::TempDir;

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

/// Each machine's paths that other machines edit too, by those machines: `[(machine, [(other
/// machine, [path])])]`, machines without overlaps left out.
type Seen = BTreeMap<String, BTreeMap<String, BTreeSet<String>>>;

/// Each machine of `space` publishes its edit set, in turn; returns what each learns.
fn publish(space: &mut Space) -> Seen {
    space.publish();
    space
        .edits
        .iter()
        .map(|edit| {
            let overlaps = space.server.overlaps(&edit.publication.machine);
            (
                edit.publication.machine.clone(),
                edit.overlapping_paths(&overlaps),
            )
        })
        .filter(|(_, overlaps)| !overlaps.is_empty())
        .collect()
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
    let repo = Repo::new(&tmp, "repo", FILES);
    let (machine, client) = repo.machine("alice-laptop");
    let (other, _) = repo.machine("bob-desktop");
    let (maintainer, _) = repo.machine("maintainer");
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
    // One per path, and one per boundary each hunk touches: 2 for each one-line change, 2 for the
    // deleted one-line file, 1 for each added file.
    assert_eq!(edits.publication.fingerprints.len(), 5 + 2 + 2 + 2 + 1 + 1);
    assert_eq!(edits.publication.machine, "alice-laptop");
}

#[test]
fn machines_editing_one_file_overlap() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::new(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    a.write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
        .unwrap();
    a.write("README.md", "# Inventory\n").unwrap();
    a.commit("count one").unwrap();
    b.write("src/inventory.rs", "pub fn count() -> u32 {\n    2\n}\n")
        .unwrap();
    b.write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();

    let seen_now = publish(&mut Space::new(&[(&a, &a_client), (&b, &b_client)]));
    assert_eq!(
        seen_now,
        pairs(&[("alice-laptop", "bob-desktop", &["src/inventory.rs"])])
    );
}

#[test]
fn machines_editing_different_files_do_not_overlap() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::new(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    a.write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
        .unwrap();
    b.write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();
    b.write("src/shipping.rs", "pub struct Shipment;\n")
        .unwrap();

    let seen_now = publish(&mut Space::new(&[(&a, &a_client), (&b, &b_client)]));
    assert_eq!(seen_now, Seen::new());
}

#[test]
fn rename_overlaps_an_edit_of_either_path() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::new(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (c, c_client) = repo.machine("carol-server");
    a.rename("src/warehouse.rs", "src/storage.rs").unwrap();
    a.commit("rename the warehouse").unwrap();
    b.write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();
    c.write("src/storage.rs", "pub struct Storage;\n").unwrap();

    let seen_now = publish(&mut Space::new(&[
        (&a, &a_client),
        (&b, &b_client),
        (&c, &c_client),
    ]));
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
    let repo = Repo::new(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    a.delete("docs/onboarding-guide.md").unwrap();
    b.write("docs/onboarding-guide.md", "Read the docs.\n")
        .unwrap();

    let seen_now = publish(&mut Space::new(&[(&a, &a_client), (&b, &b_client)]));
    assert_eq!(
        seen_now,
        pairs(&[("alice-laptop", "bob-desktop", &["docs/onboarding-guide.md"])])
    );
}

#[test]
fn machine_whose_changes_are_merged_leaves_the_overlap_once_it_pulls() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::new(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (maintainer, _) = repo.machine("maintainer");
    a.write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
        .unwrap();
    a.commit("count one").unwrap();
    a.push("count-one").unwrap();
    b.write("src/inventory.rs", "pub fn count() -> u32 {\n    2\n}\n")
        .unwrap();
    let overlap = pairs(&[("alice-laptop", "bob-desktop", &["src/inventory.rs"])]);
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    assert_eq!(publish(&mut space), overlap);

    // Merged upstream, but the machine does not know it yet.
    maintainer.merge("count-one").unwrap();
    assert_eq!(publish(&mut space), overlap);

    a.pull().unwrap();
    assert_eq!(a_client.edits(a.dir()).unwrap().paths(), BTreeSet::new());
    assert_eq!(publish(&mut space), Seen::new());
}

#[test]
fn same_path_in_another_repository_or_under_another_key_does_not_overlap() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::new(&tmp, "repo", FILES);
    // The same paths; another root commit.
    let mut other_files = FILES.to_vec();
    other_files[1] = ("README.md", "# Billing service\n");
    let other_repo = Repo::new(&tmp, "other-repo", &other_files);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (c, c_client) = other_repo.machine("carol-server");
    let (d, d_client) = repo.machine_with_key("dave-laptop", other_key());
    for machine in [&a, &b, &c, &d] {
        machine
            .write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
            .unwrap();
    }

    let seen_now = publish(&mut Space::new(&[
        (&a, &a_client),
        (&b, &b_client),
        (&c, &c_client),
        (&d, &d_client),
    ]));
    // Only the clones of one repository under one key overlap.
    assert_eq!(
        seen_now,
        pairs(&[("alice-laptop", "bob-desktop", &["src/inventory.rs"])])
    );
}

#[test]
fn server_receives_and_keeps_no_path() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::new(&tmp, "repo", FILES);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    a.write("src/inventory.rs", "pub fn count() -> u32 {\n    1\n}\n")
        .unwrap();
    a.rename("src/warehouse.rs", "src/storage.rs").unwrap();
    a.commit("count one").unwrap();
    a.delete("docs/onboarding-guide.md").unwrap();
    b.write("src/inventory.rs", "pub fn count() -> u32 {\n    2\n}\n")
        .unwrap();
    b.write("notes/todo-list.md", "- ship\n").unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);

    let seen_now = publish(&mut space);
    let edits = &space.edits;
    assert_eq!(
        seen_now,
        pairs(&[("alice-laptop", "bob-desktop", &["src/inventory.rs"])])
    );
    let kept: Vec<Publication> = space
        .server
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
        let bytes = bytes([(&publication.machine, &publication.fingerprints)]);
        for path in paths.union(&names) {
            assert!(
                !holds(&bytes, path),
                "{path} in what {} sent",
                publication.machine
            );
        }
    }
}
