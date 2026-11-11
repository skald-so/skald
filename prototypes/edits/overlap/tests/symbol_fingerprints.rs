//! The symbol fingerprint scenarios of `docs/dev/edit-prototype.md`, "Scenarios", in each
//! supported language: machines also publish fingerprints of the functions, methods and types
//! their changed lines are in, and those match when both change one, whether or not
//! `git merge-tree` conflicts.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{Repo, Seen, Space, assert_holds_none, bytes, names, paths, symbols};
use tempfile::TempDir;

/// A file in a supported language: a function, and a type with two methods, the first of the
/// function's name.
struct Fixture {
    path: &'static str,
    lines: &'static [&'static str],
    /// What starts a comment to the end of the line.
    comment: &'static str,
    function: Item,
    methods: [Item; 2],
}

/// A function or method of a [`Fixture`]: its symbol, and its first and last lines, 1-based. Its
/// body is the three lines after its first.
struct Item {
    symbol: &'static str,
    first: usize,
    last: usize,
}

const RUST: Fixture = Fixture {
    path: "src/store.rs",
    lines: &[
        "pub fn reserve(store: &mut Store) -> u32 {",
        "    let wanted = 1;",
        "    store.items -= wanted;",
        "    wanted",
        "}",
        "",
        "pub struct Store {",
        "    items: u32,",
        "}",
        "",
        "impl Store {",
        "    pub fn reserve(&mut self) -> u32 {",
        "        let wanted = 1;",
        "        self.items -= wanted;",
        "        wanted",
        "    }",
        "",
        "    pub fn release(&mut self) -> u32 {",
        "        let freed = 1;",
        "        self.items += freed;",
        "        freed",
        "    }",
        "}",
    ],
    comment: "//",
    function: Item {
        symbol: "function_item reserve",
        first: 1,
        last: 5,
    },
    methods: [
        Item {
            symbol: "impl_item Store > function_item reserve",
            first: 12,
            last: 16,
        },
        Item {
            symbol: "impl_item Store > function_item release",
            first: 18,
            last: 22,
        },
    ],
};

const TYPESCRIPT: Fixture = Fixture {
    path: "src/store.ts",
    lines: &[
        "export function reserve(store: Store): number {",
        "  const wanted = 1;",
        "  store.items -= wanted;",
        "  return wanted;",
        "}",
        "",
        "export class Store {",
        "  items = 0;",
        "",
        "  reserve(): number {",
        "    const wanted = 1;",
        "    this.items -= wanted;",
        "    return wanted;",
        "  }",
        "",
        "  release(): number {",
        "    const freed = 1;",
        "    this.items += freed;",
        "    return freed;",
        "  }",
        "}",
    ],
    comment: "//",
    function: Item {
        symbol: "function_declaration reserve",
        first: 1,
        last: 5,
    },
    methods: [
        Item {
            symbol: "class_declaration Store > method_definition reserve",
            first: 10,
            last: 14,
        },
        Item {
            symbol: "class_declaration Store > method_definition release",
            first: 16,
            last: 20,
        },
    ],
};

const PYTHON: Fixture = Fixture {
    path: "src/store.py",
    lines: &[
        "def reserve(store):",
        "    wanted = 1",
        "    store.items -= wanted",
        "    return wanted",
        "",
        "",
        "class Store:",
        "    items = 0",
        "",
        "    def reserve(self):",
        "        wanted = 1",
        "        self.items -= wanted",
        "        return wanted",
        "",
        "    def release(self):",
        "        freed = 1",
        "        self.items += freed",
        "        return freed",
    ],
    comment: "#",
    function: Item {
        symbol: "function_definition reserve",
        first: 1,
        last: 4,
    },
    methods: [
        Item {
            symbol: "class_definition Store > function_definition reserve",
            first: 10,
            last: 13,
        },
        Item {
            symbol: "class_definition Store > function_definition release",
            first: 15,
            last: 18,
        },
    ],
};

