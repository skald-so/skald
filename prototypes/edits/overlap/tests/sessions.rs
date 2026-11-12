//! The scripted sessions of `docs/dev/edit-prototype.md`, "Edits via Bash and generators", on the
//! test repository: what each step changes and names, what each source reports and when it learns
//! of each change, and the command's tools and tables.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use common::Repo;
use edit_overlap::git;
use edit_overlap::replay::Repository;
use edit_overlap::sessions::{self, Kind, Played, Session, Source, Step, When};
use edit_overlap::tools::{self, Environment, Generator, Pins};
use tempfile::TempDir;

/// The fixture's session: edit tool calls, a restore, `sed` on named files and on a glob, a move,
/// a deletion, and a generator that writes a tracked file and an ignored one.
const FIXTURE: Session = Session {
    repository: "fixture",
    setup: &[],
    steps: &[
        Step::Edit {
            path: "src/inventory.rs",
            old: "    let items = 0;",
            new: "    let items = 3;",
        },
        Step::Edit {
            path: "src/warehouse.rs",
            old: "Warehouse;",
            new: "Warehouse(u32);",
        },
        Step::Git {
            kind: Kind::Restore,
            args: &["checkout", "--", "src/warehouse.rs"],
        },
        Step::Shell {
            kind: Kind::InPlaceEdit,
            dir: "",
            line: "sed -i.bak -e 's/Guide/Handbook/' -e 's/let items = 3/let items = 4/' docs/guide.md \
                src/inventory.rs && rm docs/guide.md.bak src/inventory.rs.bak",
        },
        Step::Shell {
            kind: Kind::InPlaceEdit,
            dir: "",
            line: "sed -i.bak -e 's/the code/the source/' docs/*.md && rm docs/*.md.bak",
        },
        Step::Shell {
            kind: Kind::Move,
            dir: "docs",
            line: "mv faq.md questions.md",
        },
        Step::Shell {
            kind: Kind::Deletion,
            dir: "",
            line: "rm src/legacy.rs",
        },
        Step::Shell {
            kind: Kind::Generator,
            dir: "",
            line: "sh {scratch}/generate",
        },
        Step::Edit {
            path: "docs/questions.md",
            old: "# FAQ",
            new: "# Questions",
        },
        Step::Write {
            path: "src/stock.rs",
            contents: "pub struct Stock;\n",
        },
    ],
};

/// The generator of [`FIXTURE`], in the scratch directory: a tracked file and an ignored one.
const GENERATE: &str = "mkdir -p target\n\
    printf 'pub const ITEMS: u32 = 4;\\n' > src/schema.rs\n\
    printf 'generated\\n' > target/schema.txt\n";

/// The test repository's extra files, which the fixture moves and deletes.
const EXTRA: [(&str, &str); 2] = [
    ("docs/faq.md", "# FAQ\n\nRead the code.\n"),
    ("src/legacy.rs", "pub fn old() {}\n"),
];

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// The test repository with [`EXTRA`] as a measured repository named `fixture`, fetched from its
/// upstream.
fn repository(tmp: &TempDir) -> Repository {
    let repo = Repo::create_with(tmp, &EXTRA);
    let seed = tmp.path().join("repo").join("seed");
    let upstream = tmp.path().join("repo").join("upstream.git");
    drop(repo);
    Repository {
        name: "fixture",
        url: leak(upstream.to_str().unwrap().to_owned()),
        commit: leak(git::line(&seed, &["rev-parse", "HEAD"]).unwrap()),
        merges: 0,
    }
}

/// [`FIXTURE`] played on a clone of the test repository, with the system's shell commands.
fn played(tmp: &TempDir) -> Played {
    let dir = tmp.path().join("clone");
    sessions::checkout(&dir, &repository(tmp)).unwrap();
    let scratch = tmp.path().join("scratch");
    fs::create_dir(&scratch).unwrap();
    fs::write(scratch.join("generate"), GENERATE).unwrap();
    let env = Environment::system().unwrap();
    sessions::play(&dir, &scratch, &FIXTURE, &env, |_| ()).unwrap()
}

fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|path| path.to_string()).collect()
}

