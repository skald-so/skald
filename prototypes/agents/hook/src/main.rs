//! `agent-hook [--codex] <source-dir>`: a command hook of Claude Code, or with `--codex` of Codex,
//! for `PostToolUse` and `Stop`, and on Codex `UserPromptSubmit`. Reads the hook input on stdin and
//! prints the waiting messages of the source as the hook's output.
//!
//! It fails with exit code 1, never 2: on `Stop`, exit code 2 would keep the turn going with stderr
//! as the agent's next instruction; on Codex's `PostToolUse`, it would replace the tool result.

use std::io::{self, Read};
use std::process::ExitCode;

use agent_hook::Agent;
use agent_source::Source;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (agent, dir) = match args.as_slice() {
        [flag, dir] if flag == "--codex" => (Agent::Codex, dir),
        [dir] if dir != "--codex" => (Agent::Claude, dir),
        _ => {
            eprintln!("usage: agent-hook [--codex] <source-dir>");
            return ExitCode::FAILURE;
        }
    };
    let source = match Source::open(dir) {
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
        .and_then(|_| agent_hook::handle(agent, &input, &source, &mut io::stdout().lock()));
    match handled {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("agent-hook: {error}");
            ExitCode::FAILURE
        }
    }
}
