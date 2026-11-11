//! The hunk fingerprint scenarios of `docs/dev/edit-prototype.md`, "Scenarios": machines also
//! publish fingerprints of their changed hunks, and those match where `git merge-tree` conflicts.
#![allow(
    clippy::single_range_in_vec_init,
    reason = "a machine's changed lines are lists of ranges"
)]

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{
    Repo, Space, assert_holds_none, bytes, files, inventory, inventory_with, lines, long_lines,
};
use edit_overlap::harness::Machine;
use edit_overlap::{Lines, git};
use tempfile::TempDir;

/// What each machine learns from the server, by the other machine.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    /// Its paths that the other machine edits too.
    paths: BTreeMap<String, BTreeSet<String>>,
    /// Its changed lines whose hunks overlap the other machine's.
    hunks: BTreeMap<String, Lines>,
}

impl Space<'_> {
    /// What the machine `name` learns from the server.
    fn seen(&self, name: &str) -> Seen {
        let edits = self.edits_of(name);
        let overlaps = self.server.overlaps(name);
        Seen {
            paths: edits.overlapping_paths(&overlaps),
            hunks: edits.overlapping_hunks(&overlaps),
        }
    }
}

/// Each other machine with the same paths, `[(machine, [path])]`.
fn paths(others: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    others
        .iter()
        .map(|(machine, paths)| {
            let paths = paths.iter().map(|path| path.to_string()).collect();
            (machine.to_string(), paths)
        })
        .collect()
}

/// Each other machine with the changed lines, `[(machine, lines)]`.
fn hunks(others: &[(&str, Lines)]) -> BTreeMap<String, Lines> {
    others
        .iter()
        .map(|(machine, lines)| (machine.to_string(), lines.clone()))
        .collect()
}

#[test]
fn edits_to_the_same_lines_overlap_at_hunk_level_and_conflict() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    // Lines 7 and 8, committed on one machine and uncommitted on the other, which also edits
    // line 2.
    a.write(
        "src/inventory.rs",
        &inventory(&[(7, "    let reserved = 10;"), (8, "    reserved + 1")]),
    )
    .unwrap();
    a.commit("reserve ten").unwrap();
    b.write(
        "src/inventory.rs",
        &inventory(&[
            (2, "    let items = 3;"),
            (7, "    let reserved = 20;"),
            (8, "    reserved * 2"),
        ]),
    )
    .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    let file = &["src/inventory.rs"][..];
    assert_eq!(
        space.seen("alice-laptop"),
        Seen {
            paths: paths(&[("bob-desktop", file)]),
            hunks: hunks(&[("bob-desktop", lines(&[("src/inventory.rs", &[7..9])]))]),
        }
    );
    assert_eq!(
        space.seen("bob-desktop"),
        Seen {
            paths: paths(&[("alice-laptop", file)]),
            hunks: hunks(&[("alice-laptop", lines(&[("src/inventory.rs", &[7..9])]))]),
        }
    );
    // The lines that overlap at hunk level are the lines that conflict.
    for (machine, other) in [
        ("alice-laptop", "bob-desktop"),
        ("bob-desktop", "alice-laptop"),
    ] {
        let check = &space.check(machine)[other];
        assert!(!check.clean);
        assert_eq!(check.files, space.seen(machine).hunks[other]);
    }
}

#[test]
fn edits_next_to_each_other_or_at_one_place_overlap_at_hunk_level_and_conflict() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (c, c_client) = repo.machine("carol-server");
    let (d, d_client) = repo.machine("dave-laptop");
    // Line 7; line 8, next to it; a line inserted between them; another line inserted there.
    a.write(
        "src/inventory.rs",
        &inventory(&[(7, "    let reserved = 10;")]),
    )
    .unwrap();
    b.write("src/inventory.rs", &inventory(&[(8, "    reserved * 2")]))
        .unwrap();
    let inserted = |line: &str| {
        let text = format!("    let reserved = 1;\n{line}");
        inventory(&[(7, &text)])
    };
    c.write("src/inventory.rs", &inserted("    // Reserve one."))
        .unwrap();
    d.write("src/inventory.rs", &inserted("    assert!(reserved > 0);"))
        .unwrap();
    let machines = [
        ("alice-laptop", 7..8),
        ("bob-desktop", 8..9),
        ("carol-server", 8..9),
        ("dave-laptop", 8..9),
    ];
    let mut space = Space::new(&[
        (&a, &a_client),
        (&b, &b_client),
        (&c, &c_client),
        (&d, &d_client),
    ]);
    space.publish();

    for (machine, own) in &machines {
        let others: Vec<(&str, Lines)> = machines
            .iter()
            .filter(|(other, _)| other != machine)
            .map(|(other, _)| {
                (
                    *other,
                    lines(&[("src/inventory.rs", std::slice::from_ref(own))]),
                )
            })
            .collect();
        assert_eq!(space.seen(machine).hunks, hunks(&others), "{machine}");
        let checks = space.check(machine);
        for (other, _) in &others {
            assert!(!checks[*other].clean, "{machine} and {other}");
        }
    }
}

