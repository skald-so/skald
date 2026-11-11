//! Live checks of the delivery paths against the installed CLIs; `usage` lists the commands.
//! `script/test` does not run them.

mod pty;
mod run;

use std::process::ExitCode;

use agent_live::hook::Hook;
use agent_live::{Agent, Options, exit_code};
use agent_source::Source;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["add", dir, sender, text] => match Source::open(dir).and_then(|s| s.add(sender, text)) {
            Ok(message) => {
                println!("{}", message.msg_id);
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("live-check: {error}");
                ExitCode::FAILURE
            }
        },
        ["inbox", agent, options @ ..] => {
            let (Some(agent), Some(options)) = (Agent::parse(agent), Options::parse(options))
            else {
                return usage();
            };
            match run::inbox(agent, &options) {
                Ok(pass) => exit_code(pass),
                Err(error) => {
                    eprintln!("live-check: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        ["channel", "claude", options @ ..] => {
            let Some(options) = Options::parse(options) else {
                return usage();
            };
            match pty::channel(&options) {
                Ok(pass) => exit_code(pass),
                Err(error) => {
                    eprintln!("live-check: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [hook, "claude", options @ ..] if Hook::parse(hook).is_some() => {
            let (Some(hook), Some(options)) = (Hook::parse(hook), Options::parse(options)) else {
                return usage();
            };
            match run::hook(hook, &options) {
                Ok(pass) => exit_code(pass),
                Err(error) => {
                    eprintln!("live-check: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => usage(),
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