/// The files the fixture changes by the end: all but the restored `src/warehouse.rs` and the
/// ignored `target/schema.txt`.
const CHANGED: [&str; 7] = [
    "docs/faq.md",
    "docs/guide.md",
    "docs/questions.md",
    "src/inventory.rs",
    "src/legacy.rs",
    "src/schema.rs",
    "src/stock.rs",
];

#[test]
fn checkout_fetches_the_repository_at_its_commit_with_an_empty_edit_set() {
    let tmp = TempDir::new().unwrap();
    let repository = repository(&tmp);
    let dir = tmp.path().join("clone");
    sessions::checkout(&dir, &repository).unwrap();
    assert_eq!(
        git::line(&dir, &["rev-parse", "HEAD"]).unwrap(),
        repository.commit
    );
    assert!(git::edit_set(&dir).unwrap().is_empty());
    assert!(dir.join("docs/faq.md").is_file());
}

#[test]
fn each_step_gives_the_files_it_changes_and_the_files_it_names() {
    let tmp = TempDir::new().unwrap();
    let played = played(&tmp);
    let steps: Vec<(Kind, BTreeSet<String>, BTreeSet<String>)> = played
        .steps
        .iter()
        .map(|ran| (ran.step.kind(), ran.changes.clone(), ran.named.clone()))
        .collect();
    let inventory = set(&["src/inventory.rs"]);
    let warehouse = set(&["src/warehouse.rs"]);
    let moved = set(&["docs/faq.md", "docs/questions.md"]);
    assert_eq!(
        steps,
        [
            (Kind::EditTool, inventory.clone(), inventory),
            (Kind::EditTool, warehouse.clone(), warehouse.clone()),
            (Kind::Restore, warehouse.clone(), warehouse),
            (
                Kind::InPlaceEdit,
                set(&["docs/guide.md", "src/inventory.rs"]),
                set(&["docs/guide.md", "src/inventory.rs"])
            ),
            // A glob names no file: the shell expands it.
            (
                Kind::InPlaceEdit,
                set(&["docs/faq.md", "docs/guide.md"]),
                set(&[])
            ),
            // Named relative to its directory, before and after the move.
            (Kind::Move, moved.clone(), moved),
            (
                Kind::Deletion,
                set(&["src/legacy.rs"]),
                set(&["src/legacy.rs"])
            ),
            // Its script is outside the worktree; its ignored output is no change.
            (Kind::Generator, set(&["src/schema.rs"]), set(&[])),
            (
                Kind::EditTool,
                set(&["docs/questions.md"]),
                set(&["docs/questions.md"])
            ),
            (
                Kind::EditTool,
                set(&["src/stock.rs"]),
                set(&["src/stock.rs"])
            ),
        ]
    );
    assert_eq!(played.changed, set(&CHANGED));
}

#[test]
fn the_diff_reports_every_changed_file_the_generators_included() {
    let tmp = TempDir::new().unwrap();
    let played = played(&tmp);
    let changed: BTreeSet<&str> = CHANGED.into();
    assert_eq!(played.report(Source::Diff), changed);
    assert_eq!(played.report(Source::DiffEachStep), changed);
    assert!(changed.contains("src/schema.rs"));
    assert!(tmp.path().join("clone/target/schema.txt").is_file());
}

#[test]
fn a_file_changed_and_restored_is_not_reported_by_the_diff() {
    let tmp = TempDir::new().unwrap();
    let played = played(&tmp);
    let restored = "src/warehouse.rs";
    // After the edit, before the restore, the diff after each step reports it.
    assert!(played.reports(Source::DiffEachStep)[1][1].contains(restored));
    for source in [Source::Diff, Source::DiffEachStep] {
        assert!(!played.report(source).contains(restored), "{source:?}");
    }
    // The hooks saw the edit tool's call, and the declarations named it.
    for source in [Source::Hooks, Source::Declarations] {
        assert!(played.report(source).contains(restored), "{source:?}");
    }
}