/// A file in a language without symbol fingerprints, Go, with a function whose body is lines 4
/// to 6.
const GO: (&str, &str) = (
    "src/store.go",
    "package store\n\nfunc Reserve(items int) int {\n\twanted := 1\n\titems -= wanted\n\treturn wanted\n}\n",
);

impl Fixture {
    /// The file with a comment of `machine` at the end of each of `lines`, 1-based.
    fn edited(&self, lines: &[usize], machine: &str) -> String {
        edited(self.lines, lines, self.comment, machine)
    }

    /// The file with its function moved to its end, and the line its function starts on there.
    fn moved(&self) -> (Vec<&'static str>, usize) {
        let function = &self.lines[self.function.first - 1..self.function.last];
        let rest = self.lines[self.function.last..]
            .iter()
            .skip_while(|line| line.is_empty());
        let mut lines: Vec<&str> = rest.copied().collect();
        lines.push("");
        let first = lines.len() + 1;
        lines.extend(function);
        (lines, first)
    }

    /// A repository with the file and `extra` files, each a path and its contents.
    fn repo(&self, tmp: &TempDir, extra: &[(&str, &str)]) -> Repo {
        let text = self.edited(&[], "");
        let files = [(self.path, text.as_str())];
        Repo::create_with(tmp, &[&files[..], extra].concat())
    }
}

/// `lines` with a comment of `machine`, started with `comment`, at the end of each of `edited`,
/// 1-based.
fn edited(lines: &[&str], edited: &[usize], comment: &str, machine: &str) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| match edited.contains(&(i + 1)) {
            true => format!("{line} {comment} {machine}\n"),
            false => format!("{line}\n"),
        })
        .collect()
}

/// What `machine` learns of `other`: the path of `fixture`, without hunks, with `functions`, the
/// symbols both change.
fn file_and(fixture: &Fixture, other: &str, functions: &[&str]) -> Seen {
    Seen {
        paths: paths(&[(other, &[fixture.path])]),
        hunks: BTreeMap::new(),
        symbols: match functions {
            [] => BTreeMap::new(),
            _ => symbols(&[(other, fixture.path, functions)]),
        },
    }
}

/// The function and the first method share a name; edits to them overlap at file level only.
fn different_functions(fixture: &Fixture) {
    let tmp = TempDir::new().unwrap();
    let repo = fixture.repo(&tmp, &[]);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let function = fixture.function.first + 2;
    let method = fixture.methods[0].first + 2;
    a.write(fixture.path, &fixture.edited(&[function], "alice"))
        .unwrap();
    a.commit("reserve").unwrap();
    b.write(fixture.path, &fixture.edited(&[method], "bob"))
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    assert_eq!(
        space.seen("alice-laptop"),
        file_and(fixture, "bob-desktop", &[])
    );
    assert_eq!(
        space.seen("bob-desktop"),
        file_and(fixture, "alice-laptop", &[])
    );
}

/// The first and the last line of the first method's body: they overlap at symbol level and
/// merge cleanly.
fn distant_lines_of_one_function(fixture: &Fixture) {
    let tmp = TempDir::new().unwrap();
    let repo = fixture.repo(&tmp, &[]);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let method = &fixture.methods[0];
    a.write(fixture.path, &fixture.edited(&[method.first + 1], "alice"))
        .unwrap();
    a.commit("reserve").unwrap();
    b.write(fixture.path, &fixture.edited(&[method.first + 3], "bob"))
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    for (machine, other) in [
        ("alice-laptop", "bob-desktop"),
        ("bob-desktop", "alice-laptop"),
    ] {
        assert_eq!(
            space.seen(machine),
            file_and(fixture, other, &[method.symbol])
        );
        assert!(space.check(machine)[other].clean, "{machine}");
    }
}

