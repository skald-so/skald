//! `agent-hook <source-dir>`: a Claude Code command hook for `PostToolUse` and `Stop`. Reads the hook
//! input on stdin and prints the waiting messages of the source as the hook's output.
//!
//! It fails with exit code 1, never 2: on `Stop`, exit code 2 would keep the turn going with stderr
//! as the agent's next instruction.

use std::io::{self, Read};
use std::process::ExitCode;

use agent_source::Source;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), None) = (args.next(), args.next()) else {
        eprintln!("usage: agent-hook <source-dir>");
        return ExitCode::FAILURE;
    };
    let source = match Source::open(&dir) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "agent-hook: cannot open the source {}: {error}",
                dir.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let mut input = Vec::new();
    let handled = io::stdin()
        .read_to_end(&mut input)
        .and_then(|_| agent_hook::handle(&input, &source, &mut io::stdout().lock()));
    match handled {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("agent-hook: {error}");
            ExitCode::FAILURE
        }
    }
}