#[test]
fn each_source_reports_exactly_the_files_it_can_see() {
    let tmp = TempDir::new().unwrap();
    let played = played(&tmp);
    let reports: BTreeMap<String, BTreeSet<&str>> = Source::ALL
        .into_iter()
        .map(|source| (source.name(), played.report(source)))
        .collect();
    let all: BTreeSet<&str> = CHANGED.into();
    let hooks = [
        "docs/questions.md",
        "src/inventory.rs",
        "src/stock.rs",
        "src/warehouse.rs",
    ];
    let declared = [
        "docs/faq.md",
        "docs/guide.md",
        "docs/questions.md",
        "src/inventory.rs",
        "src/legacy.rs",
        "src/stock.rs",
        "src/warehouse.rs",
    ];
    let expected = BTreeMap::from([
        ("diff, every 3 steps".to_owned(), all.clone()),
        ("diff, after each step".to_owned(), all),
        ("hooks".to_owned(), hooks.into()),
        ("declarations".to_owned(), declared.into()),
    ]);
    assert_eq!(reports, expected);
}

/// A file, the step of its first change, and when each source of `Source::ALL` learns of it.
type Learned<'p> = (&'p str, usize, [When; 4]);

#[test]
fn each_source_learns_of_each_change_when_it_can() {
    use When::*;
    let tmp = TempDir::new().unwrap();
    let played = played(&tmp);
    // For each kind and every kind, each file's first change and when each source of
    // `Source::ALL` learns of it.
    let expected: [(Option<Kind>, &[Learned]); 8] = [
        (
            Some(Kind::EditTool),
            &[
                ("docs/questions.md", 8, [Before, Before, AtTheEdit, Before]),
                (
                    "src/inventory.rs",
                    0,
                    [NextScan, AtTheEdit, AtTheEdit, Before],
                ),
                ("src/stock.rs", 9, [NextScan, AtTheEdit, AtTheEdit, Before]),
            ],
        ),
        (
            Some(Kind::InPlaceEdit),
            &[
                ("docs/faq.md", 4, [NextScan, AtTheEdit, Never, LaterCall]),
                ("docs/guide.md", 3, [NextScan, AtTheEdit, Never, Before]),
                ("src/inventory.rs", 3, [Before, Before, Before, Before]),
            ],
        ),
        (
            Some(Kind::Move),
            &[
                ("docs/faq.md", 5, [NextScan, Before, Never, Before]),
                (
                    "docs/questions.md",
                    5,
                    [NextScan, AtTheEdit, LaterCall, Before],
                ),
            ],
        ),
        (
            Some(Kind::Deletion),
            &[("src/legacy.rs", 6, [NextScan, AtTheEdit, Never, Before])],
        ),
        (
            Some(Kind::Generator),
            &[("src/schema.rs", 7, [NextScan, AtTheEdit, Never, Never])],
        ),
        (Some(Kind::Restore), &[]),
        (Some(Kind::Formatter), &[]),
        (
            None,
            &[
                ("docs/faq.md", 4, [NextScan, AtTheEdit, Never, LaterCall]),
                ("docs/guide.md", 3, [NextScan, AtTheEdit, Never, Before]),
                (
                    "docs/questions.md",
                    5,
                    [NextScan, AtTheEdit, LaterCall, Before],
                ),
                (
                    "src/inventory.rs",
                    0,
                    [NextScan, AtTheEdit, AtTheEdit, Before],
                ),
                ("src/legacy.rs", 6, [NextScan, AtTheEdit, Never, Before]),
                ("src/schema.rs", 7, [NextScan, AtTheEdit, Never, Never]),
                ("src/stock.rs", 9, [NextScan, AtTheEdit, AtTheEdit, Before]),
            ],
        ),
    ];
    for (kind, files) in expected {
        let first = played.first_changes(kind);
        let found: Vec<Learned> = first
            .iter()
            .map(|(path, step)| {
                let when = Source::ALL.map(|source| played.when(source, *step, path));
                (*path, *step, when)
            })
            .collect();
        assert_eq!(found, files, "{kind:?}");
        for (i, source) in Source::ALL.into_iter().enumerate() {
            let never = files.iter().filter(|(_, _, when)| when[i] == Never).count();
            assert_eq!(played.missed(source, kind), never, "{kind:?} {source:?}");
        }
    }
}

#[test]
fn the_diff_on_its_schedule_scans_every_3_steps_and_after_the_last() {
    let tmp = TempDir::new().unwrap();
    let played = played(&tmp);
    let reports = played.reports(Source::Diff);
    let scanned: Vec<usize> = (0..reports.len())
        .filter(|&i| {
            reports[i][1]
                == played.steps[i]
                    .edit_set
                    .iter()
                    .map(String::as_str)
                    .collect()
        })
        .filter(|&i| reports[i][0] != reports[i][1])
        .collect();
    assert_eq!(sessions::SCAN_EVERY, 3);
    assert_eq!(scanned, [2, 5, 8, 9]);
    assert!(reports[0][1].is_empty() && reports[1][1].is_empty());
}

