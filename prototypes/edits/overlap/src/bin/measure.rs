//! The measurement on real repositories: fetches each repository at its fixed commit into a
//! temporary directory, replays its merges and prints the findings as Markdown, with the machine
//! and the git version. Progress goes to the standard error. The temporary directory is deleted
//! at the end. Findings: `docs/dev/edit-prototype.md`, "Measurement".

use std::process::ExitCode;

use edit_overlap::Result;
use edit_overlap::replay::{self, Repository};

/// The measured repositories, each fetched over HTTPS without an account.
const REPOSITORIES: [Repository; 3] = [
    Repository {
        name: "diesel",
        url: "https://github.com/diesel-rs/diesel",
        commit: "31afd783aeddb2471bba2bece823816fe4925fed",
        merges: 300,
    },
    Repository {
        name: "drizzle-orm",
        url: "https://github.com/drizzle-team/drizzle-orm",
        commit: "bea7dd3623279ec2a12ca6c838cf5c8b17d3de42",
        merges: 300,
    },
    Repository {
        name: "netbox",
        url: "https://github.com/netbox-community/netbox",
        commit: "cf6d3f0319fade2fa0f1c51155f1c11330e58ff3",
        merges: 300,
    },
];

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
