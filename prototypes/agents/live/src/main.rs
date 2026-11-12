//! Live checks of the delivery paths and of the subscription limits against the installed CLIs;
//! `usage` lists the commands.
//! `script/test` does not run them.

mod app;
mod gui_run;
mod pty;
mod read_run;
mod run;
mod subscription;
mod wake_run;

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use agent_live::gui::{GuiPath, Surface};
use agent_live::hook::{self, Hook};
use agent_live::wake::Wake;
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
        ["inbox", agent, options @ ..] => agent_check(agent, options, run::inbox),
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
        [path, "claude", options @ ..] if Wake::parse(path).is_some() => {
            let (Some(wake), Some(options)) = (Wake::parse(path), Options::parse(options)) else {
                return usage();
            };
            ended(wake_run::wake(wake, &options))
        }
        ["ack", agent, options @ ..] => agent_check(agent, options, read_run::ack),
        ["transcript", agent, options @ ..] => agent_check(agent, options, read_run::transcript),
        ["resend", "claude", options @ ..] => {
            let Some(options) = Options::parse(options).filter(|options| !options.real) else {
                return usage();
            };
            ended(read_run::resend(&options))
        }
        ["limits", agent, options @ ..] => {
            let (Some(agent), Some(options)) = (Agent::parse(agent), real_only(options)) else {
                return usage();
            };
            if options.model.is_some() {
                return usage();
            }
            ended(match agent {
                Agent::Claude => subscription::usage(&options),
                Agent::Codex => subscription::codex(&options),
            })
        }
        ["status-line", "claude", options @ ..] => {
            let Some(options) = real_only(options) else {
                return usage();
            };
            ended(subscription::status_line(&options))
        }
        ["gui", path, surface, flags @ ..] => {
            let (Some(path), Some(surface)) = (GuiPath::parse(path), Surface::parse(surface))
            else {
                return usage();
            };
            let (busy, bypass) = match flags {
                [] => (false, false),
                ["--busy"] if path.runs_busy_on(surface) => (true, false),
                ["--bypass"] if path == GuiPath::Wake(Wake::Socket) => (false, true),
                _ => return usage(),
            };
            if !path.runs_on(surface) {
                return usage();
            }
            ended(gui_run::prepare(path, surface, busy, bypass))
        }
        ["gui-socket", work, flags @ ..] => {
            let (auth, listen) = match flags {
                [] => (true, None),
                ["--no-auth"] => (false, None),
                ["--listen", seconds] => (true, Some(*seconds)),
                ["--no-auth", "--listen", seconds] => (false, Some(*seconds)),
                _ => return usage(),
            };
            let listen = match listen.map(str::parse::<u64>) {
                None => None,
                Some(Ok(seconds)) => Some(std::time::Duration::from_secs(seconds)),
                Some(Err(_)) => return usage(),
            };
            ended(gui_run::socket(Path::new(work), auth, listen))
        }
        ["gui-add", work] => ended(gui_run::add(Path::new(work))),
        ["gui-queue", work, cli @ ..] => {
            let cli = match cli {
                [] => false,
                ["--cli"] => true,
                _ => return usage(),
            };
            ended(gui_run::queue(Path::new(work), cli))
        }
        ["gui-result", work, token] => ended(gui_run::result(Path::new(work), token)),
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

/// The check `run` of the agent named `agent` with `options`; the usage for anything else.
fn agent_check(
    agent: &str,
    options: &[&str],
    run: impl FnOnce(Agent, &Options) -> Result<bool>,
) -> ExitCode {
    match (Agent::parse(agent), Options::parse(options)) {
        (Some(agent), Some(options)) => ended(run(agent, &options)),
        _ => usage(),
    }
}

/// The options of a check that runs only off the model stub: `[--model <model>] [--keep]`.
fn real_only<'a>(options: &[&'a str]) -> Option<Options<'a>> {
    Options::parse(options).filter(|options| !options.all && !options.real)
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
         live-check <socket|rewake|mod|loop> claude [--all] [--real] [--model <model>] [--keep]\n       \
         live-check <ack|transcript> <claude|codex> [--all] [--real] [--model <model>] [--keep]\n       \
         live-check resend claude [--all] [--model <model>] [--keep]\n       \
         live-check limits <claude|codex> [--keep]\n       \
         live-check status-line claude [--model <model>] [--keep]\n       \
         live-check gui <inbox|post-tool-use|stop> <claude-desktop|claude-vscode> [--busy]\n       \
         live-check gui channel <claude-desktop|claude-vscode>\n       \
         live-check gui inbox <codex-desktop|codex-vscode>\n       \
         live-check gui <post-tool-use|stop|queue> <codex-desktop|codex-vscode> [--busy]\n       \
         live-check gui user-prompt-submit <codex-desktop|codex-vscode>\n       \
         live-check gui <socket|rewake|mod|loop> <claude-desktop|claude-vscode> [--busy]\n       \
         live-check gui socket <claude-desktop|claude-vscode> --bypass\n       \
         live-check gui read <codex-desktop|codex-vscode> [--busy]\n       \
         live-check gui-socket <check-dir> [--no-auth] [--listen <seconds>]\n       \
         live-check gui-add <check-dir>\n       \
         live-check gui-queue <check-dir> [--cli]\n       \
         live-check gui-result <check-dir> <token>\n       \
         live-check timed-hook <log> <handler> [<arg>...]"
    );
    ExitCode::from(2)
}
