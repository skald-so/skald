//! `agent-inbox <source-dir> [--claim <claim-dir>]`: serves the `inbox` tool over stdio for the
//! messages in the source; with a claim directory, also the stub `claim` tool.

use std::path::PathBuf;
use std::process::ExitCode;

use agent_inbox::InboxServer;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (dir, claims) = match args.as_slice() {
        [dir] => (dir, None),
        [dir, flag, claims] if flag == "--claim" => (dir, Some(PathBuf::from(claims))),
        _ => {
            eprintln!("usage: agent-inbox <source-dir> [--claim <claim-dir>]");
            return ExitCode::from(2);
        }
    };
    let source = match agent_source::Source::open(dir) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "agent-inbox: cannot open the source {}: {error}",
                std::path::Path::new(dir).display()
            );
            return ExitCode::FAILURE;
        }
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
