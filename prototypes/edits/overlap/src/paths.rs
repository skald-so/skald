//! The one form of a path that fingerprints take, the paths an agent's tools give mapped to a
//! repository's paths, and the paths of a repository that share that form.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::noise::table;
use crate::replay::Measured;
use crate::{Result, git};

/// The form of `path`, relative to the repository root with `/` separators, that fingerprints
/// take: each character lowercased, as `char::to_lowercase` does, in Unicode Normalization Form C.
/// Spellings that a case-insensitive or a normalization-insensitive file system takes for one
/// file get one form.
pub fn normalize(path: &str) -> String {
    path.nfd().flat_map(char::to_lowercase).nfc().collect()
}

/// The path of the file at `given`, a path an agent's tool gives, relative to the root of the
/// worktree of the repository at `dir` that holds it, with `/` separators; none when no worktree
/// of that repository holds it. A relative `given` is taken from `dir`. Its components that exist
/// are resolved as the file system resolves them: symbolic links, `..`, and on Windows drive
/// letters and both separators; on a case-insensitive file system, their case as the directory
/// holds it. Those after the last that exists, such as a deleted file's name, stay as `given`
/// spells them, `..` dropping the one before it.
pub fn relative(dir: &Path, given: &Path) -> Result<Option<String>> {
    let given = dir.join(given);
    let mut existing = PathBuf::new();
    let mut missing: Vec<OsString> = Vec::new();
    for component in given.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => existing.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                if missing.pop().is_none() {
                    existing.pop();
                }
            }
            Component::Normal(name) if missing.is_empty() => {
                match fs::canonicalize(existing.join(name)) {
                    Ok(resolved) => existing = resolved,
                    Err(error) if error.kind() == ErrorKind::NotFound => {
                        missing.push(name.to_owned())
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Component::Normal(name) => missing.push(name.to_owned()),
        }
    }
    let directory = if existing.is_dir() {
        existing.as_path()
    } else {
        existing.parent().ok_or("a file without a directory")?
    };
    let Some(root) = worktree(directory, dir)? else {
        return Ok(None);
    };
    let Ok(inside) = existing.strip_prefix(&root) else {
        return Ok(None);
    };
    let mut components = Vec::new();
    for name in inside.iter().chain(missing.iter().map(OsString::as_os_str)) {
        components.push(name.to_str().ok_or("a path that is not UTF-8")?);
    }
    Ok((!components.is_empty()).then(|| components.join("/")))
}

/// The root of the worktree that holds `directory`, as the file system resolves it, when it is a
/// worktree of the repository at `dir`.
fn worktree(directory: &Path, dir: &Path) -> Result<Option<PathBuf>> {
    let args = [
        "rev-parse",
        "--path-format=absolute",
        "--show-toplevel",
        "--git-common-dir",
    ];
    let output = git::command(directory, &args).output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let output = String::from_utf8(output.stdout)?;
    let [root, common] = output.lines().collect::<Vec<_>>()[..] else {
        return Err(format!("git rev-parse gave {output:?}").into());
    };
    let ours = git::line(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if fs::canonicalize(common)? != fs::canonicalize(ours)? {
        return Ok(None);
    }
    Ok(Some(fs::canonicalize(root)?))
}

/// How the paths of a group that share their [`normalize`] form differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Difference {
    Case,
    Normalization,
    CaseAndNormalization,
}

impl Difference {
    pub fn name(self) -> &'static str {
        match self {
            Self::Case => "case",
            Self::Normalization => "Unicode normalization",
            Self::CaseAndNormalization => "case and Unicode normalization",
        }
    }
}

