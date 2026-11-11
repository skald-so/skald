//! `agent-queue <source-dir> <thread>`: queues the waiting messages of the source for the running
//! Codex session `thread`, a session ID or name, with `codex queue`, and prints how many it handed
//! over. Fails with exit code 1 and leaves them waiting if the command fails.

use std::process::{Command, ExitCode};

use agent_source::Source;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), Some(thread), None) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: agent-queue <source-dir> <thread>");
        return ExitCode::FAILURE;
    };
    let handed = Source::open(&dir).and_then(|source| {
        agent_queue::hand_over(&source, &thread.to_string_lossy(), Command::new("codex"))
    });
    match handed {
        Ok(messages) => {
            println!("{}", messages.len());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("agent-queue: {error}");
            ExitCode::FAILURE
        }
    }
}
