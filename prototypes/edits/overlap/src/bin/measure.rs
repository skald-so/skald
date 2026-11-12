//! The measurement on real repositories: fetches each repository at its fixed commit into a
//! temporary directory, replays its merges and prints the findings as Markdown, with the machine
//! and the git version. Progress goes to the standard error. The temporary directory is deleted
//! at the end. Findings: `docs/dev/edit-prototype.md`, "Measurement".

use std::process::ExitCode;

use edit_overlap::Result;
use edit_overlap::replay::{self, REPOSITORIES};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("measure: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let jobs = std::thread::available_parallelism()?.get();
    let mut measured = Vec::new();
    for repository in &REPOSITORIES {
        eprintln!(
            "measure: fetching {} at {}",
            repository.url, repository.commit
        );
        let progress = |done| {
            eprint!(
                "\rmeasure: {} merge {done}/{}",
                repository.name, repository.merges
            )
        };
        measured.push(replay::measure(tmp.path(), repository, jobs, progress)?);
        eprintln!();
    }
    print!("{}", replay::header(tmp.path())?);
    print!("{}", replay::report(&measured));
    Ok(())
}
