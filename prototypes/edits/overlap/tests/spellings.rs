//! Path normalization, `docs/dev/edit-prototype.md`, "Path normalization": every spelling of one
//! path gets one fingerprint, LF and CRLF versions of one edit get the same fingerprints, and the
//! check of the `spellings` command finds on this machine's file system what the report records.
#![allow(
    clippy::single_range_in_vec_init,
    reason = "a machine's changed lines are lists of ranges"
)]

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use common::{Clones, Repo, Space, hunks, lines, paths, table};
use edit_overlap::spellings::{self, Arrival, FileSystem, LineEndings, NFC, NFD, Rename};
use tempfile::TempDir;

/// The file system of the test's temporary directory, found apart from [`spellings::file_system`]:
/// how many files a directory holds once one is written under two names.
fn file_system() -> FileSystem {
    let tmp = TempDir::new().unwrap();
    let entries = |names: [&str; 2]| {
        let dir = tmp.path().join(names[0]);
        fs::create_dir(&dir).unwrap();
        for name in names {
            fs::write(dir.join(name), name).unwrap();
        }
        fs::read_dir(dir).unwrap().count()
    };
    FileSystem {
        case_insensitive: entries(["a", "A"]) == 1,
        normalization_insensitive: entries([NFC, NFD]) == 1,
    }
}

#[test]
fn the_file_system_and_the_settings_git_init_writes_are_found() {
    let tmp = TempDir::new().unwrap();
    let system = file_system();
    assert_eq!(
        spellings::file_system(&tmp.path().join("probe")).unwrap(),
        system
    );
    // Git writes `core.ignorecase` on a case-insensitive file system and
    // `core.precomposeunicode` on macOS only.
    let settings = spellings::init_settings(&tmp.path().join("init")).unwrap();
    let set = |set: bool| set.then(|| "true".to_owned());
    assert_eq!(
        settings,
        [set(system.case_insensitive), set(cfg!(target_os = "macos"))]
    );
}

#[test]
fn a_case_only_rename_gets_one_path_fingerprint() {
    let tmp = TempDir::new().unwrap();
    let both = || Some((["README.md", "Readme.md"].map(str::to_owned).into(), 1));
    let one = |path: &str| Some(([path.to_owned()].into(), 1));
    // On a case-insensitive file system git sees a rename made outside it as nothing, or with
    // `core.ignorecase` off as a new file; `git mv` refuses it with `core.ignorecase` off. On a
    // case-sensitive one, `core.ignorecase` hides the new name from the untracked files.
    let want: [Option<(BTreeSet<String>, usize)>; 4] = if file_system().case_insensitive {
        [both(), Some((BTreeSet::new(), 0)), None, one("Readme.md")]
    } else {
        [both(), one("README.md"), both(), both()]
    };
    let mut cases = want.into_iter();
    for ignorecase in [true, false] {
        for how in Rename::ALL {
            let dir = tmp.path().join(format!("{ignorecase}-{how:?}"));
            let found = spellings::case_only_rename(&dir, ignorecase, how).unwrap();
            assert_eq!(found, cases.next().unwrap(), "{ignorecase} {how:?}");
        }
    }
}

#[test]
fn names_in_nfc_and_nfd_get_one_path_fingerprint() {
    let tmp = TempDir::new().unwrap();
    let names =
        |edit_set: &[&'static str], index: &[&'static str], fingerprints| spellings::Names {
            edit_set: edit_set.to_vec(),
            index: index.to_vec(),
            fingerprints,
        };
    // Git precomposes the names it reads from the file system on macOS only, with
    // `core.precomposeunicode` on; a name checked out in NFD then reads as a new file in NFC.
    let precomposed = [
        names(&["NFC"], &["NFC"], 1),
        names(&["NFC"], &["NFC"], 1),
        names(&["NFC"], &["NFD"], 1),
    ];
    let as_written = [
        names(&["NFC"], &["NFC"], 1),
        names(&["NFD"], &["NFD"], 1),
        names(&[], &["NFD"], 0),
    ];
    for precompose in [true, false] {
        let want = if precompose && cfg!(target_os = "macos") {
            &precomposed
        } else {
            &as_written
        };
        for (arrival, want) in Arrival::ALL.into_iter().zip(want) {
            let dir = tmp.path().join(format!("{precompose}-{arrival:?}"));
            let found = spellings::names(&dir, precompose, arrival).unwrap();
            assert_eq!(&found, want, "{precompose} {arrival:?}");
        }
    }
}

