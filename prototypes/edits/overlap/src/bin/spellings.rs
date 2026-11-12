//! The check of how git and the fingerprints spell paths and line endings on this machine: runs it
//! in a temporary directory, deleted at the end, and prints its findings as Markdown.
//! Findings: `docs/dev/edit-prototype.md`, "Path normalization".

use std::process::ExitCode;

use edit_overlap::spellings;

fn main() -> ExitCode {
    let checked = tempfile::TempDir::new()
        .map_err(Into::into)
        .and_then(|tmp| spellings::check(tmp.path()));
    match checked {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("spellings: {error}");
            ExitCode::FAILURE
        }
    }
}
