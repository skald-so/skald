//! `agent-inbox <source-dir>`: serves the `inbox` tool over stdio for the messages in the source.

use std::process::ExitCode;

use agent_inbox::InboxServer;
use agent_source::Source;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), None) = (args.next(), args.next()) else {
        eprintln!("usage: agent-inbox <source-dir>");
        return ExitCode::from(2);
    };
    let source = match Source::open(&dir) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "agent-inbox: cannot open the source {}: {error}",
                dir.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let (server, transport) = InboxServer::new(
        source,
        rmcp::transport::IntoTransport::into_transport(stdio()),
    );
    match server.serve(transport).await {
        Ok(running) => match running.waiting().await {
            Ok(_) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("agent-inbox: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("agent-inbox: {error}");
            ExitCode::FAILURE
        }
    }
}