/// The groups of two or more of `paths` that share their [`normalize`] form, sorted, each with
/// how its paths differ: in case, when their NFC forms differ; also in Unicode normalization, when
/// their lowercase forms differ too; otherwise in Unicode normalization only.
pub fn collisions<'p>(
    paths: impl IntoIterator<Item = &'p str>,
) -> Vec<(Difference, BTreeSet<&'p str>)> {
    let mut groups = BTreeMap::<String, BTreeSet<&str>>::new();
    for path in paths {
        groups.entry(normalize(path)).or_default().insert(path);
    }
    groups
        .into_values()
        .filter(|group| group.len() > 1)
        .map(|group| {
            let distinct = |form: fn(&str) -> String| {
                let forms: BTreeSet<String> = group.iter().map(|path| form(path)).collect();
                forms.len() > 1
            };
            let case = distinct(|path| path.nfc().collect());
            let normalization = distinct(str::to_lowercase);
            let difference = match (case, normalization) {
                (true, false) => Difference::Case,
                (true, true) => Difference::CaseAndNormalization,
                (false, _) => Difference::Normalization,
            };
            (difference, group)
        })
        .collect()
}

/// The table of the paths of `measured` that share a fingerprint, in Markdown, as
/// [`crate::replay::report`] gives it: those of the tree at each repository's commit, then those
/// the replayed sides change.
pub fn report(measured: &[Measured]) -> String {
    let mut out = String::new();
    let title = "Paths that share a fingerprint once case is folded and Unicode normalized";
    let header = ["Repository", "Paths of", "Differ in", "Paths"].map(str::to_owned);
    table(&mut out, title, &header, |row| {
        for Measured {
            repository,
            replays,
            tree,
            ..
        } in measured
        {
            let changed: BTreeSet<&str> = replays
                .iter()
                .flat_map(|replay| &replay.sides)
                .flat_map(|side| side.edits.paths())
                .collect();
            let tree = tree.iter().map(String::as_str);
            let sources = [
                ("the tree at the commit", collisions(tree)),
                ("the replayed sides", collisions(changed)),
            ];
            for (source, groups) in sources {
                for (difference, group) in groups {
                    let paths: Vec<String> = group.iter().map(|path| format!("`{path}`")).collect();
                    let name = repository.name.to_owned();
                    row(vec![
                        name,
                        source.into(),
                        difference.name().into(),
                        paths.join(", "),
                    ]);
                }
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Café/Résumé.md` with its accented letters precomposed, NFC, and decomposed, NFD.
    const NFC: &str = "Caf\u{e9}/R\u{e9}sum\u{e9}.md";
    const NFD: &str = "Cafe\u{301}/Re\u{301}sume\u{301}.md";

    #[test]
    fn every_spelling_of_a_path_gets_one_form() {
        let form = "caf\u{e9}/r\u{e9}sum\u{e9}.md";
        for spelling in [
            NFC,
            NFD,
            "CAF\u{c9}/R\u{c9}SUM\u{c9}.MD",
            "CAFE\u{301}/RE\u{301}SUME\u{301}.MD",
            "caf\u{e9}/r\u{e9}sum\u{e9}.md",
            "cafe\u{301}/Re\u{301}sum\u{e9}.md",
        ] {
            assert_eq!(normalize(spelling), form, "{spelling:?}");
        }
        // Lowercasing a decomposed letter keeps it composable: `İ` is `i` and a dot above.
        assert_eq!(normalize("\u{130}.rs"), "i\u{307}.rs");
        assert_eq!(normalize("I\u{307}.rs"), "i\u{307}.rs");
        for other in [
            "Cafe/Resume.md",
            "Caf\u{e9}\\R\u{e9}sum\u{e9}.md",
            "Caf\u{e8}/R\u{e9}sum\u{e9}.md",
        ] {
            assert_ne!(normalize(other), form, "{other:?}");
        }
    }

    #[test]
    fn collisions_are_the_groups_of_one_form_with_how_they_differ() {
        let paths = [
            "README.md",
            "Readme.md",
            "readme.md",
            NFC,
            NFD,
            "src/Caf\u{e9}.rs",
            "src/cafe\u{301}.rs",
            "src/lib.rs",
            "src/main.rs",
        ];
        assert_eq!(
            collisions(paths),
            [
                (Difference::Normalization, BTreeSet::from([NFC, NFD])),
                (
                    Difference::Case,
                    BTreeSet::from(["README.md", "Readme.md", "readme.md"])
                ),
                (
                    Difference::CaseAndNormalization,
                    BTreeSet::from(["src/Caf\u{e9}.rs", "src/cafe\u{301}.rs"])
                ),
            ]
        );
        assert_eq!(collisions(["a", "b", "a"]), []);
    }
}
