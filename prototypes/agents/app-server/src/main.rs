//! `agent-app-server <source-dir> [--socket <path>] [--thread <id>] [--prompt <text>] [--log <file>]
//! [-- <app-server option>...]`: hands the waiting messages of the source to a Codex app-server
//! thread as they come, and prints what happens as JSON lines ([`agent_app_server::Client`]).
//!
//! - It runs `codex app-server <app-server option>...` on stdio, or with `--socket` connects to the
//!   app-server that listens on that Unix socket.
//! - It starts a thread in the current directory, or with `--thread` resumes that one.
//! - `--prompt` starts the thread's first turn with that prompt.
//! - `--log` writes each JSON-RPC message to the file.
//!
//! It stops once its stdin ends, and fails with exit code 1 when the app-server refuses to start or
//! resume the thread or closes the connection.

use std::io;
use std::path::Path;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use agent_app_server::{Client, Socket, Stdio, Transport, logged};
use agent_source::Source;

const USAGE: &str = "usage: agent-app-server <source-dir> [--socket <path>] [--thread <id>] \
                     [--prompt <text>] [--log <file>] [-- <app-server option>...]";

#[derive(Default)]
struct Args<'a> {
    source: &'a str,
    socket: Option<&'a str>,
    thread: Option<&'a str>,
    prompt: Option<&'a str>,
    log: Option<&'a str>,
    options: &'a [String],
}

impl<'a> Args<'a> {
    fn parse(args: &'a [String]) -> Option<Self> {
        let (source, mut rest) = args.split_first()?;
        if source.starts_with("--") {
            return None;
        }
        let mut parsed = Self {
            source,
            ..Self::default()
        };
        loop {
            match rest {
                [] => return Some(parsed),
                [flag, options @ ..] if flag == "--" && parsed.socket.is_none() => {
                    parsed.options = options;
                    return Some(parsed);
                }
                [flag, value, tail @ ..] => {
                    let slot = match flag.as_str() {
                        "--socket" => &mut parsed.socket,
                        "--thread" => &mut parsed.thread,
                        "--prompt" => &mut parsed.prompt,
                        "--log" => &mut parsed.log,
                        _ => return None,
                    };
                    if slot.replace(value).is_some() {
                        return None;
                    }
                    rest = tail;
                }
                _ => return None,
            }
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(args) = Args::parse(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let stop = Arc::new(AtomicBool::new(false));
    let stopper = Arc::clone(&stop);
    thread::spawn(move || {
        let _ = io::copy(&mut io::stdin(), &mut io::sink());
        stopper.store(true, Ordering::SeqCst);
    });
    match run(&args, stop) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("agent-app-server: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args, stop: Arc<AtomicBool>) -> io::Result<()> {
    let source = Source::open(args.source)?;
    let transport: Box<dyn Transport> = match args.socket {
        Some(path) => Box::new(Socket::connect(Path::new(path))?),
        None => {
            let mut codex = Command::new("codex");
            codex.arg("app-server").args(args.options);
            Box::new(Stdio::spawn(codex)?)
        }
    };
    let transport = logged(transport, args.log)?;
    let mut client = Client::new(transport, io::stdout().lock(), stop);
    client.initialize()?;
    match args.thread {
        Some(thread) => client.resume_thread(thread)?,
        None => client.start_thread(&std::env::current_dir()?)?,
    };
    if let Some(prompt) = args.prompt {
        client.start_turn(prompt)?;
    }
    client.run(&source)
}
