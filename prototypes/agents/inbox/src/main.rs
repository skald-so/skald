//! `agent-inbox <source-dir>`: serves the `inbox` tool over stdio for the messages in the source.

use std::process::ExitCode;

use agent_inbox::InboxServer;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> ExitCode {
    let source = match agent_source::from_args("agent-inbox") {
        Ok(source) => source,
        Err(code) => return code,
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
