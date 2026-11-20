//! `agent-channel <source-dir> [--permission-log <file>]`: a Claude Code channel over stdio that
//! pushes the source's messages; with a permission log, it also records each relayed permission
//! prompt there.

use std::process::ExitCode;

use agent_channel::ChannelServer;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> ExitCode {
    let (source, log) = match agent_source::from_args("agent-channel", ("--permission-log", "file"))
    {
        Ok(parsed) => parsed,
        Err(code) => return code,
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