#[test]
fn every_spelling_of_one_path_gives_one_fingerprint() {
    let tmp = TempDir::new().unwrap();
    let system = file_system();
    let found = spellings::tool_paths(&tmp.path().join("tools")).unwrap();
    // On a case-insensitive or normalization-insensitive file system, the file system gives the
    // name as the directory holds it; on another, the name stays as the tool spells it.
    let other_case = if system.case_insensitive {
        "src/inventory.rs"
    } else {
        "SRC/Inventory.RS"
    };
    let nfd = if system.normalization_insensitive {
        format!("docs/{NFC}")
    } else {
        format!("docs/{NFD}")
    };
    let backslashes = if cfg!(windows) {
        "src/inventory.rs"
    } else {
        "src\\inventory.rs"
    };
    let mut want = vec![
        ("relative", Some("src/inventory.rs")),
        ("relative, with `.` and `..`", Some("src/inventory.rs")),
        ("relative, with backslashes", Some(backslashes)),
        (
            "absolute, as the temporary directory spells it",
            Some("src/inventory.rs"),
        ),
        (
            "absolute, resolved by the file system",
            Some("src/inventory.rs"),
        ),
        (
            "through a symbolic link to the clone",
            Some("src/inventory.rs"),
        ),
        ("from another worktree", Some("src/inventory.rs")),
        ("in another case", Some(other_case)),
        ("with the file name in NFD", Some(&nfd)),
    ];
    if cfg!(windows) {
        want.extend([
            (
                "absolute, with a lowercase drive letter",
                Some("src/inventory.rs"),
            ),
            ("absolute, with forward slashes", Some("src/inventory.rs")),
        ]);
    }
    want.extend([
        ("a deleted file", Some("docs/guide.md")),
        ("outside the repository", None),
        ("in another repository", None),
    ]);
    let found: Vec<(&str, Option<&str>, bool)> = found
        .iter()
        .map(|spelling| {
            let maps_to = spelling.maps_to.as_ref().expect(spelling.name);
            (spelling.name, maps_to.as_deref(), spelling.same)
        })
        .collect();
    let want: Vec<(&str, Option<&str>, bool)> = want
        .into_iter()
        .map(|(name, maps_to)| {
            let same = maps_to.is_some() && (cfg!(windows) || name != "relative, with backslashes");
            (name, maps_to, same)
        })
        .collect();
    assert_eq!(found, want);
}

#[test]
fn paths_past_the_last_existing_component_stay_as_given() {
    let tmp = TempDir::new().unwrap();
    let repo = Repo::create(&tmp);
    let (machine, _) = repo.machine("alice-laptop");
    let relative = |given: &str| edit_overlap::paths::relative(machine.dir(), Path::new(given));
    assert_eq!(
        relative("src/new/../inventory.rs").unwrap().as_deref(),
        Some("src/inventory.rs")
    );
    assert_eq!(
        relative("src/new/deeper/../lib.rs").unwrap().as_deref(),
        Some("src/new/lib.rs")
    );
    // A missing directory is not skipped, even where the path without it names a file.
    assert_eq!(
        relative("src/new/inventory.rs").unwrap().as_deref(),
        Some("src/new/inventory.rs")
    );
    // Nothing resolves below a file.
    assert_eq!(
        relative("src/inventory.rs/x").unwrap().as_deref(),
        Some("src/inventory.rs/x")
    );
    assert_eq!(relative("src/..").unwrap(), None);
    assert_eq!(relative(".").unwrap(), None);
    // A directory of the clone maps to its path.
    assert_eq!(relative("src").unwrap().as_deref(), Some("src"));
}

