//! Live checks of the delivery paths against the installed CLIs; `usage` lists the commands.
//! `script/test` does not run them.

mod pty;
mod run;

use std::process::ExitCode;

use agent_live::hook::Hook;
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
        // The commands above take their own `claude`.
        [hook, "claude", options @ ..] => {
            let (Some(hook), Some(options)) = (Hook::parse(hook), Options::parse(options)) else {
                return usage();
            };
            ended(run::hook(hook, &options))
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
         live-check <post-tool-use|stop> claude [--all] [--real] [--model <model>] [--keep]"
    );
    ExitCode::from(2)
}
