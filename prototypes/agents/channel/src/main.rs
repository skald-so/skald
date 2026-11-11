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
    match ChannelServer::new(source).serve(stdio()).await {
        Ok(running) => match running.waiting().await {
            Ok(_) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("agent-channel: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("agent-channel: {error}");
            ExitCode::FAILURE
        }
    }
}
