//! What a holder of the space key recovers from the fingerprints the server keeps: with a fixed
//! dictionary of common paths and symbol names, kept in `dictionary/`, or with a clone of the
//! repository. A fingerprint takes the repository's scope too, so either needs it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::fingerprint::{self, Keying};
use crate::symbols::{self, Language, Symbol};
use crate::{Fingerprint, Result, Scope, git, replay};

/// Paths to try against path fingerprints, and names to try against symbol fingerprints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictionary {
    pub paths: Vec<String>,
    pub names: Vec<String>,
}

impl Dictionary {
    /// The fixed dictionary of `dictionary/paths.txt` and `dictionary/names.txt`.
    pub fn fixed() -> Self {
        Self::parse(
            include_str!("../dictionary/paths.txt"),
            include_str!("../dictionary/names.txt"),
        )
    }

    /// The dictionary of `paths` and `names`, one per line; empty lines and lines that start with
    /// `#` are left out.
    pub fn parse(paths: &str, names: &str) -> Self {
        let entries = |text: &str| {
            text.lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_owned)
                .collect()
        };
        Self {
            paths: entries(paths),
            names: entries(names),
        }
    }
}

/// What a guess recovers of the fingerprints a server keeps: the path behind each path
/// fingerprint, and the path and the symbol behind each symbol fingerprint.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recovered {
    pub paths: BTreeMap<Fingerprint, String>,
    pub symbols: BTreeMap<Fingerprint, (String, Symbol)>,
}

/// The kinds of symbol in `language` that the dictionary takes to enclose others: types, traits,
/// interfaces, modules and `impl` blocks.
fn containers(language: Language) -> &'static [&'static str] {
    match language {
        Language::Rust => &["impl_item", "trait_item", "mod_item"],
        Language::TypeScript => &[
            "class_declaration",
            "abstract_class_declaration",
            "interface_declaration",
            "internal_module",
        ],
        Language::Python => &["class_definition"],
    }
}

/// What a party with `keying` recovers of `published` in `scope` with `dictionary`: each of its
/// paths whose fingerprint is published; then, on each recovered path in a language with symbols,
/// each symbol of one of its names and of a kind of the language, alone or inside a
/// [container](containers) of one of its names, whose fingerprint is published.
pub fn recover(
    keying: Keying,
    scope: &Scope,
    dictionary: &Dictionary,
    published: &BTreeSet<Fingerprint>,
) -> Recovered {
    let mut recovered = Recovered::default();
    for path in &dictionary.paths {
        let fingerprint = fingerprint::path_with(keying, scope, path);
        if published.contains(&fingerprint) {
            recovered.paths.insert(fingerprint, path.clone());
        }
    }
    let paths: Vec<String> = recovered.paths.values().cloned().collect();
    for path in paths {
        let Some(language) = Language::of(&path) else {
            continue;
        };
        let names = &dictionary.names;
        let symbol = |kind: &str, name: &String| (kind.to_owned(), name.clone());
        let symbols = fingerprint::Symbols::new(keying, scope, &path);
        let mut try_symbol = |symbol: Symbol| {
            let fingerprint = symbols.fingerprint(&symbol);
            if published.contains(&fingerprint) {
                recovered
                    .symbols
                    .insert(fingerprint, (path.clone(), symbol));
            }
        };
        for kind in language.kinds() {
            for name in names {
                try_symbol(Symbol(vec![symbol(kind, name)]));
                for outer in containers(language) {
                    for outer_name in names {
                        let outer = symbol(outer, outer_name);
                        try_symbol(Symbol(vec![outer, symbol(kind, name)]));
                    }
                }
            }
        }
    }
    recovered
}

