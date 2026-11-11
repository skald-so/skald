//! The scenario harness: a test repository in a bare upstream, and machines, each a local clone of
//! it that edits its working tree and commits, pushes and pulls through the `git` CLI.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{Result, git};

/// The upstream's default branch.
pub const DEFAULT_BRANCH: &str = "main";

/// A test repository: a bare upstream in a directory of its own.
pub struct Upstream {
    dir: PathBuf,
}

/// A local clone of an [`Upstream`].
pub struct Machine {
    dir: PathBuf,
}

impl Upstream {
    /// Creates the upstream in `dir`, a new directory, with one commit on its default branch that
    /// holds `files`, each a path and its contents.
    pub fn create(dir: &Path, files: &[(&str, &str)]) -> Result<Self> {
        fs::create_dir(dir)?;
        let upstream = Self {
            dir: dir.to_path_buf(),
        };
        git::run(
            dir,
            &[
                "init",
                "--quiet",
                "--bare",
                "--initial-branch",
                DEFAULT_BRANCH,
                "upstream.git",
            ],
        )?;
        let seed = Machine {
            dir: dir.join("seed"),
        };
        git::run(
            dir,
            &[
                "init",
                "--quiet",
                "--initial-branch",
                DEFAULT_BRANCH,
                "seed",
            ],
        )?;
        for (path, contents) in files {
            seed.write(path, contents)?;
        }
        seed.commit("initial commit")?;
        let url = upstream.url();
        git::run(&seed.dir, &["push", "--quiet", &url, DEFAULT_BRANCH])?;
        Ok(upstream)
    }

    fn url(&self) -> String {
        self.dir.join("upstream.git").display().to_string()
    }

    /// Clones the upstream as the machine `name`, in a directory of that name next to it.
    pub fn clone_as(&self, name: &str) -> Result<Machine> {
        git::run(&self.dir, &["clone", "--quiet", &self.url(), name])?;
        Ok(Machine {
            dir: self.dir.join(name),
        })
    }
}

impl Machine {
    /// The root of the machine's working tree.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Runs `git` in the working tree and returns its standard output.
    pub fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        git::run(&self.dir, args)
    }

    /// Writes `contents` to `path`, creating its directories, as an edit tool would.
    pub fn write(&self, path: &str, contents: &str) -> Result<()> {
        let path = self.dir.join(path);
        fs::create_dir_all(path.parent().ok_or("a path without a parent")?)?;
        fs::write(path, contents)?;
        Ok(())
    }

    /// Deletes `path` from the working tree.
    pub fn delete(&self, path: &str) -> Result<()> {
        fs::remove_file(self.dir.join(path))?;
        Ok(())
    }

    /// Moves `from` to `to` with `git mv`.
    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.git(&["mv", from, to])?;
        Ok(())
    }

    /// Commits every change of the working tree.
    pub fn commit(&self, message: &str) -> Result<()> {
        self.git(&["add", "--all"])?;
        self.git(&["commit", "--quiet", "--message", message])?;
        Ok(())
    }

    /// Pushes `HEAD` to the upstream's branch `branch`, as for a pull request.
    pub fn push(&self, branch: &str) -> Result<()> {
        self.git(&[
            "push",
            "--quiet",
            "origin",
            &format!("HEAD:refs/heads/{branch}"),
        ])?;
        Ok(())
    }

    /// Merges the upstream's branch `branch` into its default branch with a merge commit and
    /// pushes it, as a maintainer merging a pull request would.
    pub fn merge(&self, branch: &str) -> Result<()> {
        self.pull()?;
        self.git(&["fetch", "--quiet", "origin", branch])?;
        self.git(&[
            "merge",
            "--quiet",
            "--no-ff",
            "--message",
            branch,
            "FETCH_HEAD",
        ])?;
        self.git(&[
            "push",
            "--quiet",
            "origin",
            &format!("HEAD:{DEFAULT_BRANCH}"),
        ])?;
        Ok(())
    }

    /// Fetches the upstream and fast-forwards the current branch to its default branch.
    pub fn pull(&self) -> Result<()> {
        self.git(&["pull", "--quiet", "--ff-only", "origin", DEFAULT_BRANCH])?;
        Ok(())
    }
}