#[test]
fn edits_to_different_hunks_of_one_file_overlap_at_file_level_only() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (c, c_client) = repo.machine("carol-server");
    // Lines 7 and 9, one unchanged line apart, and line 2, far from both.
    a.write(
        "src/inventory.rs",
        &inventory(&[(7, "    let reserved = 10;")]),
    )
    .unwrap();
    a.commit("reserve ten").unwrap();
    b.write("src/inventory.rs", &inventory(&[(9, "} // reserve")]))
        .unwrap();
    c.write("src/inventory.rs", &inventory(&[(2, "    let items = 3;")]))
        .unwrap();
    let names = ["alice-laptop", "bob-desktop", "carol-server"];
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client), (&c, &c_client)]);
    space.publish();

    for machine in names {
        let others: Vec<(&str, &[&str])> = names
            .iter()
            .filter(|other| **other != machine)
            .map(|other| (*other, &["src/inventory.rs"][..]))
            .collect();
        assert_eq!(
            space.seen(machine),
            Seen {
                paths: paths(&others),
                hunks: BTreeMap::new(),
            },
            "{machine}"
        );
        let checks = space.check(machine);
        for (other, _) in others {
            assert!(checks[other].clean, "{machine} and {other}");
        }
    }
}

#[test]
fn bases_that_differ_by_lines_added_above_the_hunk_still_match() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (maintainer, _) = repo.machine("maintainer");
    // Lines added on top upstream; Alice pulls them, Bob does not.
    let top = ["//! Inventory.", "", "use std::fmt;", ""];
    maintainer
        .write("src/inventory.rs", &inventory_with(&top, &[]))
        .unwrap();
    maintainer.commit("document the inventory").unwrap();
    maintainer.push("main").unwrap();
    a.pull().unwrap();
    a.write(
        "src/inventory.rs",
        &inventory_with(&top, &[(7, "    let reserved = 10;")]),
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
        space.seen("alice-laptop").hunks,
        hunks(&[("bob-desktop", lines(&[("src/inventory.rs", &[11..12])]))])
    );
    assert_eq!(
        space.seen("bob-desktop").hunks,
        hunks(&[("alice-laptop", lines(&[("src/inventory.rs", &[7..8])]))])
    );
    for (machine, other) in [
        ("alice-laptop", "bob-desktop"),
        ("bob-desktop", "alice-laptop"),
    ] {
        let check = &space.check(machine)[other];
        assert!(!check.clean);
        assert_eq!(check.files, space.seen(machine).hunks[other]);
    }
}

