//! Every `skald send` the README and the docs show names its addressee.

use std::fs;
use std::path::{Path, PathBuf};

use crate::fixtures::{ALL, SEND, TO};

/// The repository's root, two levels above this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The Markdown files a reader of the repository reads: those in its root but the CHANGELOG, which
/// records past releases, and those under `docs/`.
fn markdown_files() -> Vec<PathBuf> {
    fn under(dir: &Path, found: &mut Vec<PathBuf>) {
        let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("{}: {err}", dir.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                under(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                found.push(path);
            }
        }
    }
    let root = root();
    let mut found: Vec<PathBuf> = (fs::read_dir(&root).expect("the repository's root"))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "md"))
        .filter(|path| path.file_name().is_some_and(|name| name != "CHANGELOG.md"))
        .collect();
    under(&root.join("docs"), &mut found);
    found.sort();
    found
}

/// The code in a Markdown file's `line`: the whole line in a fenced block, otherwise each inline
/// code span.
fn code_of(line: &str, fenced: bool) -> Vec<&str> {
    if fenced {
        vec![line]
    } else {
        line.split('`').skip(1).step_by(2).collect()
    }
}

/// The arguments of each `skald [--agent <name>] send` in `code`, up to a shell comment or the
/// next command.
fn send_args(code: &str) -> Vec<Vec<&str>> {
    let words: Vec<&str> = code.split_whitespace().collect();
    let mut found = Vec::new();
    for (at, word) in words.iter().enumerate() {
        if *word != "skald" {
            continue;
        }
        let mut next = at + 1;
        if words.get(next) == Some(&"--agent") {
            next += 2;
        }
        if words.get(next) != Some(&SEND) {
            continue;
        }
        let args: Vec<&str> = (words[next + 1..].iter())
            .take_while(|word| !["#", "|", "&&", "||", ";"].contains(word))
            .copied()
            .collect();
        found.push(args);
    }
    found
}

/// Each `skald send` with arguments in the Markdown files of the README and the docs, in a code
/// block or an inline code span, has `--to` or `--all`, or asks for the help; a bare `skald send`
/// in prose names the command only.
#[test]
fn every_skald_send_in_the_readme_and_the_docs_names_its_addressee() {
    let mut unnamed = Vec::new();
    let files = markdown_files();
    assert!(
        files.iter().any(|file| file.ends_with("README.md")),
        "no README among {files:?}"
    );
    for file in files {
        let text =
            fs::read_to_string(&file).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
        let mut fenced = false;
        for (number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
                continue;
            }
            for code in code_of(line, fenced) {
                for args in send_args(code) {
                    let names = args.iter().any(|arg| {
                        *arg == TO || arg.starts_with("--to=") || *arg == ALL || *arg == "--help"
                    });
                    if !args.is_empty() && !names {
                        unnamed.push(format!("{}:{}: {code}", file.display(), number + 1));
                    }
                }
            }
        }
    }
    assert!(
        unnamed.is_empty(),
        "skald send without --to or --all:\n{}",
        unnamed.join("\n")
    );
}
