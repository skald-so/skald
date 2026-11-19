//! `agent-channel [--permission-log <file>] <source-dir>`: a Claude Code channel over stdio that
//! pushes the source's messages; with a permission log, it also records each relayed permission
//! prompt there.

use std::path::PathBuf;
use std::process::ExitCode;

use agent_channel::ChannelServer;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (log, dir) = match args.as_slice() {
        [dir] => (None, dir),
        [flag, log, dir] if flag == "--permission-log" => (Some(PathBuf::from(log)), dir),
        _ => {
            eprintln!("usage: agent-channel [--permission-log <file>] <source-dir>");
            return ExitCode::from(2);
        }
    };
    let source = match agent_source::Source::open(dir) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "agent-channel: cannot open the source {}: {error}",
                PathBuf::from(dir).display()
            );
            return ExitCode::FAILURE;
        }
    };
    let server = match log {
        Some(log) => ChannelServer::with_permission_log(source, log),
        None => ChannelServer::new(source),
    };
    let stopped = server.clone();
    let running = match server.serve(stdio()).await {
        Ok(running) => running,
        Err(error) => {
            eprintln!("agent-channel: {error}");
            return ExitCode::FAILURE;
        }
    };
    tokio::select! {
        ended = running.waiting() => match ended {
            Ok(_) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("agent-channel: {error}");
                ExitCode::FAILURE
            }
        },
        // Ending frees the claim, so the next take puts back the messages it still holds. The exit
        // does not wait for the blocked read of stdin, as the runtime's shutdown would.
        () = stopped.stopped() => std::process::exit(1),
    }
}
