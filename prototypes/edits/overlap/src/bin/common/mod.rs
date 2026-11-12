//! What the commands share: a temporary directory, deleted at the end, the Markdown they print,
//! and their progress on the standard error.
#![allow(dead_code, reason = "each command uses some of it")]

use std::path::Path;
use std::process::ExitCode;

use edit_overlap::Result;
use edit_overlap::replay::Repository;

/// Runs the command `name`: `run` in a new temporary directory, deleted at the end, also on an
/// error, then prints the Markdown it returns; on an error, prints it after the name to the
/// standard error and fails.
pub fn main(name: &str, run: impl FnOnce(&Path) -> Result<String>) -> ExitCode {
    let ran = tempfile::TempDir::new()
        .map_err(Into::into)
        .and_then(|tmp| run(tmp.path()));
    match ran {
        Ok(out) => {
            print!("{out}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{name}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Tells that the command `name` fetches `repository`; returns what tells, after each merge it
/// replays, how many it has replayed.
pub fn replaying<'a>(name: &'a str, repository: &'a Repository) -> impl Fn(usize) + Sync + 'a {
    eprintln!(
        "{name}: fetching {} at {}",
        repository.url, repository.commit
    );
    move |done| {
        eprint!(
            "\r{name}: {} merge {done}/{}",
            repository.name, repository.merges
        )
    }
}
