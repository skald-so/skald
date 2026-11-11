//! The exact check scenarios of `docs/dev/edit-prototype.md`, "Scenarios": on an overlap the
//! server reports, a machine gets the other machine's changes over end-to-end encrypted messages
//! and merges them into its own with `git merge-tree`.
#![allow(
    clippy::single_range_in_vec_init,
    reason = "a check's ranges of lines are lists of ranges"
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use common::{INVENTORY, Repo, Space, bytes, holds, inventory, inventory_with, lines};
use edit_overlap::harness::Machine;
use edit_overlap::{Check, Conflict, Edits, Publication, git};
use tempfile::TempDir;

impl Space<'_> {
    /// `name` asks each machine the server says it overlaps for its changes, they answer, and it
    /// checks them: what it finds, by machine.
    fn check(&mut self, name: &str) -> BTreeMap<String, Check> {
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

/// A check that found the conflicts `conflicts`, each a type and paths, in the files `files`, each
/// a path and the conflicting ranges of the checking machine's lines.
fn conflicted(conflicts: &[(&str, &[&str])], files: &[(&str, &[Range<usize>])]) -> Check {
    Check {
        clean: false,
        conflicts: conflicts
            .iter()
            .map(|(kind, paths)| Conflict {
                kind: kind.to_string(),
                paths: paths.iter().map(|path| path.to_string()).collect(),
            })
            .collect(),
        files: lines(files),
    }
}

fn clean() -> Check {
    Check {
        clean: true,
        conflicts: Vec::new(),
        files: BTreeMap::new(),
    }
}

#[test]
fn snapshot_commits_the_working_tree_and_leaves_the_repository_as_it_is() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (machine, _) = repo.machine("alice-laptop");
    machine
        .write("src/inventory.rs", &inventory(&[(2, "    let items = 1;")]))
        .unwrap();
    machine.commit("count one").unwrap();
    machine
        .write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();
    machine.git(&["add", "src/warehouse.rs"]).unwrap();
    machine
        .write("src/warehouse.rs", "pub struct Warehouse(u64);\n")
        .unwrap();
    machine.delete("docs/guide.md").unwrap();
    machine.write("notes/todo-list.md", "- ship\n").unwrap();
    machine
        .write("target/build-output.bin", "ignored\n")
        .unwrap();
    let state = |machine: &Machine| {
        let git_dir: BTreeSet<_> = std::fs::read_dir(machine.dir().join(".git"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        (
            machine
                .git(&["status", "--porcelain", "--untracked-files=all"])
                .unwrap(),
            machine.git(&["diff", "--cached"]).unwrap(),
            machine.git(&["for-each-ref"]).unwrap(),
            git_dir,
        )
    };
    let before = state(&machine);

    let snapshot = git::snapshot(machine.dir()).unwrap();

    assert_eq!(state(&machine), before);
    let parent = machine
        .git(&["rev-parse", &format!("{snapshot}^")])
        .unwrap();
    assert_eq!(parent, machine.git(&["rev-parse", "HEAD"]).unwrap());
    let paths = machine
        .git(&["ls-tree", "-r", "--name-only", &snapshot])
        .unwrap();
    assert_eq!(
        String::from_utf8(paths).unwrap(),
        ".gitignore\nnotes/todo-list.md\nsrc/inventory.rs\nsrc/warehouse.rs\n"
    );
    let show = |path: &str| {
        let text = machine
            .git(&["show", &format!("{snapshot}:{path}")])
            .unwrap();
        String::from_utf8(text).unwrap()
    };
    assert_eq!(
        show("src/inventory.rs"),
        inventory(&[(2, "    let items = 1;")])
    );
    assert_eq!(show("src/warehouse.rs"), "pub struct Warehouse(u64);\n");
    assert_eq!(show("notes/todo-list.md"), "- ship\n");
}

#[test]
fn edits_to_the_same_lines_conflict_with_their_files_and_lines() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    // Committed on one machine; uncommitted and untracked on the other, with a clean change above
    // the conflict that shifts its lines.
    a.write(
        "src/inventory.rs",
        &inventory(&[(7, "    let reserved = 10;")]),
    )
    .unwrap();
    a.write("notes/plan.md", "- count\n- reserve\n").unwrap();
    a.commit("reserve ten").unwrap();
    b.write(
        "src/inventory.rs",
        &inventory_with(
            &["//! Inventory.", "", ""],
            &[(7, "    let reserved = 20;")],
        ),
    )
    .unwrap();
    b.write("notes/plan.md", "- count\n- release\n").unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    let conflicts: &[(&str, &[&str])] = &[
        // A file both sides add: `git merge-tree` types it as a content conflict.
        ("CONFLICT (contents)", &["notes/plan.md"]),
        ("CONFLICT (contents)", &["src/inventory.rs"]),
    ];
    assert_eq!(
        space.check("alice-laptop"),
        BTreeMap::from([(
            "bob-desktop".into(),
            conflicted(
                conflicts,
                &[("notes/plan.md", &[2..3]), ("src/inventory.rs", &[7..8])]
            )
        )])
    );
    assert_eq!(
        space.check("bob-desktop"),
        BTreeMap::from([(
            "alice-laptop".into(),
            conflicted(
                conflicts,
                &[("notes/plan.md", &[2..3]), ("src/inventory.rs", &[10..11])]
            )
        )])
    );
}

#[test]
fn conflicting_lines_are_found_whatever_the_repository_attributes_and_config() {
    let tmp = TempDir::new().unwrap();
    // Markers of 32 characters in Rust files, as git's own repository sets for some files, and
    // of 7 in the others; `-diff` makes `git diff` show any file as binary, without hunks.
    let repo = Repo::create_with(
        &tmp,
        &[(
            ".gitattributes",
            "* -diff\nsrc/*.rs conflict-marker-size=32\n",
        )],
    );
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    // `git diff` joins hunks up to 3 lines apart into one, and colors its output.
    a.git(&["config", "diff.interHunkContext", "3"]).unwrap();
    a.git(&["config", "color.diff", "always"]).unwrap();
    a.write(
        "src/inventory.rs",
        &inventory(&[(7, "    let reserved = 10;")]),
    )
    .unwrap();
    a.write("notes/plan.md", "- count\n- reserve\n").unwrap();
    a.commit("reserve ten").unwrap();
    // A line added two lines above the conflict and one changed two lines below it merge cleanly.
    let mut lines = INVENTORY.to_vec();
    lines[6] = "    let reserved = 20;";
    lines[8] = "} // reserve";
    lines.insert(5, "/// Reserves one item.");
    b.write("src/inventory.rs", &(lines.join("\n") + "\n"))
        .unwrap();
    b.write("notes/plan.md", "- count\n- release\n").unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    let conflicts: &[(&str, &[&str])] = &[
        ("CONFLICT (contents)", &["notes/plan.md"]),
        ("CONFLICT (contents)", &["src/inventory.rs"]),
    ];
    assert_eq!(
        space.check("alice-laptop"),
        BTreeMap::from([(
            "bob-desktop".into(),
            conflicted(
                conflicts,
                &[("notes/plan.md", &[2..3]), ("src/inventory.rs", &[7..8])]
            )
        )])
    );
    assert_eq!(
        space.check("bob-desktop"),
        BTreeMap::from([(
            "alice-laptop".into(),
            conflicted(
                conflicts,
                &[("notes/plan.md", &[2..3]), ("src/inventory.rs", &[8..9])]
            )
        )])
    );
}

#[test]
fn conflicting_lines_are_found_after_an_unstaged_deletion_of_the_attributes() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create_with(
        &tmp,
        &[(".gitattributes", "src/*.rs conflict-marker-size=32\n")],
    );
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    // `git check-attr` still reads the deleted file from the index; `git merge-tree` reads the
    // working tree only, so it writes markers of 7 characters.
    a.delete(".gitattributes").unwrap();
    a.write(
        "src/inventory.rs",
        &inventory(&[(7, "    let reserved = 10;")]),
    )
    .unwrap();
    b.write(
        "src/inventory.rs",
        &inventory(&[(7, "    let reserved = 20;")]),
    )
    .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    assert_eq!(
        space.check("alice-laptop"),
        BTreeMap::from([(
            "bob-desktop".into(),
            conflicted(
                &[("CONFLICT (contents)", &["src/inventory.rs"])],
                &[("src/inventory.rs", &[7..8])]
            )
        )])
    );
}

