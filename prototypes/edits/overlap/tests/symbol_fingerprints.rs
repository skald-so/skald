//! The symbol fingerprint scenarios of `docs/dev/edit-prototype.md`, "Scenarios", in each
//! supported language: machines also publish fingerprints of the functions, methods and types
//! their changed lines are in, and those match when both change one, whether or not
//! `git merge-tree` conflicts.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{
    ALICE_AND_BOB, Clones, PAIRS, Repo, Seen, Space, assert_holds_none, names, paths, symbols,
};
use edit_overlap::harness::Machine;
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

/// Two machines, Alice and Bob, cloned from a repository with the file of `fixture` and `extra`
/// files: `edit` edits their working trees, given the repository, then both publish, and `then`
/// gets their space.
fn two_machines(
    fixture: &Fixture,
    extra: &[(&str, &str)],
    edit: impl FnOnce(&Repo, &Machine, &Machine),
    then: impl FnOnce(&mut Space),
) {
    let clones = Clones::new(|tmp| fixture.repo(tmp, extra), ALICE_AND_BOB);
    let [a, b] = &clones.machines;
    edit(&clones.repo, a, b);
    then(&mut clones.published());
}

/// Alice comments the lines `alice` of the file of `fixture` and commits; Bob comments the lines
/// `bob` and does not commit; then both publish, and `then` gets their space.
fn edit_lines(fixture: &Fixture, alice: &[usize], bob: &[usize], then: impl FnOnce(&mut Space)) {
    let edit = |_: &Repo, a: &Machine, b: &Machine| {
        a.write(fixture.path, &fixture.edited(alice, "alice"))
            .unwrap();
        a.commit("alice's edit").unwrap();
        b.write(fixture.path, &fixture.edited(bob, "bob")).unwrap();
    };
    two_machines(fixture, &[], edit, then);
}

/// Asserts that each of the two machines learns what `seen` gives for the other one.
fn assert_each_sees(space: &Space, seen: impl Fn(&str) -> Seen) {
    for (machine, other) in PAIRS {
        assert_eq!(space.seen(machine), seen(other), "{machine}");
    }
}

/// The function and the first method share a name; edits to them overlap at file level only.
fn different_functions(fixture: &Fixture) {
    let function = fixture.function.first + 2;
    let method = fixture.methods[0].first + 2;
    edit_lines(fixture, &[function], &[method], |space| {
        assert_each_sees(space, |other| file_and(fixture, other, &[]));
    });
}

/// The first and the last line of the first method's body: they overlap at symbol level and
/// merge cleanly.
fn distant_lines_of_one_function(fixture: &Fixture) {
    let method = &fixture.methods[0];
    edit_lines(fixture, &[method.first + 1], &[method.first + 3], |space| {
        assert_each_sees(space, |other| file_and(fixture, other, &[method.symbol]));
        for (machine, other) in PAIRS {
            assert!(space.check(machine)[other].clean, "{machine}");
        }
    });
}

/// The middle lines of the two methods of one type: they overlap at file level only.
fn two_methods_of_one_type(fixture: &Fixture) {
    let [first, second] = &fixture.methods;
    edit_lines(fixture, &[first.first + 2], &[second.first + 2], |space| {
        assert_each_sees(space, |other| file_and(fixture, other, &[]));
    });
}

/// The function moved to the end of the file upstream; one machine pulls it and edits the first
/// line of its body there, the other does not and edits the last one in its old place.
fn moved_symbol(fixture: &Fixture) {
    let edit = |repo: &Repo, a: &Machine, b: &Machine| {
        let (maintainer, _) = repo.machine("maintainer");
        let (moved, first) = fixture.moved();
        maintainer
            .write(fixture.path, &edited(&moved, &[], "", ""))
            .unwrap();
        maintainer.commit("move the function").unwrap();
        maintainer.push("main").unwrap();
        a.pull().unwrap();
        let text = edited(&moved, &[first + 1], fixture.comment, "alice");
        a.write(fixture.path, &text).unwrap();
        let last = fixture.function.first + 3;
        b.write(fixture.path, &fixture.edited(&[last], "bob"))
            .unwrap();
    };
    two_machines(fixture, &[], edit, |space| {
        let function = &[fixture.function.symbol][..];
        assert_each_sees(space, |other| file_and(fixture, other, function));
    });
}