#[test]
fn report_gives_the_tables_of_the_sessions() {
    let tmp = TempDir::new().unwrap();
    let played = played(&tmp);
    let versions = [("rustc".to_owned(), "rustc 1.0.0".to_owned())];
    let report = sessions::report(&versions, &[played]);
    for row in [
        "| rustc | rustc 1.0.0 |",
        "| fixture | 1 | edit tool | edit `src/inventory.rs` | `src/inventory.rs` | `src/inventory.rs` |",
        "| fixture | 3 | restore | `git checkout -- src/warehouse.rs` | `src/warehouse.rs` | `src/warehouse.rs` |",
        "| fixture | 5 | in-place edit | `sed -i.bak -e 's/the code/the source/' docs/*.md && rm docs/*.md.bak` \
            | `docs/faq.md`, `docs/guide.md` | none |",
        "| fixture | 6 | move | in `docs`: `mv faq.md questions.md` | `docs/faq.md`, `docs/questions.md` \
            | `docs/faq.md`, `docs/questions.md` |",
        "| fixture | 10 | edit tool | write `src/stock.rs` | `src/stock.rs` | `src/stock.rs` |",
        "| Repository | Kind | Files | diff, every 3 steps | diff, after each step | hooks | declarations |",
        "| fixture | edit tool | 3 | 0 (0%) | 0 (0%) | 0 (0%) | 0 (0%) |",
        "| fixture | in-place edit | 3 | 0 (0%) | 0 (0%) | 2 (67%) | 0 (0%) |",
        "| fixture | move | 2 | 0 (0%) | 0 (0%) | 1 (50%) | 0 (0%) |",
        "| fixture | deletion | 1 | 0 (0%) | 0 (0%) | 1 (100%) | 0 (0%) |",
        "| fixture | code generator | 1 | 0 (0%) | 0 (0%) | 1 (100%) | 1 (100%) |",
        "| fixture | restore | 0 | - | - | - | - |",
        "| fixture | any | 7 | 0 (0%) | 0 (0%) | 4 (57%) | 1 (14%) |",
        "| Repository | Source | Files | Before the edit | At the edit | At the next scan \
            | At a later tool call | Never | Reported, not changed at the end |",
        "| fixture | diff, every 3 steps | 7 | 0 | 0 | 7 | 0 | 0 | 0 |",
        "| fixture | diff, after each step | 7 | 0 | 7 | 0 | 0 | 0 | 0 |",
        "| fixture | hooks | 7 | 0 | 2 | 0 | 1 | 4 | 1 |",
        "| fixture | declarations | 7 | 5 | 0 | 0 | 1 | 1 | 1 |",
    ] {
        assert!(report.lines().any(|line| line == row), "{row}\n{report}");
    }
    assert!(
        !report.contains("| fixture | formatter |"),
        "a kind without steps has no row"
    );
}

#[test]
fn a_pipe_in_a_command_is_escaped_in_the_table() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("clone");
    sessions::checkout(&dir, &repository(&tmp)).unwrap();
    let session = Session {
        repository: "fixture",
        setup: &[],
        steps: &[Step::Shell {
            kind: Kind::InPlaceEdit,
            dir: "",
            line: "ls src | sed -e 's/x/y/'",
        }],
    };
    let env = Environment::system().unwrap();
    let played = sessions::play(&dir, tmp.path(), &session, &env, |_| ()).unwrap();
    let report = sessions::report(&[], &[played]);
    assert!(
        report.contains("| `ls src \\| sed -e 's/x/y/'` | none | none |"),
        "{report}"
    );
}

/// Plays `session` on a clone of the test repository with the system's shell commands.
fn play(tmp: &TempDir, session: &Session) -> edit_overlap::Result<Played> {
    let dir = tmp.path().join("clone");
    sessions::checkout(&dir, &repository(tmp)).unwrap();
    let env = Environment::system().unwrap();
    sessions::play(&dir, tmp.path(), session, &env, |_| ())
}

