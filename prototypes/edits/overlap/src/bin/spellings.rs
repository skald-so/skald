//! The check of how git and the fingerprints spell paths and line endings on this machine: runs it
//! and prints its findings, as [`common::main`] runs a command. Findings:
//! `docs/dev/edit-prototype.md`, "Path normalization".

mod common;

use std::process::ExitCode;

use edit_overlap::spellings;

fn main() -> ExitCode {
    common::main("spellings", spellings::check)
}
