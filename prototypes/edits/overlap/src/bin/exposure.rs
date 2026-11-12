//! What fingerprints show: fetches each measured repository at its fixed commit, replays its
//! merges, and prints what the server learns from their fingerprints without the space key and what
//! a holder of the key recovers from them, as [`common::main`] runs a command. Findings:
//! `docs/dev/edit-prototype.md`, "What the server learns".

mod common;

use std::process::ExitCode;

use edit_overlap::exposure::{self, Exposed};
use edit_overlap::replay::{self, REPOSITORIES};

fn main() -> ExitCode {
    common::main("exposure", |tmp| {
        let jobs = std::thread::available_parallelism()?.get();
        let mut exposed = Vec::new();
        for repository in &REPOSITORIES {
            let progress = common::replaying("exposure", repository);
            let fetched = replay::fetch(tmp, repository)?;
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
        Ok(replay::header(tmp)? + &exposure::report(&exposed))
    })
}
