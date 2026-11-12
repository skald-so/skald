//! `agent-socket <source-dir> [--no-auth] [--listen <seconds>]`: writes the waiting messages of the
//! source to the inbox socket of the Claude Code session its `SessionStart` hook registered, and
//! prints how many it handed over. With `--no-auth`, without the session's token. With `--listen`,
//! from a reply address it then keeps for that long, printing each receipt's status and `msg_id`.
//! Fails with exit code 1 and leaves them waiting if the write fails.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use agent_socket::{FROM, Receipts, hand_over, registered};
use agent_source::Source;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((dir, options)) = args.split_first() else {
        return usage();
    };
    let (auth, listen) = match options.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => (true, None),
        ["--no-auth"] => (false, None),
        ["--listen", seconds] => (true, Some(seconds)),
        ["--no-auth", "--listen", seconds] => (false, Some(seconds)),
        _ => return usage(),
    };
    let listen = match listen.map(str::parse::<u64>) {
        None => None,
        Some(Ok(seconds)) => Some(Duration::from_secs(seconds)),
        Some(Err(_)) => return usage(),
    };
    match run(Path::new(dir), auth, listen) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("agent-socket: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(dir: &Path, auth: bool, listen: Option<Duration>) -> std::io::Result<()> {
    let source = Source::open(dir)?;
    let Some(listen) = listen else {
        println!("{}", hand_over(&source, auth, FROM)?.len());
        return Ok(());
    };
    let socket = registered(&source)?.socket.unwrap_or_default();
    let receipts = Receipts::bind(Path::new(&socket))?;
    println!("{}", hand_over(&source, auth, &receipts.address())?.len());
    receipts.collect(&source, Instant::now() + listen, |receipt| {
        println!("{} {}", receipt.status, receipt.msg_id);
    })
}

fn usage() -> ExitCode {
    eprintln!("usage: agent-socket <source-dir> [--no-auth] [--listen <seconds>]");
    ExitCode::from(2)
}
