//! The measurement on real repositories: fetches each repository at its fixed commit into a
//! temporary directory, replays its merges and prints the findings as Markdown, with the machine
//! and the git version. Progress goes to the standard error. The temporary directory is deleted
//! at the end. Findings: `docs/dev/edit-prototype.md`, "Measurement".

use std::path::Path;
use std::process::ExitCode;

use edit_overlap::replay::{self, Measured, Repository};
use edit_overlap::{Result, git};

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
    let mut measured = Vec::new();
    for repository in &REPOSITORIES {
        measured.push(measure(tmp.path(), repository)?);
    }
    println!("Machine: {}", machine());
    println!("Git: {}", git::line(tmp.path(), &["version"])?);
    println!();
    print!("{}", replay::report(&measured));
    Ok(())
}

/// Fetches `repository` at its commit into a bare repository in `tmp`, replays its merges on
/// every CPU, then times its largest sides one after another.
fn measure<'r>(tmp: &Path, repository: &'r Repository) -> Result<Measured<'r>> {
    let dir = tmp.join(format!("{}.git", repository.name));
    eprintln!(
        "measure: fetching {} at {}",
        repository.url, repository.commit
    );
    let path = dir.to_str().ok_or("a path that is not UTF-8")?;
    git::run(tmp, &["init", "--quiet", "--bare", path])?;
    git::run(
        &dir,
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            repository.url,
            repository.commit,
        ],
    )?;
    let scope = git::scope_of(&dir, repository.commit)?;
    let merges = replay::merges(&dir, repository.commit, repository.merges)?;
    if merges.len() != repository.merges {
        return Err(format!(
            "{} has {} merges of two parents, not {}",
            repository.name,
            merges.len(),
            repository.merges
        )
        .into());
    }
    let jobs = std::thread::available_parallelism()?.get();
    let progress = |done| {
        eprint!(
            "\rmeasure: {} merge {done}/{}",
            repository.name,
            merges.len()
        )
    };
    let replays = replay::replay_all(&dir, &scope, &merges, jobs, progress)?;
    eprintln!();
    let times = replay::cost(&dir, &scope, &replays)?;
    Ok(Measured {
        repository,
        replays,
        times,
    })
}

/// The operating system, the architecture, the CPU where the system names it, and the number of
/// CPUs.
fn machine() -> String {
    let cpu = std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .env_clear()
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    let cpus = std::thread::available_parallelism().map_or(0, |cpus| cpus.get());
    format!(
        "{} {}, {}{cpus} CPUs",
        std::env::consts::OS,
        std::env::consts::ARCH,
        cpu.map(|cpu| format!("{cpu}, ")).unwrap_or_default()
    )
}
