//! What fingerprints show: fetches each measured repository at its fixed commit into a temporary
//! directory, replays its merges, and prints as Markdown, with the machine and the git version,
//! what the server learns from their fingerprints without the space key and what a holder of the
//! key recovers from them. Progress goes to the standard error. The temporary directory is deleted
//! at the end. Findings: `docs/dev/edit-prototype.md`, "What the server learns".

use std::process::ExitCode;

use edit_overlap::Result;
use edit_overlap::exposure::{self, Exposed};
use edit_overlap::replay::{self, REPOSITORIES};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("exposure: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let jobs = std::thread::available_parallelism()?.get();
    let mut exposed = Vec::new();
    for repository in &REPOSITORIES {
        eprintln!(
            "exposure: fetching {} at {}",
            repository.url, repository.commit
        );
        let fetched = replay::fetch(tmp.path(), repository)?;
        let progress = |done| {
            eprint!(
                "\rexposure: {} merge {done}/{}",
                repository.name, repository.merges
            )
        };
        let replays = replay::replay_all(
            &fetched.dir,
            &fetched.scope,
            &fetched.merges,
            jobs,
            progress,
        )?;
        eprintln!();
        eprintln!(
            "exposure: recovering the fingerprints of {}",
            repository.name
        );
        let recovered = exposure::recover(&fetched.dir, &fetched.scope, &replays)?;
        exposed.push(Exposed {
            repository,
            replays,
            recovered,
        });
    }
    print!("{}", replay::header(tmp.path())?);
    print!("{}", exposure::report(&exposed));
    Ok(())
}
