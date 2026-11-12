//! The measurement of `docs/dev/edit-prototype.md`, "Measurement", on a fixture repository with
//! known merges: the replay gives the expected warnings, conflicts, lifetimes and sizes at each
//! level.

mod common;

use std::fs;
use std::sync::Mutex;

use common::Repo;
use edit_overlap::git;
use edit_overlap::harness::{DATE, Machine};
use edit_overlap::noise::{self, Class, Signal};
use edit_overlap::replay::{self, Counts, Level, Measured, Replay, Repository};
use tempfile::TempDir;

/// `machine` replaces each `(path, line, text)` of `edits`, a 1-based line, with `text` on a new
/// branch `branch` from the upstream's default branch; a text with line breaks stands for several
/// lines, and a path that does not exist is created. It commits them with an author date `hours`
/// before [`DATE`] and pushes the branch.
fn branch(machine: &Machine, branch: &str, hours: i64, edits: &[(&str, usize, &str)]) {
    renaming_branch(machine, branch, hours, &[], edits);
}

/// [`branch`] with each `(from, to)` of `renames` moved before the edits.
fn renaming_branch(
    machine: &Machine,
    branch: &str,
    hours: i64,
    renames: &[(&str, &str)],
    edits: &[(&str, usize, &str)],
) {
    machine.git(&["fetch", "--quiet", "origin"]).unwrap();
    machine
        .git(&["checkout", "--quiet", "-B", branch, "origin/main"])
        .unwrap();
    for (from, to) in renames {
        machine.rename(from, to).unwrap();
    }
    for &(path, line, text) in edits {
        let file = fs::read_to_string(machine.dir().join(path)).unwrap_or_default();
        let mut lines: Vec<String> = file.lines().map(|line| format!("{line}\n")).collect();
        if lines.len() < line {
            lines.resize(line, String::new());
        }
        lines[line - 1] = text.lines().map(|line| format!("{line}\n")).collect();
        machine.write(path, &lines.concat()).unwrap();
    }
    machine.git(&["add", "--all"]).unwrap();
    let (seconds, zone) = DATE.split_once(' ').unwrap();
    let date = format!("{} {zone}", seconds.parse::<i64>().unwrap() - hours * 3600);
    machine
        .git(&["commit", "--quiet", "--message", branch, "--date", &date])
        .unwrap();
    machine.push(branch).unwrap();
}

/// A branch: its name, the hours before [`DATE`] of its commit, and its edits, each
/// `(path, line, text)` as [`branch`] takes it.
type Branch = (
    &'static str,
    i64,
    &'static [(&'static str, usize, &'static str)],
);

/// The merges of the fixture, as the maintainer merges them: each pair of branches starts from
/// the default branch after the pair before it, the first branch of a pair is Alice's, the second
/// Bob's, which the maintainer merges second, resolving any conflict to the default branch's side.
const PAIRS: [[Branch; 2]; 6] = [
    // Different files.
    [
        (
            "warehouse",
            2,
            &[("src/warehouse.rs", 1, "pub struct Warehouse(u32);")],
        ),
        (
            "guide",
            3,
            &[("docs/guide.md", 3, "Read the code and the docs.")],
        ),
    ],
    // The same line of `reserve()`: a conflict.
    [
        (
            "same-a",
            1,
            &[("src/inventory.rs", 7, "    let reserved = 10;")],
        ),
        (
            "same-b",
            4,
            &[("src/inventory.rs", 7, "    let reserved = 20;")],
        ),
    ],
    // Distant lines of `reserve()`.
    [
        (
            "distant-a",
            5,
            &[("src/inventory.rs", 6, "pub fn reserve() -> u32 { // one")],
        ),
        (
            "distant-b",
            6,
            &[("src/inventory.rs", 8, "    reserved + 0")],
        ),
    ],
    // Different functions of one file, and distant lines of the guide.
    [
        (
            "functions-a",
            7,
            &[
                ("src/inventory.rs", 2, "    let items = 3;"),
                ("docs/guide.md", 1, "# User guide"),
            ],
        ),
        (
            "functions-b",
            8,
            &[
                ("src/inventory.rs", 12, "    let released = 4;"),
                ("docs/guide.md", 3, "Read the code first."),
            ],
        ),
    ],
    // Different lines inserted at one place of `release()`: only additions, and a conflict.
    [
        (
            "insert-a",
            9,
            &[("src/inventory.rs", 13, "    let _a = 1;\n    released")],
        ),
        (
            "insert-b",
            10,
            &[("src/inventory.rs", 13, "    let _b = 2;\n    released")],
        ),
    ],
    // The empty line between `count()` and `reserve()`, outside every symbol: a conflict.
    [
        ("between-a", 11, &[("src/inventory.rs", 5, "// Alice")]),
        ("between-b", 12, &[("src/inventory.rs", 5, "// Bob")]),
    ],
];