/// The middle lines of the two methods of one type: they overlap at file level only.
fn two_methods_of_one_type(fixture: &Fixture) {
    let tmp = TempDir::new().unwrap();
    let repo = fixture.repo(&tmp, &[]);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let [first, second] = &fixture.methods;
    a.write(fixture.path, &fixture.edited(&[first.first + 2], "alice"))
        .unwrap();
    b.write(fixture.path, &fixture.edited(&[second.first + 2], "bob"))
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    assert_eq!(
        space.seen("alice-laptop"),
        file_and(fixture, "bob-desktop", &[])
    );
    assert_eq!(
        space.seen("bob-desktop"),
        file_and(fixture, "alice-laptop", &[])
    );
}

/// The function moved to the end of the file upstream; one machine pulls it and edits the first
/// line of its body there, the other does not and edits the last one in its old place.
fn moved_symbol(fixture: &Fixture) {
    let tmp = TempDir::new().unwrap();
    let repo = fixture.repo(&tmp, &[]);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let (maintainer, _) = repo.machine("maintainer");
    let (moved, first) = fixture.moved();
    maintainer
        .write(fixture.path, &edited(&moved, &[], "", ""))
        .unwrap();
    maintainer.commit("move the function").unwrap();
    maintainer.push("main").unwrap();
    a.pull().unwrap();
    a.write(
        fixture.path,
        &edited(&moved, &[first + 1], fixture.comment, "alice"),
    )
    .unwrap();
    let last = fixture.function.first + 3;
    b.write(fixture.path, &fixture.edited(&[last], "bob"))
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    let function = &[fixture.function.symbol][..];
    assert_eq!(
        space.seen("alice-laptop"),
        file_and(fixture, "bob-desktop", function)
    );
    assert_eq!(
        space.seen("bob-desktop"),
        file_and(fixture, "alice-laptop", function)
    );
}

/// Distant lines of one function edited in the fixture and in a Go file: only the fixture's
/// overlap at symbol level.
fn file_in_another_language(fixture: &Fixture) {
    let tmp = TempDir::new().unwrap();
    let (go, go_text) = GO;
    let repo = fixture.repo(&tmp, &[GO]);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let go_lines: Vec<&str> = go_text.lines().collect();
    let method = &fixture.methods[0];
    for (machine, who, offset, go_line) in [(&a, "alice", 1, 4), (&b, "bob", 3, 6)] {
        let text = fixture.edited(&[method.first + offset], who);
        machine.write(fixture.path, &text).unwrap();
        machine
            .write(go, &edited(&go_lines, &[go_line], "//", who))
            .unwrap();
    }
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    for (machine, other) in [
        ("alice-laptop", "bob-desktop"),
        ("bob-desktop", "alice-laptop"),
    ] {
        let edits = space.edits_of(machine);
        let files: BTreeSet<String> = edits.symbols().into_keys().collect();
        assert_eq!(files, BTreeSet::from([fixture.path.into()]), "{machine}");
        assert_eq!(
            space.seen(machine),
            Seen {
                paths: paths(&[(other, &[fixture.path, go])]),
                hunks: BTreeMap::new(),
                symbols: symbols(&[(other, fixture.path, &[method.symbol])]),
            }
        );
        // The two paths and the method: nothing of the Go function.
        let overlaps = space.server.overlaps(machine);
        assert_eq!(overlaps[0].fingerprints.len(), 3, "{machine}");
    }
}