#[test]
fn edits_to_distant_lines_of_one_file_merge_cleanly() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    a.write("src/inventory.rs", &inventory(&[(2, "    let items = 1;")]))
        .unwrap();
    a.commit("count one").unwrap();
    b.write(
        "src/inventory.rs",
        &inventory(&[(12, "    let released = 3;")]),
    )
    .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    // The server reports the overlap; the check clears it.
    assert_eq!(
        space.check("alice-laptop"),
        BTreeMap::from([("bob-desktop".into(), clean())])
    );
    assert_eq!(
        space.check("bob-desktop"),
        BTreeMap::from([("alice-laptop".into(), clean())])
    );
}

#[test]
fn modify_delete_conflict_is_found() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    a.delete("src/warehouse.rs").unwrap();
    b.write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    let conflict = conflicted(
        &[("CONFLICT (modify/delete)", &["src/warehouse.rs"])],
        &[("src/warehouse.rs", &[])],
    );
    assert_eq!(
        space.check("alice-laptop"),
        BTreeMap::from([("bob-desktop".into(), conflict.clone())])
    );
    assert_eq!(
        space.check("bob-desktop"),
        BTreeMap::from([("alice-laptop".into(), conflict)])
    );
}

#[test]
fn rename_conflicts_are_found() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (c, c_client) = repo.machine("carol-server");
    let (d, d_client) = repo.machine("dave-laptop");
    a.rename("docs/guide.md", "docs/handbook.md").unwrap();
    a.commit("rename the guide").unwrap();
    b.rename("docs/guide.md", "docs/manual.md").unwrap();
    c.delete("docs/guide.md").unwrap();
    // An edit of the old path follows the rename.
    d.write("docs/guide.md", "# Guide\n\nRead the docs.\n")
        .unwrap();
    let mut space = Space::new(&[
        (&a, &a_client),
        (&b, &b_client),
        (&c, &c_client),
        (&d, &d_client),
    ]);
    space.publish();

    assert_eq!(
        space.check("alice-laptop"),
        BTreeMap::from([
            (
                "bob-desktop".into(),
                conflicted(
                    &[(
                        "CONFLICT (rename/rename)",
                        &["docs/guide.md", "docs/handbook.md", "docs/manual.md"]
                    )],
                    &[
                        ("docs/guide.md", &[]),
                        ("docs/handbook.md", &[]),
                        ("docs/manual.md", &[])
                    ]
                )
            ),
            (
                "carol-server".into(),
                conflicted(
                    &[(
                        "CONFLICT (rename/delete)",
                        &["docs/handbook.md", "docs/guide.md"]
                    )],
                    &[("docs/handbook.md", &[])]
                )
            ),
            ("dave-laptop".into(), clean()),
        ])
    );
}