#[test]
fn line_endings_of_one_edit_give_its_fingerprints_under_each_autocrlf_setting() {
    let tmp = TempDir::new().unwrap();
    // One path, two boundaries and one symbol, `reserve`; written with CRLF and stored so, every
    // line of the file changes: ten boundaries and both functions.
    let want = [
        ("true", "CRLF", "LF", [4, 4, 4]),
        ("input", "LF", "LF", [4, 4, 4]),
        ("false", "LF", "CRLF", [4, 13, 4]),
    ];
    for (autocrlf, (setting, checkout, committed, fingerprints)) in
        spellings::AUTOCRLF.into_iter().zip(want)
    {
        assert_eq!(autocrlf, setting);
        let found = spellings::line_endings(&tmp.path().join(autocrlf), autocrlf).unwrap();
        let want = LineEndings {
            checkout,
            committed,
            fingerprints,
        };
        assert_eq!(found, want, "{autocrlf}");
    }
}

#[test]
fn the_check_prints_each_part() {
    let tmp = TempDir::new().unwrap();
    let report = spellings::check(tmp.path()).unwrap();
    let system = file_system();
    let yes = |value: bool| if value { "yes" } else { "no" };
    let lines: Vec<&str> = report.lines().collect();
    let header = edit_overlap::replay::header(tmp.path()).unwrap();
    assert!(report.starts_with(&header));
    let system_line = format!(
        "File system: case-insensitive: {}; Unicode normalization-insensitive: {}",
        yes(system.case_insensitive),
        yes(system.normalization_insensitive)
    );
    assert_eq!(lines[3], system_line);
    assert!(lines[4].starts_with("`git init` sets: `core.ignorecase` "));
    // Each table with its header and its rows.
    let rows = |title: &str| table(&report, title);
    let rename = rows("Case-only rename of `README.md` to `Readme.md`");
    assert_eq!(rename.len(), 2 + 4);
    assert_eq!(
        rename[2],
        "| true | git mv | `README.md`, `Readme.md` | 1 |"
    );
    let names = rows("File names in NFC and NFD: `café.md`, by the form of each name");
    assert_eq!(names.len(), 2 + 6);
    assert_eq!(names[2], "| true | written in NFC | NFC | NFC | 1 |");
    assert_eq!(
        names[7],
        "| false | checked out from a commit in NFD | none | NFD | 0 |"
    );
    let tools = rows("Paths an agent's tools give");
    assert_eq!(
        tools[2],
        "| relative | `src/inventory.rs` | `src/inventory.rs` | yes |"
    );
    let outside =
        "| outside the repository | `src/inventory.rs` | no worktree of the repository | no |";
    assert!(tools.contains(&outside));
    assert_eq!(
        rows("Line endings of one edit")[2..],
        [
            "| true | CRLF | LF | 4 | 4 | 4 |",
            "| input | LF | LF | 4 | 4 | 4 |",
            "| false | LF | CRLF | 4 | 13 | 4 |",
        ]
    );
}

/// `src/store.rs` of [`lf_and_crlf_versions_of_one_edit_give_the_same_hunk_and_symbol_fingerprints`]:
/// a type whose `impl` block names it over three lines, with `ending` ending each line.
fn store(ending: &str, reserved: u32) -> String {
    let lines = [
        "pub struct Store<T>(T);",
        "",
        "impl Store<",
        "    u32,",
        "> {",
        "    pub fn reserve(&self) -> u32 {",
        &format!("        let reserved = {reserved};"),
        "        reserved",
        "    }",
        "}",
    ];
    lines.map(|line| format!("{line}{ending}")).concat()
}

