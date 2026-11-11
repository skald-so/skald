//! A machine's edit set and repository scope, read through the `git` CLI.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use crate::{Result, Scope};

/// The upstream's default branch, as `git clone` records it.
pub const UPSTREAM_HEAD: &str = "refs/remotes/origin/HEAD";

/// Runs `git` in `dir` and returns its standard output. It reads no global or system config and
/// no repository from the environment, never prompts, and commits as a fixed identity.
pub fn run(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "skald")
        .env("GIT_AUTHOR_EMAIL", "skald@example.com")
        .env("GIT_COMMITTER_NAME", "skald")
        .env("GIT_COMMITTER_EMAIL", "skald@example.com")
        .output()?;
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

/// The edit set of the working tree at `dir`, its root: the paths changed since the merge base of
/// `HEAD` and the upstream's default branch, committed or not, untracked ones included and ignored
/// ones not. A rename counts both paths: `--no-renames` lists it as a deletion and an addition.
pub fn edit_set(dir: &Path) -> Result<BTreeSet<String>> {
    let base = run(dir, &["merge-base", "HEAD", UPSTREAM_HEAD])?;
    let base = String::from_utf8(base)?;
    let changed = run(
        dir,
        &[
            "diff",
            "--no-ext-diff",
            "--no-renames",
            "--name-only",
            "-z",
            base.trim(),
            "--",
        ],
    )?;
    let untracked = run(dir, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut paths = BTreeSet::new();
    for output in [changed, untracked] {
        for path in output
            .split(|&byte| byte == 0)
            .filter(|path| !path.is_empty())
        {
            paths.insert(String::from_utf8(path.to_vec())?);
        }
    }
    Ok(paths)
}

/// The scope of the repository at `dir`: the least root commit of the upstream's default branch,
/// which every clone of the repository shares.
pub fn scope(dir: &Path) -> Result<Scope> {
    let roots = String::from_utf8(run(dir, &["rev-list", "--max-parents=0", UPSTREAM_HEAD])?)?;
    let root = roots
        .lines()
        .min()
        .ok_or("the upstream's default branch has no root commit")?;
    Ok(Scope::new(root))
}
