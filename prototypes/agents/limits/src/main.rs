//! Prints the subscription limits a CLI shows the user, one JSON line each:
//! `{"limit", "usedPercent", "resetsAt"}`, `resetsAt` in seconds since the Unix epoch.
//!
//! - `agent-limits claude [--log <file>]`: from Claude Code's `/usage`, `claude` with
//!   [`agent_limits::CLAUDE_USAGE`] in the current directory; `--log` writes the command's output to
//!   the file.
//! - `agent-limits codex [--log <file>] [-- <app-server option>...]`: from the Codex app-server,
//!   `codex app-server <app-server option>...` on stdio. First `{"account"}`, the account's kind;
//!   then the limits, or `{"refused"}` with the app-server's reason. `--log` writes each JSON-RPC
//!   message to the file.
//! - `agent-limits status-line [--log <file>]`: Claude Code's status line command. It prints the
//!   limits of its input on stdin as the status line; `--log` appends each input to the file, one
//!   line each.
//!
//! It fails with exit code 1 when the CLI fails, and 2 on wrong arguments.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use agent_app_server::{Client, Stdio, logged};
use agent_limits::{Limit, claude_status_line, claude_usage, read_codex, run_claude, status_line};
use serde_json::json;

const USAGE: &str = "usage: agent-limits claude [--log <file>]\n       \
                     agent-limits codex [--log <file>] [-- <app-server option>...]\n       \
                     agent-limits status-line [--log <file>]";

/// What to read, and the file `--log` names, if any.
enum Source<'a> {
    Claude(Option<&'a str>),
    Codex(Option<&'a str>, &'a [String]),
    StatusLine(Option<&'a str>),
}

impl<'a> Source<'a> {
    fn parse(args: &'a [String]) -> Option<Self> {
        let (source, rest) = args.split_first()?;
        let (log, rest) = match rest {
            [flag, log, rest @ ..] if flag == "--log" => (Some(log.as_str()), rest),
            rest => (None, rest),
        };
        match (source.as_str(), rest) {
            ("claude", []) => Some(Self::Claude(log)),
            ("status-line", []) => Some(Self::StatusLine(log)),
            ("codex", []) => Some(Self::Codex(log, &[])),
            ("codex", [dashes, options @ ..]) if dashes == "--" => Some(Self::Codex(log, options)),
            _ => None,
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(source) = Source::parse(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let ran = match source {
        Source::Claude(log) => claude(log),
        Source::Codex(log, options) => codex(log, options),
        Source::StatusLine(log) => status(log),
    };
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("agent-limits: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print(limits: &[Limit]) {
    for limit in limits {
        println!("{}", limit.to_json());
    }
}

fn claude(log: Option<&str>) -> io::Result<()> {
    let output = run_claude(Command::new("claude"))?;
    if let Some(log) = log {
        std::fs::write(log, &output)?;
    }
    print(&claude_usage(&output));
    Ok(())
}

fn codex(log: Option<&str>, options: &[String]) -> io::Result<()> {
    let mut app_server = Command::new("codex");
    app_server.arg("app-server").args(options);
    let transport = logged(Box::new(Stdio::spawn(app_server)?), log)?;
    let mut client = Client::new(transport, io::sink(), Arc::new(AtomicBool::new(false)));
    client.initialize()?;
    let read = read_codex(&mut client)?;
    println!("{}", json!({ "account": read.account }));
    match read.limits {
        Ok(limits) => print(&limits),
        Err(reason) => println!("{}", json!({ "refused": reason })),
    }
    Ok(())
}

fn status(log: Option<&str>) -> io::Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    if let Some(log) = log {
        let mut file: File = OpenOptions::new().create(true).append(true).open(log)?;
        writeln!(file, "{}", input.trim_end())?;
    }
    println!("{}", status_line(&claude_status_line(&input)));
    Ok(())
}
