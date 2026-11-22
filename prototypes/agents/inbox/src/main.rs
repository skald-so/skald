//! `agent-inbox <source-dir> [--claim <claim-dir>]`: serves the `inbox` tool over stdio for the
//! messages in the source; with a claim directory, also the stub `claim` tool.

use std::process::ExitCode;

use agent_inbox::InboxServer;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> ExitCode {
    let (source, claims) = match agent_source::from_args("agent-inbox", ("--claim", "claim-dir")) {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };
    let (server, transport) = InboxServer::new(
        source,
        rmcp::transport::IntoTransport::into_transport(stdio()),
    );
    let server = match claims {
        Some(dir) => server.with_claims(dir),
        None => server,
    };
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