/// Alice's, Bob's and the maintainer's clones of the test repository.
fn clones(tmp: &TempDir) -> [Machine; 3] {
    clones_with(tmp, &[])
}

/// [`clones`] of the test repository with `extra` files, each a path and its contents.
fn clones_with(tmp: &TempDir, extra: &[(&str, &str)]) -> [Machine; 3] {
    let repo = Repo::create_with(tmp, extra);
    ["alice-laptop", "bob-desktop", "maintainer"].map(|name| repo.machine(name).0)
}

/// The fixture: an octopus merge of two branches, then the merges of [`PAIRS`]. Returns the
/// maintainer's clone, whose `HEAD` is the last merge.
fn fixture(tmp: &TempDir) -> Machine {
    let [alice, bob, maintainer] = clones(tmp);
    branch(
        &alice,
        "octopus-a",
        0,
        &[("src/warehouse.rs", 1, "pub struct Warehouse;\n// A")],
    );
    branch(&bob, "octopus-b", 0, &[(".gitignore", 1, "/target\n# B")]);
    maintainer.pull().unwrap();
    maintainer.git(&["fetch", "--quiet", "origin"]).unwrap();
    maintainer
        .git(&[
            "merge",
            "--quiet",
            "--no-ff",
            "--message",
            "octopus",
            "origin/octopus-a",
            "origin/octopus-b",
        ])
        .unwrap();
    maintainer
        .git(&["push", "--quiet", "origin", "HEAD:main"])
        .unwrap();
    merge_pairs([&alice, &bob, &maintainer], &PAIRS);
    maintainer
}

/// Each pair of `pairs` in turn: Alice's branch and Bob's from the default branch, merged by the
/// maintainer, Alice's first, Bob's resolving any conflict to the default branch's side.
fn merge_pairs([alice, bob, maintainer]: [&Machine; 3], pairs: &[[Branch; 2]]) {
    for &[(a, a_hours, a_edits), (b, b_hours, b_edits)] in pairs {
        branch(alice, a, a_hours, a_edits);
        branch(bob, b, b_hours, b_edits);
        maintainer.merge(a).unwrap();
        maintainer
            .merge_with(b, &["--strategy-option=ours"])
            .unwrap();
    }
}

/// The merges of the fixture replayed on 3 threads, the newest first.
fn replays(maintainer: &Machine) -> Vec<Replay> {
    let dir = maintainer.dir();
    let scope = git::scope_of(dir, "HEAD").unwrap();
    let merges = replay::merges(dir, "HEAD", 100).unwrap();
    let progress = Mutex::new(Vec::new());
    let replays = replay::replay_all(dir, &scope, &merges, 3, |done| {
        progress.lock().unwrap().push(done)
    })
    .unwrap();
    let replayed: Vec<&str> = replays.iter().map(|replay| replay.merge.as_str()).collect();
    assert_eq!(replayed, merges);
    let mut progress = progress.into_inner().unwrap();
    progress.sort();
    assert_eq!(progress, (1..=merges.len()).collect::<Vec<_>>());
    replays
}

/// The rows of the table `title` of `report`.
fn rows(report: &str, title: &str) -> Vec<String> {
    let lines: Vec<&str> = report.lines().collect();
    let start = lines.iter().position(|line| *line == title).unwrap();
    lines[start + 4..]
        .iter()
        .take_while(|line| line.starts_with('|'))
        .map(|line| line.to_string())
        .collect()
}

/// The message of `commit`.
fn message(maintainer: &Machine, commit: &str) -> String {
    git::line(maintainer.dir(), &["log", "-1", "--format=%s", commit]).unwrap()
}

#[test]
fn merges_are_those_of_two_parents_newest_first() {
    let tmp = TempDir::new().unwrap();
    let maintainer = fixture(&tmp);
    let dir = maintainer.dir();
    let merges = replay::merges(dir, "HEAD", 100).unwrap();
    let mut messages: Vec<String> = merges.iter().map(|m| message(&maintainer, m)).collect();
    messages.sort();
    let mut want: Vec<&str> = PAIRS
        .iter()
        .flatten()
        .map(|(branch, _, _)| *branch)
        .collect();
    want.sort();
    assert_eq!(messages, want);
    let head = git::line(dir, &["rev-parse", "HEAD"]).unwrap();
    assert_eq!(replay::merges(dir, "HEAD", 1).unwrap(), [head]);
}

