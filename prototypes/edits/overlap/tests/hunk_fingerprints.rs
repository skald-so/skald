//! The hunk fingerprint scenarios of `docs/dev/edit-prototype.md`, "Scenarios": machines also
//! publish fingerprints of their changed hunks, and those match where `git merge-tree` conflicts.
#![allow(
    clippy::single_range_in_vec_init,
    reason = "a machine's changed lines are lists of ranges"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use edit_overlap::harness::{Machine, Upstream};
use edit_overlap::{Check, Client, Edits, Lines, Messages, Server, SpaceKey};
use tempfile::TempDir;

/// The space key, a test fixture.
fn key() -> SpaceKey {
    SpaceKey::from_bytes(*b"a fixture space key, 32 bytes!!!")
}

/// `src/inventory.rs` in the test repository, line by line.
const INVENTORY: [&str; 14] = [
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
fn files() -> Vec<(&'static str, String)> {
    vec![
        ("src/inventory.rs", inventory(&[])),
        ("src/warehouse.rs", "pub struct Warehouse;\n".into()),
    ]
}

/// `src/inventory.rs` with `top` on top of it and each `(line, text)` of `edits`, 1-based, in
/// place of its line; a text with line breaks stands for several lines, an empty one for none.
fn inventory_with(top: &[&str], edits: &[(usize, &str)]) -> String {
    let mut lines: Vec<String> = INVENTORY.iter().map(|line| format!("{line}\n")).collect();
    for &(line, text) in edits {
        lines[line - 1] = text.lines().map(|line| format!("{line}\n")).collect();
    }
    let top: String = top.iter().map(|line| format!("{line}\n")).collect();
    top + &lines.concat()
}

fn inventory(edits: &[(usize, &str)]) -> String {
    inventory_with(&[], edits)
}

struct Repo {
    upstream: Upstream,
}

impl Repo {
    fn create(tmp: &TempDir) -> Self {
        Self::create_with(tmp, &[])
    }

    /// The test repository with `extra` files, each a path and its contents.
    fn create_with(tmp: &TempDir, extra: &[(&str, &str)]) -> Self {
        let files = files();
        let files: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, text)| (*path, text.as_str()))
            .chain(extra.iter().copied())
            .collect();
        Self {
            upstream: Upstream::create(&tmp.path().join("repo"), &files).unwrap(),
        }
    }

    fn machine(&self, name: &str) -> (Machine, Client) {
        (
            self.upstream.clone_as(name).unwrap(),
            Client::new(name, key()),
        )
    }
}

/// What each machine learns from the server, by the other machine.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    /// Its paths that the other machine edits too.
    paths: BTreeMap<String, BTreeSet<String>>,
    /// Its changed lines whose hunks overlap the other machine's.
    hunks: BTreeMap<String, Lines>,
}

/// A space: the server stand-in, the messages between machines, and its machines.
struct Space<'a> {
    server: Server,
    messages: Messages,
    machines: Vec<(&'a Machine, &'a Client)>,
    edits: Vec<Edits>,
}

impl<'a> Space<'a> {
    fn new(machines: &[(&'a Machine, &'a Client)]) -> Self {
        Self {
            server: Server::default(),
            messages: Messages::default(),
            machines: machines.to_vec(),
            edits: Vec::new(),
        }
    }

    /// Each machine publishes its edit set.
    fn publish(&mut self) {
        self.edits = self
            .machines
            .iter()
            .map(|(machine, client)| client.edits(machine.dir()).unwrap())
            .collect();
        for edit in &self.edits {
            self.server.publish(edit.publication.clone());
        }
    }

    /// What the machine `name` learns from the server.
    fn seen(&self, name: &str) -> Seen {
        let edits = self
            .edits
            .iter()
            .find(|edits| edits.publication.machine == name)
            .unwrap();
        let overlaps = self.server.overlaps(name);
        Seen {
            paths: edits.overlapping_paths(&overlaps),
            hunks: edits.overlapping_hunks(&overlaps),
        }
    }

    fn find(&self, name: &str) -> (&'a Machine, &'a Client) {
        *self
            .machines
            .iter()
            .find(|(_, client)| client.machine == name)
            .unwrap()
    }

    /// `name` checks its overlap with `other` with `git merge-tree`.
    fn check(&mut self, name: &str, other: &str) -> Check {
        let (machine, client) = self.find(name);
        let (other_machine, other_client) = self.find(other);
        client
            .request_changes(machine.dir(), other, &mut self.messages)
            .unwrap();
        other_client
            .answer_requests(other_machine.dir(), &mut self.messages)
            .unwrap();
        let mut checks = client.check(machine.dir(), &mut self.messages).unwrap();
        checks.remove(other).unwrap()
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

/// Paths with ranges of lines, `[(path, [lines])]`.
fn lines(files: &[(&str, &[Range<usize>])]) -> Lines {
    files
        .iter()
        .map(|(path, ranges)| (path.to_string(), ranges.to_vec()))
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
        let check = space.check(machine, other);
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
        for (other, _) in &others {
            assert!(!space.check(machine, other).clean, "{machine} and {other}");
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
        for (other, _) in others {
            assert!(space.check(machine, other).clean, "{machine} and {other}");
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
    maintainer
        .git(&["push", "--quiet", "origin", "HEAD:main"])
        .unwrap();
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
        let check = space.check(machine, other);
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
    let repo = Repo::create_with(
        &tmp,
        &[
            (".gitattributes", "* -diff\n*.txt diff=upper\n"),
            ("steps.txt", steps),
            ("src/braces.rs", braces),
        ],
    );
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    // A text conversion, an external diff that fails, and no lines of context.
    a.git(&["config", "diff.upper.textconv", "tr a-z A-Z"])
        .unwrap();
    a.git(&["config", "diff.external", "false"]).unwrap();
    a.git(&["config", "diff.context", "0"]).unwrap();
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

    let files = BTreeSet::from(["src/braces.rs", "src/inventory.rs", "steps.txt"]);
    for (machine, other) in [
        ("alice-laptop", "bob-desktop"),
        ("bob-desktop", "alice-laptop"),
    ] {
        let seen = space.seen(machine).hunks;
        let check = space.check(machine, other);
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
    assert!(space.check("alice-laptop", "bob-desktop").clean);
    assert!(space.check("alice-laptop", "carol-server").clean);
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
    maintainer
        .git(&["push", "--quiet", "origin", "HEAD:main"])
        .unwrap();
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
    let check = space.check("bob-desktop", "alice-laptop");
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
    let mut needles = BTreeSet::new();
    for (_, text) in files() {
        needles.extend(text.lines().map(str::to_owned));
    }
    for text in [a_text.as_str(), &b_text, "- ship the reservations\n"] {
        needles.extend(text.lines().map(str::to_owned));
    }
    needles.retain(|line| line.trim().len() >= 4);
    assert!(
        needles.contains("    let reserved = 20;") && needles.contains("pub struct Warehouse;")
    );
    let mut bytes = Vec::new();
    for publication in space.server.received() {
        bytes.extend_from_slice(publication.machine.as_bytes());
        for fingerprint in &publication.fingerprints {
            bytes.extend_from_slice(&fingerprint.0);
        }
    }
    for needle in needles {
        for needle in [needle.as_str(), needle.trim()] {
            let found = bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes());
            assert!(!found, "{needle} in what the server received");
        }
    }
}
