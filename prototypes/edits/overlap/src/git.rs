//! A machine's edit set and repository scope, read through the `git` CLI.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use crate::{Result, Scope};

/// The upstream's default branch, as `git clone` records it.
pub const UPSTREAM_HEAD: &str = "refs/remotes/origin/HEAD";

/// A `git` command in `dir` whose environment holds only `PATH` and the settings below: no
/// credentials, config, repository or home directory of the parent's. It reads no global or
/// system config, attributes or ignore files, never prompts, and commits as a fixed identity.
pub fn command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(program());
    command.args(args).current_dir(dir).env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "skald")
        .env("GIT_AUTHOR_EMAIL", "skald@example.com")
        .env("GIT_COMMITTER_NAME", "skald")
        .env("GIT_COMMITTER_EMAIL", "skald@example.com");
    command
}

/// The `git` that `PATH` gives, the first file named `git` in its absolute directories, or `git`
/// when there is none. Rust's standard library starts a program given by its path with `posix_spawn`, but
/// forks to look a name up in a `PATH` the command sets; a fork of a process with many threads and
/// a large heap holds up each of its threads that allocates.
fn program() -> &'static Path {
    static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
    PROGRAM.get_or_init(|| {
        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("git"))
            .find(|git| git.is_file())
            .unwrap_or_else(|| "git".into())
    })
}

/// Runs `git` in `dir`, as [`command`] sets it up, and returns its standard output.
pub fn run(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    checked(command(dir, args).output()?, args, dir)
}

/// Runs `git` in `dir` with `input` on its standard input and returns its standard output.
pub fn run_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    let mut child = command(dir, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("git without a standard input")?
        .write_all(input)?;
    checked(child.wait_with_output()?, args, dir)
}

