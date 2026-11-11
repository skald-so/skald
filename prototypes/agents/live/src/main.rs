//! Live checks of the delivery paths against the installed CLIs; `usage` lists the commands.
//! `script/test` does not run them.

mod app;
mod pty;
mod run;

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use agent_live::hook::{self, Hook};
use agent_live::{Agent, Options, exit_code};
use agent_source::Source;

/// A command's result, or why it failed.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["add", dir, sender, text] => ended(
            Source::open(dir)
                .and_then(|s| s.add(sender, text))
                .map(|message| {
                    println!("{}", message.msg_id);
                    true
                })
                .map_err(Into::into),
        ),
        ["inbox", agent, options @ ..] => {
            let (Some(agent), Some(options)) = (Agent::parse(agent), Options::parse(options))
            else {
                return usage();
            };
            ended(run::inbox(agent, &options))
        }
        ["channel", "claude", options @ ..] => {
            let Some(options) = Options::parse(options) else {
                return usage();
            };
            ended(pty::channel(&options))
        }
        ["queue", "codex", options @ ..] => {
            let Some(options) = Options::parse(options) else {
                return usage();
            };
            ended(pty::queue(&options))
        }
        ["app-server", "codex", options @ ..] => {
            let Some(options) = Options::parse(options) else {
                return usage();
            };
            ended(app::app_server(&options))
        }
        ["timed-hook", log, handler, args @ ..] => {
            let mut command = std::process::Command::new(handler);
            command.args(args);
            let mut input = Vec::new();
            let timed = std::io::stdin().read_to_end(&mut input).and_then(|_| {
                hook::timed(
                    Path::new(log),
                    command,
                    &input,
                    &mut std::io::stdout().lock(),
                )
            });
            match timed {
                // A hook's exit code tells the CLI what to do: it passes on as it is.
                Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
                Err(error) => {
                    eprintln!("live-check: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [hook, agent, options @ ..] => {
            let (Some(hook), Some(agent), Some(options)) = (
                Hook::parse(hook),
                Agent::parse(agent),
                Options::parse(options),
            ) else {
                return usage();
            };
            if !hook.runs_on(agent) {
                return usage();
            }
            ended(run::hook(agent, hook, &options))
        }
        _ => usage(),
    }
}

/// The exit code of a command that ended with `result`: a check's pass or fail, or an error.
fn ended(result: Result<bool>) -> ExitCode {
    match result {
        Ok(pass) => exit_code(pass),
        Err(error) => {
            eprintln!("live-check: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: live-check add <source-dir> <sender> <text>\n       \
         live-check inbox <claude|codex> [--all] [--real] [--model <model>] [--keep]\n       \
         live-check channel claude [--all] [--real] [--model <model>] [--keep]\n       \
         live-check <post-tool-use|stop> <claude|codex> [--all] [--real] [--model <model>] [--keep]\n       \
         live-check user-prompt-submit codex [--all] [--real] [--model <model>] [--keep]\n       \
         live-check queue codex [--all] [--real] [--model <model>] [--keep]\n       \
         live-check app-server codex [--all] [--real] [--model <model>] [--keep]\n       \
         live-check timed-hook <log> <handler> [<arg>...]"
    );
    ExitCode::from(2)
}