#[test]
fn machines_on_different_bases_get_the_right_result() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (maintainer, _) = repo.machine("maintainer");
    // Bob's change to line 2 is merged upstream, after lines added on top; Alice pulls both and
    // changes line 2 again, Bob does not pull.
    b.write("src/inventory.rs", &inventory(&[(2, "    let items = 1;")]))
        .unwrap();
    b.commit("count one").unwrap();
    b.push("count-one").unwrap();
    maintainer
        .write(
            "src/inventory.rs",
            &inventory_with(&["//! Inventory.", "", ""], &[]),
        )
        .unwrap();
    maintainer.commit("document the inventory").unwrap();
    maintainer
        .git(&["push", "--quiet", "origin", "HEAD:main"])
        .unwrap();
    maintainer.merge("count-one").unwrap();
    a.pull().unwrap();
    let merged = ["//! Inventory.", "", ""];
    a.write(
        "src/inventory.rs",
        &inventory_with(
            &merged,
            &[(2, "    let items = 2;"), (7, "    let reserved = 10;")],
        ),
    )
    .unwrap();
    // Both edit line 7; Bob also edits line 12.
    let b_edits = [
        (2, "    let items = 1;"),
        (7, "    let reserved = 20;"),
        (12, "    let released = 3;"),
    ];
    b.write("src/inventory.rs", &inventory(&b_edits)).unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();
    assert!(git::has_commit(a.dir(), &git::merge_base(b.dir()).unwrap()).unwrap());
    assert!(!git::has_commit(b.dir(), &git::merge_base(a.dir()).unwrap()).unwrap());

    let conflicts: &[(&str, &[&str])] = &[("CONFLICT (contents)", &["src/inventory.rs"])];
    assert_eq!(
        space.check("alice-laptop"),
        BTreeMap::from([(
            "bob-desktop".into(),
            conflicted(conflicts, &[("src/inventory.rs", &[10..11])])
        )])
    );
    assert_eq!(
        space.check("bob-desktop"),
        BTreeMap::from([(
            "alice-laptop".into(),
            conflicted(conflicts, &[("src/inventory.rs", &[7..8])])
        )])
    );

    // Without the edit of line 7, the bases' lines on top and Bob's merged change merge cleanly.
    b.write("src/inventory.rs", &inventory(&[b_edits[0], b_edits[2]]))
        .unwrap();
    space.publish();
    assert_eq!(
        space.check("alice-laptop"),
        BTreeMap::from([("bob-desktop".into(), clean())])
    );
    assert_eq!(
        space.check("bob-desktop"),
        BTreeMap::from([("alice-laptop".into(), clean())])
    );
}