#[test]
fn a_setup_that_changes_the_worktree_fails() {
    let setup = |setup| Session {
        repository: "fixture",
        setup,
        steps: &[],
    };
    let tmp = TempDir::new().unwrap();
    assert!(play(&tmp, &setup(&["touch src/new.rs"])).is_err());
    let tmp = TempDir::new().unwrap();
    assert!(
        play(&tmp, &setup(&["touch target"]))
            .unwrap()
            .steps
            .is_empty()
    );
}

#[test]
fn an_edit_of_text_the_file_does_not_hold_once_or_a_failing_command_fails() {
    let edit = |old: &'static str| Session {
        repository: "fixture",
        setup: &[],
        steps: Box::leak(Box::new([Step::Edit {
            path: "docs/faq.md",
            old,
            new: "x",
        }])),
    };
    // `docs/faq.md` holds "e" more than once.
    for old in ["e", "no such text"] {
        let tmp = TempDir::new().unwrap();
        assert!(play(&tmp, &edit(old)).is_err(), "{old}");
    }
    let tmp = TempDir::new().unwrap();
    assert_eq!(
        play(&tmp, &edit("# FAQ")).unwrap().changed,
        set(&["docs/faq.md"])
    );
    let tmp = TempDir::new().unwrap();
    let failing = Session {
        repository: "fixture",
        setup: &[],
        steps: &[Step::Shell {
            kind: Kind::Deletion,
            dir: "",
            line: "rm no-such-file",
        }],
    };
    let error = play(&tmp, &failing).unwrap_err().to_string();
    assert!(error.contains("rm no-such-file"), "{error}");
}

/// The first line `--version` prints for the toolchain's `name`.
fn version(bin: &Path, name: &str, tmp: &Path) -> &'static str {
    let env = Environment::system().unwrap();
    let mut command = env.command(bin.join(name), tmp);
    command.arg("--version");
    let printed = tools::output(command, name).unwrap();
    leak(printed.lines().next().unwrap().to_owned())
}

/// A generator crate in `dir`: its `--version` prints a header and `Version: 1.0.0`; with
/// `generate`, it writes `src/schema.rs`.
fn generator_crate(dir: &Path) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"fixture-generator\"\nversion = \"1.0.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("src/main.rs"),
        "fn main() {\n    match std::env::args().nth(1).as_deref() {\n        \
        Some(\"--version\") => println!(\"fixture-generator\\n Version: 1.0.0\"),\n        \
        _ => std::fs::write(\"src/schema.rs\", \"pub struct Schema;\\n\").unwrap(),\n    }\n}\n",
    )
    .unwrap();
}

/// A session that runs the generator and writes the environment its shell commands get, without
/// values, to the scratch directory.
const GENERATED: Session = Session {
    repository: "fixture",
    setup: &["cargo --version"],
    steps: &[
        Step::Shell {
            kind: Kind::Generator,
            dir: "",
            line: "fixture-generator generate",
        },
        Step::Shell {
            kind: Kind::Formatter,
            dir: "",
            line: "rustfmt --edition 2024 src/schema.rs && env > {scratch}/environment",
        },
    ],
};

