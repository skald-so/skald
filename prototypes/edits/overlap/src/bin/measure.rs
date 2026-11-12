//! The measurement on real repositories: fetches each repository at its fixed commit, replays its
//! merges and prints the findings, as [`common::main`] runs a command. Findings:
//! `docs/dev/edit-prototype.md`, "Measurement".

mod common;

use std::process::ExitCode;

use edit_overlap::replay::{self, REPOSITORIES};

fn main() -> ExitCode {
    common::main("measure", |tmp| {
        let jobs = std::thread::available_parallelism()?.get();
        let mut measured = Vec::new();
        for repository in &REPOSITORIES {
            let progress = common::replaying("measure", repository);
            measured.push(replay::measure(tmp, repository, jobs, progress)?);
            eprintln!();
        }
        Ok(replay::header(tmp)? + &replay::report(&measured))
    })
}