#[test]
fn a_request_with_a_base_that_is_not_an_object_id_is_refused() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, _) = repo.machine("alice-laptop");
    let snapshot = git::snapshot(a.dir()).unwrap();
    let error = git::bundle(a.dir(), &snapshot, "--all").unwrap_err();
    assert_eq!(error.to_string(), "not an object ID: \"--all\"");
}

/// A bundle of `machine`'s `HEAD` in the references `refs`, in their order, without the
/// upstream's default branch.
fn bundle_of(machine: &Machine, refs: &[&str]) -> Vec<u8> {
    machine
        .write("src/warehouse.rs", "pub struct Warehouse(u32);\n")
        .unwrap();
    machine.commit("count").unwrap();
    for name in refs {
        machine.git(&["update-ref", name, "HEAD"]).unwrap();
    }
    let without = format!("^{}", git::UPSTREAM_HEAD);
    let args: Vec<&str> = ["bundle", "create", "--quiet", "-"]
        .into_iter()
        .chain(refs.iter().copied())
        .chain([without.as_str()])
        .collect();
    machine.git(&args).unwrap()
}

#[test]
fn a_bundle_with_another_reference_is_refused() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, _) = repo.machine("alice-laptop");
    let (b, _) = repo.machine("bob-desktop");
    let bundle = bundle_of(&a, &["refs/heads/main"]);
    let error = git::unbundle(b.dir(), &bundle).unwrap_err().to_string();
    assert!(
        error.starts_with("a bundle with another reference: ")
            && error.ends_with(" refs/heads/main"),
        "{error}"
    );
}

#[test]
fn a_bundle_with_two_references_is_refused() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, _) = repo.machine("alice-laptop");
    let (b, _) = repo.machine("bob-desktop");
    // The bundle's own reference first.
    let bundle = bundle_of(&a, &[git::BUNDLE_REF, "refs/heads/main"]);
    let error = git::unbundle(b.dir(), &bundle).unwrap_err().to_string();
    assert!(
        error.starts_with("a bundle without exactly one reference: "),
        "{error}"
    );
}

#[test]
fn server_receives_and_keeps_nothing_of_the_exchanged_changes() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let a_text = inventory(&[(7, "    let reserved = 10;")]);
    let b_text = inventory(&[(7, "    let reserved = 20;"), (12, "    let released = 3;")]);
    a.write("src/inventory.rs", &a_text).unwrap();
    a.commit("reserve ten").unwrap();
    b.write("src/inventory.rs", &b_text).unwrap();
    b.write("notes/todo-list.md", "- ship the reservations\n")
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    let edits = space.publish();
    let received = space.server.received().to_vec();
    let kept = space.server.kept().clone();

    let checks = [space.check("alice-laptop"), space.check("bob-desktop")];
    assert!(
        checks
            .iter()
            .all(|check| check.values().all(|check| !check.clean))
    );

    assert_eq!(space.server.received(), received);
    assert_eq!(space.server.kept(), &kept);
    let sent: Vec<Publication> = edits.iter().map(|edit| edit.publication.clone()).collect();
    assert_eq!(received, sent);
    // Every path of the exchanged changes, its file and directory names, and every line of them
    // long enough not to occur in random bytes by chance.
    let mut needles: BTreeSet<&str> = edits.iter().flat_map(Edits::paths).collect();
    assert!(needles.contains("notes/todo-list.md"));
    needles.extend(needles.clone().iter().flat_map(|path| path.split('/')));
    for text in [
        a_text.as_str(),
        b_text.as_str(),
        "- ship the reservations\n",
    ] {
        needles.extend(text.lines().map(str::trim).filter(|line| line.len() >= 4));
    }
    assert!(needles.contains("let reserved = 20;"));
    let received = received
        .iter()
        .map(|publication| (&publication.machine, &publication.fingerprints));
    let bytes = bytes(received.chain(&kept));
    for needle in needles {
        assert!(
            !holds(&bytes, needle),
            "{needle} in what the server received or keeps"
        );
    }
}