#[test]
fn replay_gives_the_expected_counts_at_each_level() {
    let tmp = TempDir::new().unwrap();
    let maintainer = fixture(&tmp);
    let replays = replays(&maintainer);
    let counts = |warnings,
                  conflicts,
                  warnings_without_conflict,
                  conflicts_without_warning,
                  adding,
                  merges: [usize; 4]| Counts {
        warnings,
        conflicts,
        warnings_without_conflict,
        conflicts_without_warning,
        adding,
        adding_with_conflict: adding,
        same: 0,
        merges_warned: merges[0],
        merges_conflicted: merges[1],
        merges_warned_clean: merges[2],
        merges_conflicted_unwarned: merges[3],
    };
    // Warned files: the same line, distant lines and different functions of `src/inventory.rs`,
    // distant lines of the guide, insertions and the empty line; conflicts: the same line,
    // insertions and the empty line.
    assert_eq!(
        Counts::of(&replays, Level::File),
        counts(6, 3, 3, 0, 1, [5, 3, 2, 0])
    );
    assert_eq!(
        Counts::of(&replays, Level::Hunk),
        counts(3, 3, 0, 0, 1, [3, 3, 0, 0])
    );
    assert_eq!(
        Counts::of(&replays, Level::Symbol),
        counts(3, 3, 1, 1, 1, [3, 3, 1, 1])
    );
}