/// The standard output of `git` with `args` in `dir` if it succeeded; otherwise an error with its
/// standard error.
pub fn checked(output: Output, args: &[&str], dir: &Path) -> Result<Vec<u8>> {
    if !output.status.success() {
        return Err(format!(
            "git {} in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(output.stdout)
}

/// The output of `git` in `dir` as one line of text, without its line ending.
pub fn line(dir: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(run(dir, args)?)?.trim_end().to_owned())
}

/// The merge base of `HEAD` and the upstream's default branch at `dir`.
pub fn merge_base(dir: &Path) -> Result<String> {
    line(dir, &["merge-base", "HEAD", UPSTREAM_HEAD])
}

/// The contents of `path`, relative to the root, in the commit or tree `tree` of the repository at
/// `dir`, as git stores them: without the conversions of attributes and config.
pub fn blob(dir: &Path, tree: &str, path: &str) -> Result<Vec<u8>> {
    run(dir, &["cat-file", "blob", &format!("{tree}:{path}")])
}

/// Whether the repository at `dir` has the commit `oid`.
pub fn has_commit(dir: &Path, oid: &str) -> Result<bool> {
    exists(dir, &format!("{oid}^{{commit}}"))
}

/// Whether the repository at `dir` has the object `object` names, such as `<commit>:<path>`.
pub fn exists(dir: &Path, object: &str) -> Result<bool> {
    let status = command(dir, &["cat-file", "-e", object])
        .stderr(Stdio::null())
        .status()?;
    Ok(status.success())
}

/// A commit of the working tree at `dir`, its root, on top of `HEAD`: its tracked files as they
/// are now and its untracked ones, not its ignored ones. The index and the working tree stay as
/// they are; the commit is on no branch.
pub fn snapshot(dir: &Path) -> Result<String> {
    let index = PathBuf::from(line(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?);
    // A copy of the index: `git add` then rehashes only the files whose stat data changed.
    let copy = index.with_file_name(format!("skald-snapshot-index-{}", std::process::id()));
    let result = (|| {
        // The copy keeps the index's modification time: git rehashes a file whose stat data
        // matches its entry only when the entry is as new as the index file, and a later copy
        // would hide an edit of the same size made within the timestamp's resolution. The time
        // is read before the copy, so an index rewritten in between gives the copy an older time
        // and git rehashes more, never less.
        let modified = fs::metadata(&index)?.modified()?;
        fs::copy(&index, &copy)?;
        fs::File::options()
            .write(true)
            .open(&copy)?
            .set_modified(modified)?;
        let with_copy = |args: &[&str]| {
            let output = command(dir, args).env("GIT_INDEX_FILE", &copy).output()?;
            checked(output, args, dir)
        };
        with_copy(&["add", "--all"])?;
        let tree = String::from_utf8(with_copy(&["write-tree"])?)?;
        line(
            dir,
            &[
                "commit-tree",
                tree.trim_end(),
                "-p",
                "HEAD",
                "-m",
                "skald snapshot",
            ],
        )
    })();
    // When the copy failed, the error of the copy wins over the one of its removal.
    let removed = fs::remove_file(&copy);
    let commit = result?;
    removed?;
    Ok(commit)
}

/// The reference a bundle carries its commit in; it exists only while the bundle is made.
pub const BUNDLE_REF: &str = "refs/skald/changes";

/// A bundle of `commit` and its history in the repository at `dir`, without `base` and its
/// history; without the merge base with the upstream's default branch when the repository lacks
/// `base`. `base` is an object ID.
pub fn bundle(dir: &Path, commit: &str, base: &str) -> Result<Vec<u8>> {
    if !is_object_id(base) {
        return Err(format!("not an object ID: {base:?}").into());
    }
    let base = if has_commit(dir, base)? {
        base.to_owned()
    } else {
        merge_base(dir)?
    };
    run(dir, &["update-ref", BUNDLE_REF, commit])?;
    // `-` writes the bundle to the standard output: it never reaches a file.
    let created = run(
        dir,
        &[
            "bundle",
            "create",
            "--quiet",
            "-",
            BUNDLE_REF,
            &format!("^{base}"),
        ],
    );
    run(dir, &["update-ref", "-d", BUNDLE_REF])?;
    created
}

/// Stores the objects of `bundle`, made by [`bundle`], in the repository at `dir` and returns its
/// commit. The repository must have the bundle's base.
pub fn unbundle(dir: &Path, bundle: &[u8]) -> Result<String> {
    // Created readable by its owner only; git only reads it.
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(bundle)?;
    let path = file
        .path()
        .to_str()
        .ok_or("a temporary path that is not UTF-8")?;
    let heads = String::from_utf8(run(dir, &["bundle", "unbundle", path])?)?;
    // `git bundle` refuses a bundle whose references do not start with an object ID.
    match heads.lines().collect::<Vec<_>>()[..] {
        [head] => match head.split_once(' ') {
            Some((commit, BUNDLE_REF)) => Ok(commit.to_owned()),
            _ => Err(format!("a bundle with another reference: {head}").into()),
        },
        _ => Err(format!("a bundle without exactly one reference: {heads}").into()),
    }
}

/// Whether `text` is a SHA-1 or SHA-256 object ID in lowercase hex.
pub fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The edit set of the working tree at `dir`, its root: the paths changed since the merge base of
/// `HEAD` and the upstream's default branch, committed or not, untracked ones included and ignored
/// ones not. A rename counts both paths: `--no-renames` lists it as a deletion and an addition. A
/// submodule counts when its commit changes, whatever the config and `.gitmodules` say, not when its
/// work tree does: the snapshot records its commit only.
pub fn edit_set(dir: &Path) -> Result<BTreeSet<String>> {
    let base = merge_base(dir)?;
    let mut paths = changed_paths(dir, &[&base])?;
    let untracked = run(dir, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    paths.extend(nul_separated(&untracked)?);
    Ok(paths)
}

/// The paths changed from the commit `base` to the commit `commit` in the repository at `dir`,
/// as [`edit_set`] lists those of a working tree.
pub fn changed(dir: &Path, base: &str, commit: &str) -> Result<BTreeSet<String>> {
    changed_paths(dir, &[base, commit])
}

/// The paths [`changed`] gives that are not in the commit `base`: those added since it.
pub fn added(dir: &Path, base: &str, commit: &str) -> Result<BTreeSet<String>> {
    changed_paths(dir, &["--diff-filter=A", base, commit])
}

/// The paths `git diff` finds changed from the first of `commits` to the second, or to the working
/// tree when there is no second, as [`edit_set`] says; `commits` can start with options.
fn changed_paths(dir: &Path, commits: &[&str]) -> Result<BTreeSet<String>> {
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-renames",
        "--ignore-submodules=dirty",
        "--name-only",
        "-z",
    ];
    args.extend(commits);
    args.push("--");
    nul_separated(&run(dir, &args)?)
}

/// The paths of `output`, each ended by a NUL.
fn nul_separated(output: &[u8]) -> Result<BTreeSet<String>> {
    output
        .split(|&byte| byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| Ok(String::from_utf8(path.to_vec())?))
        .collect()
}

/// The scope of the repository at `dir`: the least root commit of the upstream's default branch,
/// which every clone of the repository shares.
pub fn scope(dir: &Path) -> Result<Scope> {
    scope_of(dir, UPSTREAM_HEAD)
}

/// The scope of the history of `commit` in the repository at `dir`: its least root commit.
pub fn scope_of(dir: &Path, commit: &str) -> Result<Scope> {
    let roots = String::from_utf8(run(dir, &["rev-list", "--max-parents=0", commit])?)?;
    let root = roots
        .lines()
        .min()
        .ok_or_else(|| format!("{commit} has no root commit"))?;
    Ok(Scope::new(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_ids_are_40_or_64_lowercase_hex_digits() {
        let sha1 = "0123456789abcdef0123456789abcdef01234567";
        let sha256 = sha1.to_owned() + "89abcdef0123456789abcdef";
        assert!(is_object_id(sha1));
        assert!(is_object_id(&sha256));
        for text in [
            &sha1[1..],
            &(sha1.to_owned() + "8"),
            &sha1.replace('a', "A"),
            &sha1.replace('f', "g"),
            "--output=/tmp/x",
            "",
        ] {
            assert!(!is_object_id(text), "{text}");
        }
    }
}
