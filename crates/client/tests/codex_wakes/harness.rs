//! The harness of the test targets whose binary also stands in for the `codex` command, whose
//! arguments libtest would refuse.

use std::panic;
use std::process::ExitCode;

/// Runs `tests` as libtest does for cargo and cargo-nextest: with `--list`, prints `<name>: test`
/// for each chosen test and runs none; otherwise runs each chosen test, one after another, and fails
/// with libtest's exit code if one panics. A test is chosen unless `--ignored` is given, as none is
/// ignored, or a filter is given that its name does not hold, or, with `--exact`, is not.
pub fn harness(tests: &[(&str, fn())]) -> ExitCode {
    let mut filters: Vec<String> = Vec::new();
    let mut skips: Vec<String> = Vec::new();
    let (mut list, mut exact, mut ignored) = (false, false, false);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => list = true,
            "--exact" => exact = true,
            "--ignored" => ignored = true,
            "--skip" => skips.extend(args.next()),
            // libtest's options that take a value.
            "--format" | "--color" | "--test-threads" | "--logfile" | "--shuffle-seed" | "-Z" => {
                args.next();
            }
            flag if flag.starts_with('-') => {}
            _ => filters.push(arg),
        }
    }
    let matches = |name: &str, filter: &String| {
        if exact {
            name == filter
        } else {
            name.contains(filter.as_str())
        }
    };
    let chosen: Vec<&(&str, fn())> = (tests.iter())
        .filter(|_| !ignored)
        .filter(|(name, _)| filters.is_empty() || filters.iter().any(|f| matches(name, f)))
        .filter(|(name, _)| !skips.iter().any(|skip| matches(name, skip)))
        .collect();
    if list {
        for (name, _) in &chosen {
            println!("{name}: test");
        }
        return ExitCode::SUCCESS;
    }
    let mut failed = Vec::new();
    for (name, test) in &chosen {
        println!("test {name} ...");
        match panic::catch_unwind(*test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(*name);
            }
        }
    }
    println!(
        "test result: {}. {} passed; {} failed",
        if failed.is_empty() { "ok" } else { "FAILED" },
        chosen.len() - failed.len(),
        failed.len()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        // libtest's exit code when a test fails.
        ExitCode::from(101)
    }
}
