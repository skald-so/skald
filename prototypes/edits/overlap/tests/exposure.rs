//! What a holder of the space key recovers of a machine's fingerprints, as
//! `docs/dev/edit-prototype.md`, "What the server learns", measures it: the fixed dictionary
//! recovers a fixture path and symbol name with the key, and nothing without it or with another
//! key; a clone recovers the paths and symbols it holds.

mod common;

use std::collections::BTreeSet;

use common::{Repo, key};
use edit_overlap::dictionary::{self, Dictionary, Recovered};
use edit_overlap::fingerprint::Keying;
use edit_overlap::harness::Machine;
use edit_overlap::{Fingerprint, Scope, Server, SpaceKey, git};
use tempfile::TempDir;

/// `src/main.rs` of the fixture: a function and a method of names the fixed dictionary holds.
const MAIN: &str = "\
fn main() {
    let items = 1;
    println!(\"{items}\");
}

struct Config;

impl Config {
    fn new() -> Self {
        Config
    }
}
";

/// A clone of the test repository with [`MAIN`], whose machine edits a line of `main` and of
/// `new`, the guide, and `extra` files, each a path and its contents; its scope and the
/// fingerprints the server keeps of it.
fn published(tmp: &TempDir, extra: &[(&str, &str)]) -> (Machine, Scope, BTreeSet<Fingerprint>) {
    let repo = Repo::create_with(tmp, &[("src/main.rs", MAIN)]);
    let (machine, client) = repo.machine("alice-laptop");
    machine.write("src/main.rs", &edited_main()).unwrap();
    machine
        .write("docs/guide.md", "# Guide\n\nRead it.\n")
        .unwrap();
    for (path, contents) in extra {
        machine.write(path, contents).unwrap();
    }
    let mut server = Server::default();
    server.publish(client.edits(machine.dir()).unwrap().publication);
    let kept = server.kept()["alice-laptop"].clone();
    let scope = git::scope(machine.dir()).unwrap();
    (machine, scope, kept)
}

/// [`MAIN`] with a line of `main` and one of `new` edited.
fn edited_main() -> String {
    MAIN.replace("let items = 1;", "let items = 2;")
        .replace("        Config\n", "        Config {}\n")
}

/// The paths and the symbols of `recovered`, `<path>: <symbol>`, sorted.
fn texts(recovered: &Recovered) -> (BTreeSet<&str>, BTreeSet<String>) {
    let paths = recovered.paths.values().map(String::as_str).collect();
    let symbols = (recovered.symbols.values())
        .map(|(path, symbol)| format!("{path}: {symbol}"))
        .collect();
    (paths, symbols)
}

#[test]
fn the_dictionary_recovers_a_fixture_path_and_symbol_name_with_the_key_only() {
    let tmp = TempDir::new().unwrap();
    let (_machine, scope, published) = published(&tmp, &[]);
    let key = key();
    let fixed = Dictionary::fixed();
    let recovered = dictionary::recover(Keying::Key(&key), &scope, &fixed, &published);
    // Not the guide, a path it lacks.
    assert_eq!(
        texts(&recovered),
        (
            ["src/main.rs"].into(),
            [
                "src/main.rs: function_item main".to_owned(),
                "src/main.rs: impl_item Config > function_item new".to_owned(),
            ]
            .into()
        )
    );
    let other = SpaceKey::from_bytes([1; 32]);
    for keying in [Keying::WithoutKey, Keying::Key(&other)] {
        let recovered = dictionary::recover(keying, &scope, &fixed, &published);
        assert_eq!(recovered, Recovered::default());
    }
}

#[test]
fn a_clone_recovers_the_paths_and_symbols_it_holds() {
    let tmp = TempDir::new().unwrap();
    // A new file, and a function added to `src/main.rs`, after the clone's commit.
    let report = ("src/report.rs", "fn summary() -> u32 {\n    1\n}\n");
    let main = edited_main() + "\nfn helper() -> u32 {\n    3\n}\n";
    let (machine, scope, published) = published(&tmp, &[report, ("src/main.rs", &main)]);
    let key = key();
    let dir = machine.dir();
    let recovered =
        dictionary::recover_with_clone(Keying::Key(&key), &scope, dir, "HEAD", &published).unwrap();
    // Every path and symbol of the clone that the machine publishes: not `src/report.rs` and its
    // `summary`, nor `helper`, which the clone lacks.
    assert_eq!(
        texts(&recovered),
        (
            ["docs/guide.md", "src/main.rs"].into(),
            [
                "src/main.rs: function_item main".to_owned(),
                "src/main.rs: impl_item Config > function_item new".to_owned(),
            ]
            .into()
        )
    );
    let other = SpaceKey::from_bytes([1; 32]);
    for keying in [Keying::WithoutKey, Keying::Key(&other)] {
        let recovered =
            dictionary::recover_with_clone(keying, &scope, dir, "HEAD", &published).unwrap();
        assert_eq!(recovered, Recovered::default());
    }
}