/// Every symbol of the fixture changed on two machines: nothing the server receives holds the
/// kind or the name of one.
fn no_symbol_name_reaches_the_server(fixture: &Fixture) {
    let tmp = TempDir::new().unwrap();
    let repo = fixture.repo(&tmp, &[]);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let [first, second] = &fixture.methods;
    let a_lines = [fixture.function.first + 1, first.first + 1];
    a.write(fixture.path, &fixture.edited(&a_lines, "alice"))
        .unwrap();
    a.commit("reserve").unwrap();
    let b_lines = [first.first + 3, second.first + 1];
    b.write(fixture.path, &fixture.edited(&b_lines, "bob"))
        .unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();
    assert!(!space.seen("alice-laptop").symbols.is_empty());

    let mut needles = BTreeSet::new();
    for edits in &space.edits {
        for symbol in edits.symbols().values().flatten() {
            needles.extend(
                symbol
                    .0
                    .iter()
                    .flat_map(|(kind, name)| [kind.clone(), name.clone()]),
            );
        }
    }
    assert!(needles.contains("Store") && needles.contains("release"));
    let received = space.server.received().iter();
    let bytes =
        bytes(received.map(|publication| (&publication.machine, &publication.fingerprints)));
    assert_holds_none(
        &bytes,
        needles.iter().map(String::as_str),
        "what the server received",
    );
}

#[test]
fn insertions_deletions_and_added_and_deleted_files_get_symbol_fingerprints() {
    let tmp = TempDir::new().unwrap();
    let repo = RUST.repo(&tmp, &[]);
    let (a, a_client) = repo.machine("alice-laptop");
    let (b, b_client) = repo.machine("bob-desktop");
    let method = &RUST.methods[0];
    // A line inserted after the first line of the method's body, and a deleted file: the merge
    // base alone holds the deleted file's symbol.
    let mut inserted = RUST.lines.to_vec();
    inserted.insert(method.first + 1, "        // Reserve one.");
    a.write(RUST.path, &edited(&inserted, &[], "", "")).unwrap();
    a.delete("src/warehouse.rs").unwrap();
    // The last line of the method's body deleted, and an added file: the snapshot alone holds
    // the added file's symbol.
    let mut deleted = RUST.lines.to_vec();
    deleted.remove(method.first + 2);
    b.write(RUST.path, &edited(&deleted, &[], "", "")).unwrap();
    b.write("src/shelf.rs", "pub fn stack() {\n}\n").unwrap();
    let mut space = Space::new(&[(&a, &a_client), (&b, &b_client)]);
    space.publish();

    let own = |machine: &str| names(&space.edits_of(machine).symbols());
    let file = |path: &str, symbol: &str| (path.to_owned(), BTreeSet::from([symbol.to_owned()]));
    let store = file(RUST.path, method.symbol);
    assert_eq!(
        own("alice-laptop"),
        BTreeMap::from([
            store.clone(),
            file("src/warehouse.rs", "struct_item Warehouse")
        ])
    );
    assert_eq!(
        own("bob-desktop"),
        BTreeMap::from([store, file("src/shelf.rs", "function_item stack")])
    );
    assert_eq!(
        space.seen("alice-laptop"),
        file_and(&RUST, "bob-desktop", &[method.symbol])
    );
    assert!(space.check("alice-laptop")["bob-desktop"].clean);
}

/// The scenarios of each language, in a module named after it.
macro_rules! languages {
    ($($language:ident: $fixture:ident;)*) => {$(
        mod $language {
            use super::*;

            #[test]
            fn edits_to_different_functions_of_one_file_overlap_at_file_level_only() {
                different_functions(&$fixture);
            }

            #[test]
            fn edits_to_distant_lines_of_one_function_overlap_at_symbol_level_and_merge_cleanly() {
                distant_lines_of_one_function(&$fixture);
            }

            #[test]
            fn edits_to_two_methods_of_one_type_do_not_overlap_at_symbol_level() {
                two_methods_of_one_type(&$fixture);
            }

            #[test]
            fn a_symbol_moved_within_its_file_keeps_its_fingerprint() {
                moved_symbol(&$fixture);
            }

            #[test]
            fn a_file_in_another_language_gets_no_symbol_fingerprints() {
                file_in_another_language(&$fixture);
            }

            #[test]
            fn server_receives_no_symbol_name() {
                no_symbol_name_reaches_the_server(&$fixture);
            }
        }
    )*};
}

languages! {
    rust: RUST;
    typescript: TYPESCRIPT;
    python: PYTHON;
}