#[test]
fn run_sets_up_the_pinned_tools_and_plays_each_session_on_its_repository() {
    let fixture = TempDir::new().unwrap();
    let repository = repository(&fixture);
    let crate_dir = fixture.path().join("generator");
    generator_crate(&crate_dir);
    let tmp = TempDir::new().unwrap();
    let bin = tools::toolchain(tmp.path()).unwrap();
    let pins = Pins {
        rustc: version(&bin, "rustc", tmp.path()),
        cargo: version(&bin, "cargo", tmp.path()),
        rustfmt: version(&bin, "rustfmt", tmp.path()),
    };
    let install: &'static [&'static str] = Box::leak(Box::new([
        "--path",
        leak(crate_dir.to_str().unwrap().to_owned()),
        "--offline",
    ]));
    let generator = Generator {
        install,
        binary: "fixture-generator",
        version: "Version: 1.0.0",
    };
    let report = sessions::run(
        tmp.path(),
        &pins,
        &generator,
        &[repository],
        &[GENERATED],
        |_, _| (),
    )
    .unwrap();
    let rows = [
        "| fixture | 1 | code generator | `fixture-generator generate` | `src/schema.rs` | none |",
        "| fixture-generator | Version: 1.0.0 |",
    ];
    for row in rows {
        assert!(report.lines().any(|line| line == row), "{row}\n{report}");
    }
    let version_rows = report.lines().filter(|line| line.starts_with("| ")).take(6);
    let tools: Vec<&str> = version_rows
        .filter_map(|line| line.split(" | ").next())
        .collect();
    assert_eq!(
        tools,
        [
            "| Tool",
            "| rustc",
            "| cargo",
            "| rustfmt",
            "| fixture-generator",
            "| the shell and its commands"
        ]
    );
    assert!(
        report.starts_with("Machine: "),
        "the report starts with the header"
    );
    // The shell commands get the toolchain's, the generator's and the system's `PATH`, and a
    // cargo cache in the command's directory; nothing else of the test's environment, but what
    // the shell sets itself.
    let environment = fs::read_to_string(tmp.path().join("fixture-scratch/environment")).unwrap();
    let environment: BTreeMap<&str, &str> = environment
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let shell = ["PWD", "SHLVL", "_", "OLDPWD"];
    let names: BTreeSet<&str> = environment
        .keys()
        .copied()
        .filter(|name| !shell.contains(name))
        .collect();
    assert_eq!(names, BTreeSet::from(["CARGO_HOME", "PATH"]));
    let cargo_home = tmp.path().join("cargo");
    assert!(
        environment["CARGO_HOME"] == cargo_home.to_str().unwrap(),
        "CARGO_HOME is the command's own cache"
    );
    let path: Vec<String> = std::env::split_paths(environment["PATH"])
        .map(|dir| dir.display().to_string())
        .collect();
    let generator_bin = tmp.path().join("generator/bin");
    let expected = [
        bin.display().to_string(),
        generator_bin.display().to_string(),
        "/usr/bin".into(),
        "/bin".into(),
    ];
    assert!(path == expected, "PATH of the shell commands");
}

#[test]
fn a_tool_of_another_version_is_refused() {
    let fixture = TempDir::new().unwrap();
    let repository = repository(&fixture);
    let crate_dir = fixture.path().join("generator");
    generator_crate(&crate_dir);
    let install: &'static [&'static str] = Box::leak(Box::new([
        "--path",
        leak(crate_dir.to_str().unwrap().to_owned()),
        "--offline",
    ]));
    let probe = TempDir::new().unwrap();
    let bin = tools::toolchain(probe.path()).unwrap();
    let versions = ["rustc", "cargo", "rustfmt"].map(|name| version(&bin, name, probe.path()));
    let run = |pins: [&'static str; 3], generator_version| {
        let tmp = TempDir::new().unwrap();
        let [rustc, cargo, rustfmt] = pins;
        let generator = Generator {
            install,
            binary: "fixture-generator",
            version: generator_version,
        };
        let pins = Pins {
            rustc,
            cargo,
            rustfmt,
        };
        let sessions = [Session {
            repository: "fixture",
            setup: &[],
            steps: &[],
        }];
        let repositories = std::slice::from_ref(&repository);
        sessions::run(
            tmp.path(),
            &pins,
            &generator,
            repositories,
            &sessions,
            |_, _| (),
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    };
    for i in 0..3 {
        let mut pins = versions;
        pins[i] = "0.0.0";
        let error = run(pins, "Version: 1.0.0").unwrap_err();
        assert!(error.contains("not the pinned"), "tool {i}");
    }
    let error = run(versions, "Version: 9.9.9").unwrap_err();
    assert!(
        error.contains("fixture-generator"),
        "the generator's version"
    );
}

#[test]
fn a_session_on_a_repository_that_is_not_measured_fails() {
    let tmp = TempDir::new().unwrap();
    let pins = Pins {
        rustc: "",
        cargo: "",
        rustfmt: "",
    };
    let generator = Generator {
        install: &[],
        binary: "",
        version: "",
    };
    let error = sessions::run(tmp.path(), &pins, &generator, &[], &[FIXTURE], |_, _| ())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("fixture is not a measured repository"),
        "{error}"
    );
    assert!(
        fs::read_dir(tmp.path()).unwrap().next().is_none(),
        "nothing is set up"
    );
}