#[test]
fn replay_gives_each_merges_warnings_conflicts_and_sides() {
    let tmp = TempDir::new().unwrap();
    let maintainer = fixture(&tmp);
    let replays = replays(&maintainer);
    let inventory = || ["src/inventory.rs".to_owned()].into();
    for replay in &replays {
        let branch = message(&maintainer, &replay.merge);
        let [file, hunk, symbol] = &replay.warnings;
        let conflicts: Vec<(&str, Vec<&str>)> = replay
            .conflicts
            .iter()
            .map(|c| {
                (
                    c.kind.as_str(),
                    c.paths.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        let none = std::collections::BTreeSet::new();
        let (want_file, want_hunk, want_symbol, conflicted) = match branch.as_str() {
            "same-b" | "insert-b" => (inventory(), inventory(), inventory(), true),
            "distant-b" => (inventory(), none.clone(), inventory(), false),
            "functions-b" => (
                ["docs/guide.md".to_owned(), "src/inventory.rs".to_owned()].into(),
                none.clone(),
                none.clone(),
                false,
            ),
            "between-b" => (inventory(), inventory(), none.clone(), true),
            _ => (none.clone(), none.clone(), none.clone(), false),
        };
        assert_eq!(
            (file, hunk, symbol),
            (&want_file, &want_hunk, &want_symbol),
            "{branch}"
        );
        assert_eq!(replay.clean, !conflicted, "{branch}");
        let want: Vec<(&str, Vec<&str>)> = if conflicted {
            vec![("CONFLICT (contents)", vec!["src/inventory.rs"])]
        } else {
            Vec::new()
        };
        assert_eq!(conflicts, want, "{branch}");
        // Bob's commit is the first of the second parent's side; the merge before it is the first
        // of the first parent's, when the default branch moved since Bob's branch started.
        let hours = PAIRS
            .iter()
            .flatten()
            .find(|(name, _, _)| *name == branch)
            .map(|(_, hours, _)| hours * 3600)
            .unwrap();
        let first = if branch.ends_with("-b") || branch == "guide" {
            Some(0)
        } else {
            None
        };
        let lifetimes = replay.sides.each_ref().map(|side| side.lifetime);
        assert_eq!(lifetimes, [first, Some(hours)], "{branch}");
    }
}

#[test]
fn report_gives_the_tables_of_the_replays() {
    let tmp = TempDir::new().unwrap();
    let maintainer = fixture(&tmp);
    let replays = replays(&maintainer);
    let repository = Repository {
        name: "fixture",
        url: "https://example.com/fixture",
        commit: "0123456789abcdef0123456789abcdef01234567",
        merges: 12,
    };
    let short = |replay: &Replay| replay.merge[..12].to_owned();
    let (newest, oldest) = (short(&replays[0]), short(&replays[replays.len() - 1]));
    let merge = |branch: &str| {
        let replay = replays
            .iter()
            .find(|replay| message(&maintainer, &replay.merge) == branch);
        short(replay.unwrap())
    };
    // The sides that change two files: both of `functions-b`, and the second of `functions-a`,
    // the older merge. Each has 2 path, 4 hunk and 1 symbol fingerprints.
    let largest = [
        (merge("functions-b"), 1),
        (merge("functions-b"), 2),
        (merge("functions-a"), 2),
    ];
    let scope = git::scope_of(maintainer.dir(), "HEAD").unwrap();
    let times = replay::cost(maintainer.dir(), &scope, &replays).unwrap();
    let report = replay::report(&[Measured {
        repository: &repository,
        replays,
        times: times.clone(),
    }]);
    let rows = |title: &str| rows(&report, title);
    assert_eq!(
        rows("Repositories"),
        [format!(
            "| fixture | https://example.com/fixture | `{}` | 12 | `{}` 2023-11-14 | `{}` 2023-11-14 | 6 | 0 |",
            repository.commit, newest, oldest
        )]
    );
    assert_eq!(
        rows("Warnings and conflicts, in files"),
        [
            "| fixture | file | 6 | 3 | 3 | 0 | 1 | 1 | 0 |",
            "| fixture | hunk | 3 | 3 | 0 | 0 | 1 | 1 | 0 |",
            "| fixture | symbol | 3 | 3 | 1 | 1 | 1 | 1 | 0 |",
        ]
    );
    assert!(rows("Conflicts without a file-level warning").is_empty());
    assert_eq!(
        rows("Warnings and conflicts, in merges"),
        [
            "| fixture | file | 5 | 3 | 2 | 0 |",
            "| fixture | hunk | 3 | 3 | 0 | 0 |",
            "| fixture | symbol | 3 | 3 | 1 | 1 |",
        ]
    );
    // Lifetimes: 0 for the 6 first parents that moved, and 1 to 12 hours for the second parents.
    assert_eq!(
        rows("Side lifetimes, in hours"),
        ["| fixture | 18 | 3.0 | 11.0 | 12.0 |"]
    );
    let cost = rows("Cost on the largest sides; times are not compared on a rerun");
    assert_eq!(cost.len(), largest.len());
    for ((row, (merge, parent)), time) in cost.iter().zip(largest).zip(&times) {
        assert!(!time.is_zero());
        let ms = time.as_millis();
        let want = format!("| fixture | `{merge}` | {parent} | 2 | 4 | 1 | 224 | {ms} |");
        assert_eq!(*row, want);
    }
}

#[test]
fn replaying_a_commit_that_is_not_a_merge_fails() {
    let tmp = TempDir::new().unwrap();
    let maintainer = fixture(&tmp);
    let dir = maintainer.dir();
    let scope = git::scope_of(dir, "HEAD").unwrap();
    let mut merges = replay::merges(dir, "HEAD", 100).unwrap();
    merges.insert(1, git::line(dir, &["rev-parse", "HEAD^2"]).unwrap());
    assert!(replay::replay_all(dir, &scope, &merges, 2, |_| ()).is_err());
}

#[test]
fn merges_with_several_merge_bases_are_counted() {
    let tmp = TempDir::new().unwrap();
    let [alice, bob, maintainer] = clones(&tmp);
    // Each branch merges the other before the maintainer merges both: the last merge has the
    // two branches' commits for merge bases.
    branch(
        &alice,
        "cross-a",
        1,
        &[("docs/guide.md", 1, "# User guide")],
    );
    branch(&bob, "cross-b", 2, &[(".gitignore", 1, "/build")]);
    for (machine, own, other) in [(&alice, "cross-a", "cross-b"), (&bob, "cross-b", "cross-a")] {
        machine.git(&["fetch", "--quiet", "origin"]).unwrap();
        let other = format!("origin/{other}");
        machine
            .git(&["merge", "--quiet", "--no-ff", "--message", own, &other])
            .unwrap();
    }
    alice.push("cross-a").unwrap();
    bob.push("cross-b").unwrap();
    maintainer.merge("cross-a").unwrap();
    maintainer.merge("cross-b").unwrap();
    let replays = replays(&maintainer);
    let bases: Vec<usize> = replays.iter().map(|replay| replay.bases).collect();
    assert_eq!(bases, [2, 1, 1, 1]);
    // The replay takes the first merge base `git merge-base --all` gives, Alice's commit or
    // Bob's. From it, both sides change the other's file, the same way.
    let last = &replays[0];
    let dir = maintainer.dir();
    let [first, other] = [&last.parents[0], &last.parents[1]];
    let all = git::run(dir, &["merge-base", "--all", first, other]).unwrap();
    let all: Vec<&str> = std::str::from_utf8(&all).unwrap().lines().collect();
    assert_eq!(all.len(), 2);
    assert_eq!(last.base, all[0]);
    let alice_commit = git::line(dir, &["rev-parse", "origin/cross-a^1"]).unwrap();
    let file = if last.base == alice_commit {
        ".gitignore"
    } else {
        "docs/guide.md"
    };
    let file: std::collections::BTreeSet<String> = [file.to_owned()].into();
    assert_eq!(last.warnings[Level::File as usize], file);
    assert_eq!(last.same, file);
    let repository = Repository {
        name: "cross",
        url: "https://example.com/cross",
        commit: "0123456789abcdef0123456789abcdef01234567",
        merges: 4,
    };
    let report = replay::report(&[Measured {
        repository: &repository,
        replays,
        times: Vec::new(),
    }]);
    let row = &rows(&report, "Repositories")[0];
    assert!(row.ends_with(" | 1 |"), "{row}");
}

#[test]
fn report_gives_a_repository_without_merges_and_lifetimes() {
    let repository = Repository {
        name: "empty",
        url: "https://example.com/empty",
        commit: "0123456789abcdef0123456789abcdef01234567",
        merges: 0,
    };
    let report = replay::report(&[Measured {
        repository: &repository,
        replays: Vec::new(),
        times: Vec::new(),
    }]);
    assert_eq!(
        rows(&report, "Repositories"),
        [format!(
            "| empty | https://example.com/empty | `{}` | 0 | - | - | 0 | 0 |",
            repository.commit
        )]
    );
    assert_eq!(
        rows(&report, "Side lifetimes, in hours"),
        ["| empty | 0 | - | - | - |"]
    );
    let cost = rows(
        &report,
        "Cost on the largest sides; times are not compared on a rerun",
    );
    assert!(cost.is_empty());
}

#[test]
fn a_conflict_in_a_renamed_file_has_no_file_level_warning() {
    let tmp = TempDir::new().unwrap();
    let [alice, bob, maintainer] = clones(&tmp);
    // Alice renames the file and edits a line of it; Bob edits that line at the old path.
    let renames = [("src/inventory.rs", "src/stock.rs")];
    let line = (7, "    let reserved = 10;");
    renaming_branch(
        &alice,
        "rename-a",
        1,
        &renames,
        &[("src/stock.rs", line.0, line.1)],
    );
    branch(
        &bob,
        "rename-b",
        2,
        &[("src/inventory.rs", 7, "    let reserved = 20;")],
    );
    maintainer.merge("rename-a").unwrap();
    maintainer
        .merge_with("rename-b", &["--strategy-option=ours"])
        .unwrap();
    let replays = replays(&maintainer);
    let last = &replays[0];
    let inventory = ["src/inventory.rs".to_owned()].into();
    assert_eq!(last.warnings[Level::File as usize], inventory);
    let kinds: Vec<(&str, &[String])> = last
        .conflicts
        .iter()
        .map(|conflict| (conflict.kind.as_str(), &conflict.paths[..]))
        .collect();
    assert_eq!(
        kinds,
        [("CONFLICT (contents)", &["src/stock.rs".to_owned()][..])]
    );
    let repository = Repository {
        name: "rename",
        url: "https://example.com/rename",
        commit: "0123456789abcdef0123456789abcdef01234567",
        merges: 2,
    };
    let merge = last.merge[..12].to_owned();
    let report = replay::report(&[Measured {
        repository: &repository,
        replays,
        times: Vec::new(),
    }]);
    assert_eq!(
        rows(&report, "Conflicts without a file-level warning"),
        [format!(
            "| rename | `{merge}` | 1 | CONFLICT (contents) | `src/stock.rs` |"
        )]
    );
}

#[test]
fn the_same_change_on_both_sides_is_counted() {
    let tmp = TempDir::new().unwrap();
    let [alice, bob, maintainer] = clones(&tmp);
    // Both make one change to `reserve()`, and different changes to distant lines of the guide.
    let same = ("src/inventory.rs", 7, "    let reserved = 10;");
    branch(
        &alice,
        "same-a",
        1,
        &[same, ("docs/guide.md", 1, "# User guide")],
    );
    branch(&bob, "same-b", 2, &[same, ("docs/guide.md", 3, "Read it.")]);
    maintainer.merge("same-a").unwrap();
    maintainer.merge("same-b").unwrap();
    let replays = replays(&maintainer);
    let last = &replays[0];
    assert!(last.clean);
    let inventory: std::collections::BTreeSet<String> = ["src/inventory.rs".to_owned()].into();
    assert_eq!(last.same, inventory);
    let same = Level::ALL.map(|level| {
        let counts = Counts::of(&replays, level);
        (counts.warnings, counts.same)
    });
    assert_eq!(same, [(2, 1), (1, 1), (1, 1)]);
}

#[test]
fn measure_fetches_a_repository_at_its_commit_and_replays_its_merges() {
    let tmp = TempDir::new().unwrap();
    let maintainer = fixture(&tmp);
    let dir = maintainer.dir();
    let upstream = dir.parent().unwrap().join("upstream.git");
    // Fetched at the merge before the last one, whose history has 11 merges of two parents.
    let commit = git::line(dir, &["rev-parse", "HEAD^1"]).unwrap();
    let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
    let repository = |merges| Repository {
        name: "fixture",
        url: leak(upstream.to_str().unwrap().to_owned()),
        commit: leak(commit.clone()),
        merges,
    };
    let fetched = TempDir::new().unwrap();
    let eleven = repository(11);
    let measured = replay::measure(fetched.path(), &eleven, 2, |_| ()).unwrap();
    let replayed: Vec<&str> = measured.replays.iter().map(|r| r.merge.as_str()).collect();
    assert_eq!(replayed, replay::merges(dir, &commit, 100).unwrap());
    assert_eq!(measured.times.len(), replay::LARGEST);
    let other = TempDir::new().unwrap();
    assert!(replay::measure(other.path(), &repository(12), 2, |_| ()).is_err());
}

#[test]
fn the_header_names_the_machine_and_the_git_version() {
    let tmp = TempDir::new().unwrap();
    let header = replay::header(tmp.path()).unwrap();
    let version = git::line(tmp.path(), &["version"]).unwrap();
    let cpus = std::thread::available_parallelism().unwrap().get();
    let os = format!("{} {}, ", std::env::consts::OS, std::env::consts::ARCH);
    let lines: Vec<&str> = header.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with(&format!("Machine: {os}")));
    assert!(lines[0].ends_with(&format!(", {cpus} CPUs")));
    assert_eq!(lines[1], format!("Git: {version}"));
    assert_eq!(lines[2], "");
    // On macOS the machine names the CPU that `sysctl` gives.
    if cfg!(target_os = "macos") {
        let output = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .env_clear()
            .output()
            .unwrap();
        assert!(output.status.success());
        let cpu = String::from_utf8(output.stdout).unwrap();
        let cpu = cpu.trim();
        assert!(!cpu.is_empty());
        assert_eq!(lines[0], format!("Machine: {os}{cpu}, {cpus} CPUs"));
    }
}

/// The files of the classes the noise fixture adds to the test repository: a lockfile, a
/// migration, and generated code that `.gitattributes`, a marker and a name each recognize.
const CLASSED_FILES: [(&str, &str); 6] = [
    (
        "Cargo.lock",
        "[[package]]\nname = \"inventory\"\nversion = \"0.1.0\"\nsource = \"registry\"\nchecksum = \"0\"\n",
    ),
    (".gitattributes", "assets/*.js linguist-generated\n"),
    ("assets/app.js", "var a = 1;\nvar b = 2;\nvar c = 3;\n"),
    (
        "snapshots/report.snap",
        "---\nsource: tests/report.rs\n---\nline 4\nline 5\n",
    ),
    (
        "src/schema.rs",
        "// @generated automatically by Diesel CLI.\n\ndiesel::table! {\n    users (id) {\n        id -> Integer,\n        name -> Text,\n    }\n}\n",
    ),
    (
        "app/migrations/0001_initial.py",
        "initial = True\ndependencies = []\noperations = []\n",
    ),
];

/// A migration's contents.
const MIGRATION: &str = "operations = []";

/// The merges of the noise fixture, as [`PAIRS`] are merged; a rename of `src/schema.rs` and an
/// edit of it follow them.
const CLASSED_PAIRS: [[Branch; 2]; 8] = [
    // The same line of the lockfile: a conflict.
    [
        (
            "lock-same-a",
            1,
            &[("Cargo.lock", 3, "version = \"0.2.0\"")],
        ),
        (
            "lock-same-b",
            2,
            &[("Cargo.lock", 3, "version = \"0.3.0\"")],
        ),
    ],
    // Distant lines of the lockfile.
    [
        ("lock-far-a", 1, &[("Cargo.lock", 1, "[[package]] # a")]),
        ("lock-far-b", 2, &[("Cargo.lock", 5, "checksum = \"1\"")]),
    ],
    // The same line of the script `.gitattributes` marks generated: a conflict; distant lines of
    // the snapshot; the same change to the schema, which has a marker.
    [
        (
            "generated-a",
            1,
            &[
                ("assets/app.js", 2, "var b = 20;"),
                ("snapshots/report.snap", 1, "--- # a"),
                ("src/schema.rs", 5, "        id -> BigInt,"),
            ],
        ),
        (
            "generated-b",
            2,
            &[
                ("assets/app.js", 2, "var b = 30;"),
                ("snapshots/report.snap", 5, "line five"),
                ("src/schema.rs", 5, "        id -> BigInt,"),
            ],
        ),
    ],
    // Different migrations of one number in one directory.
    [
        (
            "migrations-a",
            1,
            &[("app/migrations/0002_alice.py", 1, MIGRATION)],
        ),
        (
            "migrations-b",
            2,
            &[("app/migrations/0002_bob.py", 1, MIGRATION)],
        ),
    ],
    // Migrations of different numbers in one directory, and the same line of another
    // migration: a conflict.
    [
        (
            "shared-a",
            1,
            &[
                ("app/migrations/0003_alice.py", 1, MIGRATION),
                ("app/migrations/0001_initial.py", 1, "initial = False"),
            ],
        ),
        (
            "shared-b",
            2,
            &[
                ("app/migrations/0004_bob.py", 1, MIGRATION),
                ("app/migrations/0001_initial.py", 1, "initial = None"),
            ],
        ),
    ],
    // Migrations of one number in different directories.
    [
        (
            "elsewhere-a",
            1,
            &[("app/migrations/0005_alice.py", 1, MIGRATION)],
        ),
        (
            "elsewhere-b",
            2,
            &[("lib/migrations/0005_bob.py", 1, MIGRATION)],
        ),
    ],
    // The same migration on both sides.
    [
        (
            "same-migration-a",
            1,
            &[("app/migrations/0006_same.py", 1, MIGRATION)],
        ),
        (
            "same-migration-b",
            2,
            &[("app/migrations/0006_same.py", 1, MIGRATION)],
        ),
    ],
    // Distant lines of the guide, and the same line of `reserve()`: a conflict.
    [
        (
            "other-a",
            1,
            &[
                ("docs/guide.md", 1, "# User guide"),
                ("src/inventory.rs", 7, "    let reserved = 10;"),
            ],
        ),
        (
            "other-b",
            2,
            &[
                ("docs/guide.md", 3, "Read the code first."),
                ("src/inventory.rs", 7, "    let reserved = 20;"),
            ],
        ),
    ],
];

/// The noise fixture: the test repository with [`CLASSED_FILES`], the merges of
/// [`CLASSED_PAIRS`], then Alice's rename of `src/schema.rs`, merged first, and Bob's edit of it.
/// Returns the maintainer's clone, whose `HEAD` is the last merge.
fn classed_fixture(tmp: &TempDir) -> Machine {
    let [alice, bob, maintainer] = clones_with(tmp, &CLASSED_FILES);
    merge_pairs([&alice, &bob, &maintainer], &CLASSED_PAIRS);
    let renames = [("src/schema.rs", "src/tables.rs")];
    renaming_branch(&alice, "rename-a", 1, &renames, &[]);
    branch(
        &bob,
        "rename-b",
        2,
        &[("src/schema.rs", 6, "        name -> Varchar,")],
    );
    maintainer.merge("rename-a").unwrap();
    maintainer.merge("rename-b").unwrap();
    maintainer
}

#[test]
fn files_of_each_class_are_classed_by_what_recognizes_them() {
    let tmp = TempDir::new().unwrap();
    let maintainer = classed_fixture(&tmp);
    let replays = replays(&maintainer);
    let classes: std::collections::BTreeMap<&str, (Class, Option<Signal>)> = replays
        .iter()
        .flat_map(|replay| replay.classes.iter())
        .map(|(path, class)| (path.as_str(), *class))
        .collect();
    let generated = |signal| (Class::Generated, Some(signal));
    assert_eq!(
        classes,
        [
            ("Cargo.lock", (Class::Lockfile, Some(Signal::Name))),
            (
                "app/migrations/0001_initial.py",
                (Class::Migration, Some(Signal::Name))
            ),
            (
                "app/migrations/0006_same.py",
                (Class::Migration, Some(Signal::Name))
            ),
            ("assets/app.js", generated(Signal::Attributes)),
            ("docs/guide.md", (Class::Other, None)),
            ("snapshots/report.snap", generated(Signal::Name)),
            // The rename's merge reads the marker from the second parent: the first has no file.
            ("src/schema.rs", generated(Signal::Marker)),
            ("src/inventory.rs", (Class::Other, None)),
        ]
        .into()
    );
    let rename = &replays[0];
    assert!(
        !git::exists(
            maintainer.dir(),
            &format!("{}:src/schema.rs", rename.parents[0])
        )
        .unwrap()
    );
    assert_eq!(rename.classes["src/schema.rs"], generated(Signal::Marker));
}

#[test]
fn each_way_of_handling_a_class_gives_the_expected_counts() {
    let tmp = TempDir::new().unwrap();
    let maintainer = classed_fixture(&tmp);
    let replays = replays(&maintainer);
    let repository = Repository {
        name: "classed",
        url: "https://example.com/classed",
        commit: "0123456789abcdef0123456789abcdef01234567",
        merges: replays.len(),
    };
    let report = replay::report(&[Measured {
        repository: &repository,
        replays,
        times: Vec::new(),
    }]);
    let rows = |title: &str| rows(&report, title);
    // Warned files: the lockfile in its two pairs; the script, the snapshot and the schema, then
    // the schema again in the rename's merge; the migration both edit and the one both add; the
    // guide and `src/inventory.rs`. Conflicts: the lockfile, the script, the edited migration and
    // `src/inventory.rs`.
    assert_eq!(
        rows("Warnings and conflicts by class, in files"),
        [
            "| classed | lockfile | 2 | 1 | 0 | 1 | 0 | 0 | 1 | 0 | 0 | 1 |",
            "| classed | migration | 2 | 2 | 0 | 1 | 1 | 0 | 1 | 0 | 0 | 1 |",
            "| classed | generated | 4 | 3 | 0 | 3 | 2 | 0 | 1 | 0 | 0 | 1 |",
            "| classed | other | 2 | 1 | 1 | 1 | 0 | 0 | 1 | 0 | 0 | 0 |",
        ]
    );
    assert_eq!(
        rows(
            "Files of warnings at file level and of conflicts by what recognized their class, once per merge"
        ),
        [
            "| classed | lockfile | 2 | 0 | 0 |",
            "| classed | migration | 2 | 0 | 0 |",
            "| classed | generated | 1 | 1 | 2 |",
        ]
    );
    // As is, 6, 3 and 0 warnings without a conflict and 0, 0 and 3 conflicts without a warning.
    assert_eq!(
        rows("Ways of handling a class, in files"),
        [
            "| classed | lockfile | left out | 5 | 3 | 0 | 1 | 1 | 3 |",
            "| classed | lockfile | file level only | 6 | 4 | 1 | 0 | 0 | 2 |",
            "| classed | lockfile | warned apart | 5 | 3 | 0 | 0 | 0 | 3 |",
            "| classed | migration | left out | 5 | 2 | 0 | 1 | 1 | 3 |",
            "| classed | migration | file level only | 6 | 3 | 1 | 0 | 0 | 2 |",
            "| classed | migration | warned apart | 5 | 2 | 0 | 0 | 0 | 3 |",
            "| classed | generated | left out | 3 | 1 | 0 | 1 | 1 | 3 |",
            "| classed | generated | file level only | 6 | 4 | 3 | 0 | 0 | 2 |",
            "| classed | generated | warned apart | 3 | 1 | 0 | 0 | 0 | 3 |",
        ]
    );
    // Both sides add migrations to one directory in three merges: different ones of one number,
    // without a warning; different numbers next to a conflict on another migration; the same one.
    assert_eq!(
        rows("Migrations both sides add, in merges"),
        ["| classed | 3 | 1 | 1 | 1 |"]
    );
}

#[test]
fn attributes_of_the_given_commit_mark_files_generated() {
    let tmp = TempDir::new().unwrap();
    let attributes = "*.gen linguist-generated\n*.true linguist-generated=true\n\
        *.false linguist-generated=false\n*.bin binary\n*.nodiff -diff\n*.diff diff\n";
    let [machine, _, _] = clones_with(&tmp, &[(".gitattributes", attributes)]);
    machine.delete(".gitattributes").unwrap();
    machine.commit("no attributes").unwrap();
    let paths = [
        "a.gen", "b.true", "c.false", "d.bin", "e.nodiff", "f.diff", "src/g.rs",
    ]
    .into();
    let marked = noise::marked_generated(machine.dir(), "HEAD^", &paths).unwrap();
    let want: std::collections::BTreeSet<String> = ["a.gen", "b.true", "d.bin", "e.nodiff"]
        .map(str::to_owned)
        .into();
    assert_eq!(marked, want);
    // The commit without the attributes marks none.
    let marked = noise::marked_generated(machine.dir(), "HEAD", &paths).unwrap();
    assert!(marked.is_empty());
}