/// What a party with `keying` recovers of `published` in `scope` with a clone at `commit` of the
/// repository at `dir`: each path of its tree whose fingerprint is published, and each symbol of
/// its files in a language with symbols.
pub fn recover_with_clone(
    keying: Keying,
    scope: &Scope,
    dir: &Path,
    commit: &str,
    published: &BTreeSet<Fingerprint>,
) -> Result<Recovered> {
    let mut recovered = Recovered::default();
    for path in replay::tree(dir, commit)? {
        let fingerprint = fingerprint::path_with(keying, scope, &path);
        if !published.contains(&fingerprint) {
            continue;
        }
        if let Some(language) = Language::of(&path) {
            let source = git::blob(dir, commit, &path)?;
            let symbols = fingerprint::Symbols::new(keying, scope, &path);
            for symbol in symbols::all(language, &source)? {
                let fingerprint = symbols.fingerprint(&symbol);
                if published.contains(&fingerprint) {
                    recovered
                        .symbols
                        .insert(fingerprint, (path.clone(), symbol));
                }
            }
        }
        recovered.paths.insert(fingerprint, path);
    }
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpaceKey;

    fn key() -> SpaceKey {
        SpaceKey::from_bytes([7; 32])
    }

    fn scope() -> Scope {
        Scope::new("0123abcd")
    }

    fn symbol(chain: &[(&str, &str)]) -> Symbol {
        Symbol(chain.iter().map(|&(k, n)| (k.into(), n.into())).collect())
    }

    /// The fingerprints a machine publishes for `src/lib.rs`, `src/app.ts`, `docs/notes.md` and
    /// `README.md`, and for symbols of the first two and of `src/other.rs`.
    fn published(keying: Keying) -> BTreeSet<Fingerprint> {
        let paths = ["src/lib.rs", "src/app.ts", "docs/notes.md", "README.md"];
        let symbols = [
            ("src/lib.rs", symbol(&[("function_item", "main")])),
            (
                "src/lib.rs",
                symbol(&[("impl_item", "Config"), ("function_item", "new")]),
            ),
            (
                "src/lib.rs",
                symbol(&[("impl_item", "Config"), ("function_item", "reserve")]),
            ),
            (
                "src/app.ts",
                symbol(&[
                    ("class_declaration", "App"),
                    ("method_definition", "render"),
                ]),
            ),
            ("src/other.rs", symbol(&[("function_item", "main")])),
        ];
        let paths = paths
            .iter()
            .map(|path| fingerprint::path_with(keying, &scope(), path));
        let symbols = symbols
            .iter()
            .map(|(path, symbol)| fingerprint::symbol_with(keying, &scope(), path, symbol));
        paths.chain(symbols).collect()
    }

    fn dictionary() -> Dictionary {
        Dictionary::parse(
            "# paths\nsrc/lib.rs\n\nsrc/app.ts\nREADME.md\nsrc/other.ts\n",
            "main\nnew\nConfig\nApp\nrender\n",
        )
    }

    fn texts(recovered: &Recovered) -> (Vec<&str>, Vec<String>) {
        let mut paths: Vec<&str> = recovered.paths.values().map(String::as_str).collect();
        paths.sort();
        let mut symbols: Vec<String> = (recovered.symbols.values())
            .map(|(path, symbol)| format!("{path}: {symbol}"))
            .collect();
        symbols.sort();
        (paths, symbols)
    }

    #[test]
    fn parse_leaves_out_empty_lines_and_comments() {
        let dictionary = dictionary();
        assert_eq!(
            dictionary.paths,
            ["src/lib.rs", "src/app.ts", "README.md", "src/other.ts"]
        );
        assert_eq!(dictionary.names, ["main", "new", "Config", "App", "render"]);
    }

    #[test]
    fn the_fixed_dictionary_holds_common_paths_and_names_only() {
        let fixed = Dictionary::fixed();
        for path in [
            "README.md",
            ".gitignore",
            "Cargo.toml",
            "package.json",
            "src/main.rs",
        ] {
            assert!(fixed.paths.iter().any(|p| p == path), "{path}");
        }
        for name in ["main", "new", "Config", "__init__", "constructor"] {
            assert!(fixed.names.iter().any(|n| n == name), "{name}");
        }
        let entries = fixed.paths.iter().chain(&fixed.names);
        assert!(
            entries
                .clone()
                .all(|entry| !entry.is_empty() && !entry.starts_with('#'))
        );
    }

    #[test]
    fn with_the_key_the_dictionary_recovers_its_paths_and_their_symbols() {
        let key = key();
        let keying = Keying::Key(&key);
        let recovered = recover(keying, &scope(), &dictionary(), &published(keying));
        // Not `docs/notes.md` nor `src/other.rs`, which it lacks, nor `reserve`.
        assert_eq!(
            texts(&recovered),
            (
                vec!["README.md", "src/app.ts", "src/lib.rs"],
                vec![
                    "src/app.ts: class_declaration App > method_definition render".to_owned(),
                    "src/lib.rs: function_item main".to_owned(),
                    "src/lib.rs: impl_item Config > function_item new".to_owned(),
                ]
            )
        );
        for (fingerprint, path) in &recovered.paths {
            assert_eq!(*fingerprint, fingerprint::path(&key, &scope(), path));
        }
        for (fingerprint, (path, symbol)) in &recovered.symbols {
            assert_eq!(
                *fingerprint,
                fingerprint::symbol(&key, &scope(), path, symbol)
            );
        }
    }

    #[test]
    fn without_the_key_or_with_another_or_another_scope_the_dictionary_recovers_nothing() {
        let key = key();
        let keyed = published(Keying::Key(&key));
        let other = SpaceKey::from_bytes([8; 32]);
        for keying in [Keying::WithoutKey, Keying::Key(&other)] {
            let recovered = recover(keying, &scope(), &dictionary(), &keyed);
            assert_eq!(recovered, Recovered::default());
        }
        let elsewhere = Scope::new("4567cdef");
        let recovered = recover(Keying::Key(&key), &elsewhere, &dictionary(), &keyed);
        assert_eq!(recovered, Recovered::default());
        // Without the key, it recovers what is published without it.
        let unkeyed = published(Keying::WithoutKey);
        let recovered = recover(Keying::WithoutKey, &scope(), &dictionary(), &unkeyed);
        assert_eq!(
            texts(&recovered).0,
            ["README.md", "src/app.ts", "src/lib.rs"]
        );
    }
}