#[test]
fn hunks_follow_the_diff_of_git_merge_tree_whatever_the_repository_attributes_and_config() {
    let tmp = TempDir::new().unwrap();
    // `-diff` makes `git diff` show a file as binary, without hunks; `diff=upper` converts text
    // files with the command of the config `diff.upper.textconv` first.
    let steps = "build\nbuild\ntest\ntest\ndeploy\ntest\n";
    let braces = "}\n}\n}\n    y();\nfn f() {\n    y();\nfn f() {\n";
    let ids = "pub fn id() {}\n\n\n\npub fn di() {}\n";
    let repo = Repo::create_with(
        &tmp,
        &[
            (".gitattributes", "* -diff\n*.txt diff=upper\n"),
            ("steps.txt", steps),
            ("src/braces.rs", braces),
            ("src/[id].rs", ids),
            ("src/i.rs", "pub fn i() {}\n"),
        ],
    );
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    // A text conversion, an external diff that fails, no lines of context, and colors.
    a.git(&["config", "diff.upper.textconv", "tr a-z A-Z"])
        .unwrap();
    a.git(&["config", "diff.external", "false"]).unwrap();
    a.git(&["config", "diff.context", "0"]).unwrap();
    a.git(&["config", "color.diff", "always"]).unwrap();
    // `git diff` writes empty lines of context without their space.
    b.git(&["config", "diff.suppressBlankEmpty", "true"])
        .unwrap();
    // A step moved to the top: the histogram diff, which `git merge-tree` uses, deletes lines 1
    // and 2 and adds them after line 3; the default Myers diff adds line 1 and deletes line 4.
    a.write("steps.txt", "test\nbuild\nbuild\ntest\ndeploy\ntest\n")
        .unwrap();
    b.write(
        "steps.txt",
        &steps.replacen("build\nbuild", "build\nlint", 1),
    )
    .unwrap();
    // A fourth `}`: without the indent heuristic, as in `git merge-tree`, the diff adds it after
    // line 3; with it, `git diff` adds it after line 2.
    a.write("src/braces.rs", &format!("}}\n{braces}")).unwrap();
    b.write("src/braces.rs", &braces.replacen("y()", "z()", 1))
        .unwrap();
    // As a pattern, the path `src/[id].rs` matches `src/i.rs` too: edits of its distant lines
    // merge cleanly, edits of `src/i.rs` conflict.
    a.write("src/[id].rs", &ids.replacen("id()", "id() -> u8", 1))
        .unwrap();
    b.write("src/[id].rs", &ids.replacen("di()", "di() -> u8", 1))
        .unwrap();
    a.write("src/i.rs", "pub fn i() -> u8 { 1 }\n").unwrap();
    b.write("src/i.rs", "pub fn i() -> u16 { 2 }\n").unwrap();
    // The changed lines follow an empty line.
    a.write(
        "src/inventory.rs",
        &inventory(&[(6, "pub fn reserve() -> u64 {")]),
    )
    .unwrap();
    b.write(
        "src/inventory.rs",
        &inventory(&[(6, "pub fn reserve() -> u16 {")]),
    )
    .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    let files = BTreeSet::from(["src/braces.rs", "src/i.rs", "src/inventory.rs", "steps.txt"]);
    for (machine, other) in [
        ("alice-laptop", "bob-desktop"),
        ("bob-desktop", "alice-laptop"),
    ] {
        let seen = space.seen(machine).hunks;
        let check = &space.check(machine)[other];
        assert_eq!(
            seen[other]
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            files
        );
        assert_eq!(
            check
                .files
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            files
        );
    }
}

#[test]
fn submodule_moves_overlap_at_hunk_level_whatever_the_repository_config() {
    let tmp = TempDir::new().unwrap();
    // `ignore = all` hides the submodule's changes from `git diff`.
    let modules = "[submodule \"lib\"]\n\tpath = vendor/lib\n\turl = ./lib\n\tignore = all\n";
    let repo = Repo::create_with(&tmp, &[(".gitmodules", modules)]);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (maintainer, _) = repo.machine("maintainer");
    // Points the submodule at `commit`; it is not checked out, so its directory stays empty.
    let point = |machine: &Machine, commit: char| {
        std::fs::create_dir_all(machine.dir().join("vendor/lib")).unwrap();
        let entry = format!("160000,{},vendor/lib", commit.to_string().repeat(40));
        machine
            .git(&["update-index", "--add", "--cacheinfo", &entry])
            .unwrap();
    };
    point(&maintainer, '1');
    maintainer.commit("add the library").unwrap();
    maintainer.push("main").unwrap();
    a.pull().unwrap();
    b.pull().unwrap();
    // `git diff` writes a log of the submodule in place of its `Subproject commit` line, or hides
    // its changes.
    a.git(&["config", "diff.submodule", "log"]).unwrap();
    b.git(&["config", "diff.ignoreSubmodules", "all"]).unwrap();
    // Both move it, committed on one machine and staged on the other.
    point(&a, '2');
    a.commit("update the library").unwrap();
    point(&b, '3');
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    for (machine, other) in [
        ("alice-laptop", "bob-desktop"),
        ("bob-desktop", "alice-laptop"),
    ] {
        assert_eq!(
            space.seen(machine),
            Seen {
                paths: paths(&[(other, &["vendor/lib"])]),
                hunks: hunks(&[(other, lines(&[("vendor/lib", &[1..2])]))]),
            },
            "{machine}"
        );
    }
    // The exact check reads each conflicted path as a file, so `git merge-tree` runs here.
    let ours = git::snapshot(a.dir()).unwrap();
    let base = git::merge_base(a.dir()).unwrap();
    let bundle = git::bundle(b.dir(), &git::snapshot(b.dir()).unwrap(), &base).unwrap();
    let theirs = git::unbundle(a.dir(), &bundle).unwrap();
    let args = ["merge-tree", "--write-tree", "--name-only", &ours, &theirs];
    let output = git::command(a.dir(), &args).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("CONFLICT (submodule)"), "{stdout}");
}