#[test]
fn lf_and_crlf_versions_of_one_edit_give_the_same_hunk_and_symbol_fingerprints() {
    let tmp = TempDir::new().unwrap();
    // Stored with CRLF; a later commit upstream stores it with LF, as `git add --renormalize`
    // does once `.gitattributes` makes it text.
    let repo = Repo::new(&tmp, "repo", &[("src/store.rs", &store("\r\n", 1))]);
    let (a, a_client) = repo.machine("alice-laptop");
    let (maintainer, _) = repo.machine("maintainer");
    maintainer.write("src/store.rs", &store("\n", 1)).unwrap();
    maintainer.commit("store with LF").unwrap();
    maintainer
        .git(&["push", "--quiet", "origin", "HEAD:main"])
        .unwrap();
    let (b, b_client) = repo.machine("bob-desktop");
    a.write("src/store.rs", &store("\r\n", 2)).unwrap();
    b.write("src/store.rs", &store("\n", 2)).unwrap();

    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    let [a_edits, b_edits] = <[_; 2]>::try_from(space.publish()).unwrap();
    // One path, the two boundaries of line 7, and `reserve` in the `impl` block.
    assert_eq!(a_edits.sizes(), [1, 2, 1]);
    assert_eq!(
        a_edits.publication.fingerprints,
        b_edits.publication.fingerprints
    );
    let seen = space.seen("alice-laptop");
    assert_eq!(
        seen.hunks,
        hunks(&[("bob-desktop", lines(&[("src/store.rs", &[7..8])]))])
    );
    let symbols: Vec<&BTreeSet<String>> = seen.symbols["bob-desktop"].values().collect();
    assert_eq!(
        symbols,
        [&BTreeSet::from([
            "impl_item Store<\r\n    u32,\r\n> > function_item reserve".to_owned()
        ])]
    );
}

#[test]
fn machines_with_macos_windows_and_linux_spellings_of_one_file_overlap() {
    let names = ["linux-box", "mac-laptop", "windows-desktop"];
    let clones = Clones::new(Repo::create, names);
    let [linux, mac, windows] = &clones.machines;
    // Each adds one new file as its system spells it: in NFC; in NFD, which git keeps on macOS
    // without `core.precomposeunicode`; in another case, which NTFS takes for one file. Each edits
    // one line of a tracked file, Windows with CRLF line endings and `core.autocrlf`.
    let spellings = [
        (linux, "docs/caf\u{e9}.md"),
        (mac, "docs/cafe\u{301}.md"),
        (windows, "docs/CAF\u{c9}.md"),
    ];
    mac.git(&["config", "core.precomposeunicode", "false"])
        .unwrap();
    windows.git(&["config", "core.autocrlf", "true"]).unwrap();
    let edited = common::inventory(&[(7, "    let reserved = 3;")]);
    for (machine, path) in spellings {
        machine.write(path, "# Café\n").unwrap();
        let edited = if machine.dir() == windows.dir() {
            edited.replace('\n', "\r\n")
        } else {
            edited.clone()
        };
        machine.write("src/inventory.rs", &edited).unwrap();
    }

    let space = clones.published();
    for (machine, path) in spellings {
        let name = &space
            .machines
            .iter()
            .find(|(m, _)| m.dir() == machine.dir())
            .unwrap()
            .1
            .machine;
        let edits = space.edits_of(name);
        assert_eq!(edits.paths(), BTreeSet::from([path, "src/inventory.rs"]));
        let seen = space.seen(name);
        let others: Vec<&str> = spellings
            .iter()
            .filter(|(other, _)| other.dir() != machine.dir())
            .map(|(other, _)| other.dir().file_name().unwrap().to_str().unwrap())
            .collect();
        let both = [path, "src/inventory.rs"];
        assert_eq!(
            seen.paths,
            paths(&[(others[0], &both), (others[1], &both)]),
            "{name}"
        );
        let changed = lines(&[(path, &[1..2]), ("src/inventory.rs", &[7..8])]);
        assert_eq!(
            seen.hunks,
            hunks(&[(others[0], changed.clone()), (others[1], changed)]),
            "{name}"
        );
    }
}