/// Distant lines of one function edited in the fixture and in a Go file: only the fixture's
/// overlap at symbol level.
fn file_in_another_language(fixture: &Fixture) {
    let (go, go_text) = GO;
    let method = &fixture.methods[0];
    let edit = |_: &Repo, a: &Machine, b: &Machine| {
        let go_lines: Vec<&str> = go_text.lines().collect();
        for (machine, who, offset, go_line) in [(a, "alice", 1, 4), (b, "bob", 3, 6)] {
            let text = fixture.edited(&[method.first + offset], who);
            machine.write(fixture.path, &text).unwrap();
            machine
                .write(go, &edited(&go_lines, &[go_line], "//", who))
                .unwrap();
        }
    };
    two_machines(fixture, &[GO], edit, |space| {
        assert_each_sees(space, |other| Seen {
            paths: paths(&[(other, &[fixture.path, go])]),
            hunks: BTreeMap::new(),
            symbols: symbols(&[(other, fixture.path, &[method.symbol])]),
        });
        for (machine, _) in PAIRS {
            let files: BTreeSet<String> = space.edits_of(machine).symbols().into_keys().collect();
            assert_eq!(files, BTreeSet::from([fixture.path.into()]), "{machine}");
            // The two paths and the method: nothing of the Go function.
            let overlaps = space.server.overlaps(machine);
            assert_eq!(overlaps[0].fingerprints.len(), 3, "{machine}");
        }
    });
}

/// Every symbol of the fixture changed on two machines: nothing the server receives holds the
/// kind or the name of one.
fn no_symbol_name_reaches_the_server(fixture: &Fixture) {
    let [first, second] = &fixture.methods;
    let alice = [fixture.function.first + 1, first.first + 1];
    let bob = [first.first + 3, second.first + 1];
    edit_lines(fixture, &alice, &bob, |space| {
        assert!(!space.seen("alice-laptop").symbols.is_empty());
        let mut needles = BTreeSet::new();
        for edits in &space.edits {
            for symbol in edits.symbols().values().flatten() {
                let parts = symbol.0.iter().cloned();
                needles.extend(parts.flat_map(|(kind, name)| [kind, name]));
            }
        }
        assert!(needles.contains("Store") && needles.contains("release"));
        let needles = needles.iter().map(String::as_str);
        assert_holds_none(&space.received_bytes(), needles, "what the server received");
    });
}

#[test]
fn insertions_deletions_and_added_and_deleted_files_get_symbol_fingerprints() {
    let method = &RUST.methods[0];
    let edit = |_: &Repo, a: &Machine, b: &Machine| {
        // A line inserted after the first line of the method's body, and a deleted file: the
        // merge base alone holds the deleted file's symbol.
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
    };
    two_machines(&RUST, &[], edit, |space| {
        let own = |machine: &str| names(&space.edits_of(machine).symbols());
        let file = |path: &str, symbol: &str| (path.to_owned(), BTreeSet::from([symbol.into()]));
        let store = file(RUST.path, method.symbol);
        let warehouse = file("src/warehouse.rs", "struct_item Warehouse");
        let shelf = file("src/shelf.rs", "function_item stack");
        assert_eq!(
            own("alice-laptop"),
            BTreeMap::from([store.clone(), warehouse])
        );
        assert_eq!(own("bob-desktop"), BTreeMap::from([store, shelf]));
        assert_eq!(
            space.seen("alice-laptop"),
            file_and(&RUST, "bob-desktop", &[method.symbol])
        );
        assert!(space.check("alice-laptop")["bob-desktop"].clean);
    });
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