#[test]
fn hunk_fingerprints_match_without_a_conflict_on_common_lines_and_identical_edits() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (c, c_client) = repo.machine("carol-server");
    // A comment after the first function and one after the second: both go between `}` and an
    // empty line.
    let added = |line: usize, comment: &str| inventory(&[(line, &format!("}}\n{comment}"))]);
    a.write("src/inventory.rs", &added(4, "// counted"))
        .unwrap();
    b.write("src/inventory.rs", &added(9, "// reserved"))
        .unwrap();
    // The same edit as Alice's.
    c.write("src/inventory.rs", &added(4, "// counted"))
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client), (&c, &c_client)]);
    space.publish();

    assert_eq!(
        space.seen("alice-laptop").hunks,
        hunks(&[
            ("bob-desktop", lines(&[("src/inventory.rs", &[5..6])])),
            ("carol-server", lines(&[("src/inventory.rs", &[5..6])])),
        ])
    );
    let checks = space.check("alice-laptop");
    assert!(checks["bob-desktop"].clean && checks["carol-server"].clean);
}

#[test]
fn rename_matches_an_edit_of_its_old_path_at_hunk_level_and_merges_cleanly() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    // The rename deletes the old path, which touches each of its boundaries; `git merge-tree`
    // carries the edit into the renamed file.
    a.rename("docs/guide.md", "docs/handbook.md").unwrap();
    a.commit("rename the guide").unwrap();
    b.write("docs/guide.md", "# Guide\n\nRead the docs.\n")
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    let file = &["docs/guide.md"][..];
    assert_eq!(
        space.seen("bob-desktop"),
        Seen {
            paths: paths(&[("alice-laptop", file)]),
            hunks: hunks(&[("alice-laptop", lines(&[("docs/guide.md", &[3..4])]))]),
        }
    );
    assert_eq!(
        space.seen("alice-laptop"),
        Seen {
            paths: paths(&[("bob-desktop", file)]),
            hunks: hunks(&[("bob-desktop", lines(&[("docs/guide.md", &[1..1])]))]),
        }
    );
    assert!(space.check("bob-desktop")["alice-laptop"].clean);
    assert!(space.check("alice-laptop")["bob-desktop"].clean);
}

#[test]
fn conflicts_with_upstream_commits_only_one_machine_has_do_not_match_at_hunk_level() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (maintainer, _) = repo.machine("maintainer");
    // Line 7 changed upstream; Alice pulls it and edits line 2, Bob does not and edits line 7.
    maintainer
        .write(
            "src/inventory.rs",
            &inventory(&[(7, "    let reserved = 5;")]),
        )
        .unwrap();
    maintainer.commit("reserve five").unwrap();
    maintainer.push("main").unwrap();
    a.pull().unwrap();
    a.write(
        "src/inventory.rs",
        &inventory(&[(2, "    let items = 3;"), (7, "    let reserved = 5;")]),
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
        space.seen("bob-desktop"),
        Seen {
            paths: paths(&[("alice-laptop", &["src/inventory.rs"])]),
            hunks: BTreeMap::new(),
        }
    );
    // The conflict is with the upstream change, which Alice did not make.
    let check = &space.check("bob-desktop")["alice-laptop"];
    assert_eq!(check.files, lines(&[("src/inventory.rs", &[7..8])]));
}

#[test]
fn server_receives_no_line_of_the_test_repository() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let a_text = inventory(&[(7, "    let reserved = 10;")]);
    let b_text = inventory(&[(7, "    let reserved = 20;"), (12, "    let released = 3;")]);
    a.write("src/inventory.rs", &a_text).unwrap();
    a.commit("reserve ten").unwrap();
    a.delete("src/warehouse.rs").unwrap();
    b.write("src/inventory.rs", &b_text).unwrap();
    b.write("notes/todo-list.md", "- ship the reservations\n")
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();
    assert!(!space.seen("alice-laptop").hunks.is_empty());

    // Every line of the repository and of the edits long enough not to occur in random bytes by
    // chance.
    let files = files();
    let texts = files.iter().map(|(_, text)| text.as_str());
    let needles = long_lines(texts.chain([a_text.as_str(), &b_text, "- ship the reservations\n"]));
    assert!(needles.contains("let reserved = 20;") && needles.contains("pub struct Warehouse;"));
    let received = space.server.received().iter();
    let bytes =
        bytes(received.map(|publication| (&publication.machine, &publication.fingerprints)));
    assert_holds_none(&bytes, needles, "what the server received");
}
