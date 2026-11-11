//! `agent-channel <source-dir>`: a Claude Code channel over stdio that pushes the source's messages.

use std::process::ExitCode;

use agent_channel::ChannelServer;
use agent_source::Source;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), None) = (args.next(), args.next()) else {
        eprintln!("usage: agent-channel <source-dir>");
        return ExitCode::from(2);
    };
    let source = match Source::open(&dir) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "agent-channel: cannot open the source {}: {error}",
                dir.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let server = ChannelServer::new(source);
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
